use crate::config::Execute;
use crate::error::AppError;
use crate::placeholder::{expand_placeholders, PlaceholderContext};

use super::spawn::{outcome_message, spawn_process, wait_mode_from, SpawnArg};
use super::ActionSink;

/// execute アクションを実行する。
///
/// エラーの文面にはプログラムや引数を入れない。直前のアクション開始行に
/// 出ているので、重ねると読みにくくなるため。
pub async fn execute(
    execute: &Execute,
    ctx: &PlaceholderContext,
    sink: &ActionSink,
    step: (usize, usize),
) -> Result<(), AppError> {
    // execute は args が配列なので、各要素をそのまま 1 引数として渡せばよい。
    // クオートは argv 規則に任せる（cmd.exe のような独自規則の相手はいない）。
    let args: Vec<SpawnArg> = execute
        .args
        .iter()
        .map(|a| SpawnArg::Quoted(expand_placeholders(a, ctx)))
        .collect();
    let working_dir = Some(execute.working_dir.as_str()).filter(|s| !s.is_empty());

    let outcome = spawn_process(&execute.program, &args, working_dir, wait_mode_from(execute.wait))
        .await
        .map_err(|e| AppError::Action(format!("プログラムを起動できません: {e}")))?;

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

#[cfg(test)]
#[path = "../tests/actions_execute.rs"]
mod tests;
