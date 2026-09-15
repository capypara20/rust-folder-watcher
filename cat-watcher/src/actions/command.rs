use crate::config::ActionConfig;
use crate::error::AppError;
use crate::placeholder::{expand_placeholders, PlaceholderContext};

use super::spawn::{outcome_message, spawn_process, wait_mode_from, SpawnArg};
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
    let wait = wait_mode_from(action.wait, action.timeout_ms);

    let outcome = spawn_process(&program, &args, working_dir, wait)
        .await
        .map_err(|e| {
            AppError::Action(format!(
                "command: プロセス起動失敗 (shell={shell} cmd={expanded}): {e}"
            ))
        })?;

    match outcome_message(outcome) {
        Ok(msg) => {
            sink.ok(step.0, step.1, msg);
            Ok(())
        }
        // wait = true のときだけ起こる。アクション失敗として扱うので、
        // 以降のアクションチェーンは中断される。
        Err(reason) => Err(AppError::Action(format!(
            "command: {reason} (shell={shell} cmd={expanded})"
        ))),
    }
}

/// `shell` に指定できる値。この OS で実際に起動できるものだけを並べる。
/// 設定バリデーション（config/validate.rs）とここで同じ一覧を使う。
#[cfg(windows)]
pub const VALID_SHELLS: &[&str] = &["cmd", "powershell", "pwsh"];
#[cfg(not(windows))]
pub const VALID_SHELLS: &[&str] = &["bash", "sh", "pwsh"];

/// シェル名から、実際に起動する実行ファイル名を返す。
/// この OS で使えないシェル名なら `None`。
///
/// バリデーション（config/validate.rs）が「その実行ファイルが PATH 上にあるか」を
/// 確かめるのにも使う。ここと実行時で別の名前を見ていると検査の意味が無くなるため、
/// 対応表はこの 1 箇所に置く。
pub fn shell_program(shell: &str) -> Option<&'static str> {
    match shell.to_lowercase().as_str() {
        #[cfg(windows)]
        "cmd" => Some("cmd.exe"),
        #[cfg(windows)]
        "powershell" => Some("powershell.exe"),
        #[cfg(windows)]
        "pwsh" => Some("pwsh.exe"),
        #[cfg(not(windows))]
        "pwsh" => Some("pwsh"),
        #[cfg(not(windows))]
        "bash" => Some("bash"),
        #[cfg(not(windows))]
        "sh" => Some("sh"),
        _ => None,
    }
}

/// シェル種別から、起動するプログラムとその引数を組み立てる。
/// 他の設定値と同じく、大文字小文字は区別しない（`cmd` / `CMD` どちらも可）。
fn build_shell_command(shell: &str, expanded: &str) -> Result<(String, Vec<SpawnArg>), AppError> {
    let program = shell_program(shell).ok_or_else(|| {
        AppError::Action(format!(
            "command: 不明なシェル '{shell}'。{} のいずれかを指定してください",
            VALID_SHELLS.join(" / ")
        ))
    })?;

    let args = match shell.to_lowercase().as_str() {
        #[cfg(windows)]
        "cmd" => vec![
            SpawnArg::Quoted("/C".to_string()),
            // cmd.exe は argv 規則ではなく独自の規則でコマンドラインを解釈する。
            // 先頭がダブルクオートのときは最初と最後のダブルクオートを取り除いて
            // 残りをコマンドとして扱うため、コマンド全体を囲んでそのまま渡す。
            // argv 規則でクオートするとエスケープ用のバックスラッシュが文字として
            // 残り、cmd.exe が解釈できずコマンドが壊れる。
            SpawnArg::Raw(wrap_for_cmd(expanded)),
        ],
        #[cfg(not(windows))]
        "bash" | "sh" => quoted_args(&["-c", expanded]),
        // powershell / pwsh
        _ => quoted_args(&["-NoProfile", "-Command", expanded]),
    };

    Ok((program.to_string(), args))
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
