use std::sync::Arc;

use std::process::Stdio;

use crate::config::{ActionConfig, Global};
use crate::error::AppError;
use crate::logger::Logger;
use crate::placeholder::{expand_placeholders, PlaceholderContext};

pub async fn execute(
    action: &ActionConfig,
    ctx: &PlaceholderContext,
    _global: &Global,
    log: Arc<Logger>,
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

    log.log_action_ok(step.0, step.1, "起動");
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
    use crate::config::{ActionType, Global, LogLevel, LogRotation};
    use tempfile::tempdir;

    fn make_global() -> Global {
        let dir = tempdir().unwrap();
        Global {
            log_level: LogLevel::Info,
            log_dir: dir.path().to_str().unwrap().to_string(),
            log_file_name: "test.log".to_string(),
            log_rotation: LogRotation::Never,
            retry_count: 0,
            retry_interval_ms: 0,
            log_to_console: false,
            log_to_file: false,
            terminal_log_level: None,
            file_log_level: None,
        }
    }

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

    fn make_logger() -> Arc<Logger> {
        let dir = tempdir().unwrap();
        let global = Global {
            log_level: LogLevel::Info,
            log_dir: dir.path().to_str().unwrap().to_string(),
            log_file_name: "test.log".to_string(),
            log_rotation: LogRotation::Never,
            retry_count: 0,
            retry_interval_ms: 0,
            log_to_console: false,
            log_to_file: false,
            terminal_log_level: None,
            file_log_level: None,
        };
        std::mem::forget(dir);
        let (logger, _) = Logger::new(&global).unwrap();
        Arc::new(logger)
    }

    #[tokio::test]
    async fn unknown_shell_returns_error() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        std::fs::write(&src, b"x").unwrap();
        let ctx = make_ctx(&src, dir.path());
        let action = make_action("zsh", "echo hi", "");
        let global = make_global();
        let result = execute(&action, &ctx, &global, make_logger(), (1, 1)).await;
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
        let global = make_global();
        assert!(execute(&action, &ctx, &global, make_logger(), (1, 1)).await.is_ok());
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn sh_spawns_successfully() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        std::fs::write(&src, b"x").unwrap();
        let ctx = make_ctx(&src, dir.path());
        let action = make_action("sh", "echo hello", "");
        let global = make_global();
        assert!(execute(&action, &ctx, &global, make_logger(), (1, 1)).await.is_ok());
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn cmd_spawns_successfully() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        std::fs::write(&src, b"x").unwrap();
        let ctx = make_ctx(&src, dir.path());
        let action = make_action("cmd", "echo hello", "");
        let global = make_global();
        assert!(execute(&action, &ctx, &global, make_logger(), (1, 1)).await.is_ok());
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn placeholder_expands_in_command() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("report.txt");
        std::fs::write(&src, b"x").unwrap();
        let ctx = make_ctx(&src, dir.path());
        let action = make_action("cmd", "echo {Name}", "");
        let global = make_global();
        assert!(execute(&action, &ctx, &global, make_logger(), (1, 1)).await.is_ok());
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn working_dir_is_set() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        std::fs::write(&src, b"x").unwrap();
        let ctx = make_ctx(&src, dir.path());
        let action = make_action("cmd", "echo hi", dir.path().to_str().unwrap());
        let global = make_global();
        assert!(execute(&action, &ctx, &global, make_logger(), (1, 1)).await.is_ok());
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
