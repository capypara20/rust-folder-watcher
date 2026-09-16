use crate::config::Command;
use crate::error::AppError;
use crate::placeholder::{expand_placeholders, PlaceholderContext};

use super::spawn::{outcome_message, spawn_process, wait_mode_from, SpawnArg};
use super::ActionSink;

/// command アクションを実行する。
///
/// エラーの文面には shell やコマンドを入れない。直前のアクション開始行に
/// `shell=… command=…` が出ているので、重ねると読みにくくなるため。
pub async fn execute(
    command: &Command,
    ctx: &PlaceholderContext,
    sink: &ActionSink,
    step: (usize, usize),
) -> Result<(), AppError> {
    let expanded = expand_placeholders(&command.command, ctx);
    let working_dir = Some(command.working_dir.as_str()).filter(|s| !s.is_empty());

    let (program, args) = build_shell_command(&command.shell, &expanded)?;
    let outcome = spawn_process(&program, &args, working_dir, wait_mode_from(command.wait))
        .await
        .map_err(|e| AppError::Action(format!("シェル '{program}' を起動できません: {e}")))?;

    match outcome_message(outcome) {
        Ok(msg) => {
            sink.ok(step.0, step.1, msg);
            Ok(())
        }
        // wait = true のときだけ起こる。アクション失敗として扱うので、
        // 以降のアクションチェーンは中断される。
        Err(reason) => Err(AppError::Action(reason)),
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
    // 起動時の検査で弾いているので、実行時にここへ来るのは検査をすり抜けた場合だけ。
    let program = shell_program(shell).ok_or_else(|| {
        AppError::Action(format!(
            "シェル '{shell}' はこの OS では使えません（{} のいずれか）",
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
