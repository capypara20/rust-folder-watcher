use crate::config::ActionConfig;
use crate::error::AppError;
use crate::placeholder::{expand_placeholders, PlaceholderContext};

use super::spawn::spawn_detached;
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

    let (program, args) = build_shell_command(shell, &expanded)?;

    spawn_detached(&program, &args, working_dir).map_err(|e| {
        AppError::Action(format!(
            "command: プロセス起動失敗 (shell={shell} cmd={expanded}): {e}"
        ))
    })?;

    sink.ok(step.0, step.1, "起動".to_string());
    Ok(())
}

/// シェル種別から、起動するプログラムとその引数を組み立てる。
fn build_shell_command(shell: &str, expanded: &str) -> Result<(String, Vec<String>), AppError> {
    match shell {
        "cmd" => Ok((
            "cmd.exe".to_string(),
            vec!["/C".to_string(), expanded.to_string()],
        )),
        "powershell" => Ok((
            "powershell.exe".to_string(),
            vec![
                "-NoProfile".to_string(),
                "-Command".to_string(),
                expanded.to_string(),
            ],
        )),
        "pwsh" => {
            #[cfg(windows)]
            let bin = "pwsh.exe";
            #[cfg(not(windows))]
            let bin = "pwsh";
            Ok((
                bin.to_string(),
                vec![
                    "-NoProfile".to_string(),
                    "-Command".to_string(),
                    expanded.to_string(),
                ],
            ))
        }
        #[cfg(not(windows))]
        "bash" => Ok((
            "bash".to_string(),
            vec!["-c".to_string(), expanded.to_string()],
        )),
        #[cfg(not(windows))]
        "sh" => Ok((
            "sh".to_string(),
            vec!["-c".to_string(), expanded.to_string()],
        )),
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
#[path = "../tests/actions_command.rs"]
mod tests;
