use crate::config::ActionConfig;
use crate::error::AppError;
use crate::placeholder::{expand_placeholders, PlaceholderContext};

use super::spawn::{spawn_detached, SpawnArg};
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

/// `shell` に指定できる値。この OS で実際に起動できるものだけを並べる。
/// 設定バリデーション（config/validate.rs）とここで同じ一覧を使う。
#[cfg(windows)]
pub const VALID_SHELLS: &[&str] = &["cmd", "powershell", "pwsh"];
#[cfg(not(windows))]
pub const VALID_SHELLS: &[&str] = &["bash", "sh", "pwsh"];

/// シェル種別から、起動するプログラムとその引数を組み立てる。
/// 他の設定値と同じく、大文字小文字は区別しない（`cmd` / `CMD` どちらも可）。
fn build_shell_command(shell: &str, expanded: &str) -> Result<(String, Vec<SpawnArg>), AppError> {
    match shell.to_lowercase().as_str() {
        #[cfg(windows)]
        "cmd" => Ok((
            "cmd.exe".to_string(),
            // cmd.exe は argv 規則ではなく独自の規則でコマンドラインを解釈する。
            // 先頭がダブルクオートのときは最初と最後のダブルクオートを取り除いて
            // 残りをコマンドとして扱うため、コマンド全体を囲んでそのまま渡す。
            // argv 規則でクオートするとエスケープ用のバックスラッシュが文字として
            // 残り、cmd.exe が解釈できずコマンドが壊れる。
            vec![
                SpawnArg::Quoted("/C".to_string()),
                SpawnArg::Raw(wrap_for_cmd(expanded)),
            ],
        )),
        #[cfg(windows)]
        "powershell" => Ok((
            "powershell.exe".to_string(),
            quoted_args(&["-NoProfile", "-Command", expanded]),
        )),
        "pwsh" => {
            #[cfg(windows)]
            let bin = "pwsh.exe";
            #[cfg(not(windows))]
            let bin = "pwsh";
            Ok((
                bin.to_string(),
                quoted_args(&["-NoProfile", "-Command", expanded]),
            ))
        }
        #[cfg(not(windows))]
        "bash" => Ok(("bash".to_string(), quoted_args(&["-c", expanded]))),
        #[cfg(not(windows))]
        "sh" => Ok(("sh".to_string(), quoted_args(&["-c", expanded]))),
        other => Err(AppError::Action(format!(
            "command: 不明なシェル '{other}'。{} のいずれかを指定してください",
            VALID_SHELLS.join(" / ")
        ))),
    }
}

/// すべて argv 規則でクオートする引数列を作る。
fn quoted_args(args: &[&str]) -> Vec<SpawnArg> {
    args.iter().map(|a| SpawnArg::Quoted(a.to_string())).collect()
}

/// `cmd /C` へ渡すコマンドを、cmd.exe の規則に合わせてダブルクオートで囲む。
#[cfg(windows)]
fn wrap_for_cmd(expanded: &str) -> String {
    let q = '"';
    format!("{q}{expanded}{q}")
}

#[cfg(test)]
#[path = "../tests/actions_command.rs"]
mod tests;
