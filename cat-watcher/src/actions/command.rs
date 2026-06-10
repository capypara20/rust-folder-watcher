use std::process::Stdio;

use crate::config::ActionConfig;
use crate::error::AppError;
use crate::placeholder::{expand_placeholders, PlaceholderContext};

use super::ActionSink;

pub async fn execute(
    action: &ActionConfig,
    ctx: &PlaceholderContext,
    sink: &ActionSink,
    step: (usize, usize),
) -> Result<(), AppError> {
    let raw_command = action
        .command
        .as_deref()
        .ok_or_else(|| AppError::Action("command: command が未指定".to_string()))?;
    let expanded = expand_placeholders(raw_command, ctx)?;

    let shell = action
        .shell
        .as_deref()
        .ok_or_else(|| AppError::Action("command: shell が未指定".to_string()))?;

    let working_dir = action
        .working_dir
        .as_deref()
        .filter(|s| !s.is_empty());

    let mut cmd = build_shell_command(shell, &expanded)?;
    cmd.stdout(Stdio::null()).stderr(Stdio::null());

    if let Some(dir) = working_dir {
        cmd.current_dir(dir);
    }

    cmd.spawn().map_err(|e| {
        AppError::Action(format!(
            "command: プロセス起動失敗 (shell={shell} cmd={expanded}): {e}"
        ))
    })?;

    sink.ok(step.0, step.1, "起動".to_string());
    Ok(())
}

fn build_shell_command(
    shell: &str,
    expanded: &str,
) -> Result<tokio::process::Command, AppError> {
    match shell {
        "cmd" => {
            let mut c = tokio::process::Command::new("cmd.exe");
            c.args(["/C", expanded]);
            Ok(c)
        }
        "powershell" => {
            let mut c = tokio::process::Command::new("powershell.exe");
            c.args(["-NoProfile", "-Command", expanded]);
            Ok(c)
        }
        "pwsh" => {
            #[cfg(windows)]
            let bin = "pwsh.exe";
            #[cfg(not(windows))]
            let bin = "pwsh";
            let mut c = tokio::process::Command::new(bin);
            c.args(["-NoProfile", "-Command", expanded]);
            Ok(c)
        }
        #[cfg(not(windows))]
        "bash" => {
            let mut c = tokio::process::Command::new("bash");
            c.args(["-c", expanded]);
            Ok(c)
        }
        #[cfg(not(windows))]
        "sh" => {
            let mut c = tokio::process::Command::new("sh");
            c.args(["-c", expanded]);
            Ok(c)
        }
        other => Err(AppError::Action(format!(
            "command: 不明なシェル '{other}'。{} のいずれかを指定してください",
            if cfg!(windows) {
                "cmd / powershell / pwsh"
            } else {
                "bash / sh / pwsh"
            }
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ActionType, LogRotation};
    use crate::logger::Logger;
    use std::sync::Arc;
    use tempfile::tempdir;

    fn make_action(shell: &str, command: &str, working_dir: &str) -> ActionConfig {
        ActionConfig {
            type_: ActionType::Command,
            destination: None,
            overwrite: None,
            preserve_structure: None,
            verify_integrity: None,
            shell: Some(shell.to_string()),
            command: Some(command.to_string()),
            working_dir: Some(working_dir.to_string()),
            program: None,
            args: None,
            message: None,
        }
    }

    fn make_ctx(src: &std::path::Path, watch: &std::path::Path) -> PlaceholderContext {
        PlaceholderContext::new(src, watch, "")
    }

    fn make_sink() -> ActionSink {
        let dir = tempdir().unwrap();
        let (logger, _) = Logger::for_action(
            dir.path().to_str().unwrap().to_string(),
            "test.log".to_string(),
            LogRotation::Never,
        )
        .unwrap();
        std::mem::forget(dir);
        ActionSink::new(Arc::new(logger), None)
    }

    #[tokio::test]
    async fn unknown_shell_returns_error() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        std::fs::write(&src, b"x").unwrap();
        let ctx = make_ctx(&src, dir.path());
        let action = make_action("zsh", "echo hi", "");
        let result = execute(&action, &ctx, &make_sink(), (1, 1)).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("不明なシェル"));
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn bash_spawns_successfully() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        std::fs::write(&src, b"x").unwrap();
        let ctx = make_ctx(&src, dir.path());
        let action = make_action("bash", "echo hello", "");
        assert!(execute(&action, &ctx, &make_sink(), (1, 1)).await.is_ok());
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn sh_spawns_successfully() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        std::fs::write(&src, b"x").unwrap();
        let ctx = make_ctx(&src, dir.path());
        let action = make_action("sh", "echo hello", "");
        assert!(execute(&action, &ctx, &make_sink(), (1, 1)).await.is_ok());
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn cmd_spawns_successfully() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        std::fs::write(&src, b"x").unwrap();
        let ctx = make_ctx(&src, dir.path());
        let action = make_action("cmd", "echo hello", "");
        assert!(execute(&action, &ctx, &make_sink(), (1, 1)).await.is_ok());
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn placeholder_expands_in_command() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("report.txt");
        std::fs::write(&src, b"x").unwrap();
        let ctx = make_ctx(&src, dir.path());
        let action = make_action("cmd", "echo {Name}", "");
        assert!(execute(&action, &ctx, &make_sink(), (1, 1)).await.is_ok());
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn working_dir_is_set() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        std::fs::write(&src, b"x").unwrap();
        let ctx = make_ctx(&src, dir.path());
        let action = make_action("cmd", "echo hi", dir.path().to_str().unwrap());
        assert!(execute(&action, &ctx, &make_sink(), (1, 1)).await.is_ok());
    }

    #[test]
    fn build_shell_command_cmd() {
        assert!(build_shell_command("cmd", "echo test").is_ok());
    }

    #[test]
    fn build_shell_command_powershell() {
        assert!(build_shell_command("powershell", "Get-Date").is_ok());
    }

    #[test]
    fn build_shell_command_pwsh() {
        assert!(build_shell_command("pwsh", "Get-Date").is_ok());
    }

    #[test]
    fn build_shell_command_unknown() {
        assert!(build_shell_command("zsh", "echo hi").is_err());
    }
}
