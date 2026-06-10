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
#[path = "command_tests.rs"]
mod tests;
