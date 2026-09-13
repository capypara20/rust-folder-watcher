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
    let program = action
        .program
        .as_deref()
        .ok_or_else(|| AppError::Action("execute: program が未指定".to_string()))?;

    let raw_args = action
        .args
        .as_deref()
        .ok_or_else(|| AppError::Action("execute: args が未指定".to_string()))?;

    let expanded_args: Vec<String> = raw_args
        .iter()
        .map(|a| expand_placeholders(a, ctx))
        .collect::<Result<_, _>>()?;

    // execute は args が配列なので、各要素をそのまま 1 引数として渡せばよい。
    // クオートは argv 規則に任せる（cmd.exe のような独自規則の相手はいない）。
    let spawn_args: Vec<SpawnArg> = expanded_args
        .iter()
        .map(|a| SpawnArg::Quoted(a.clone()))
        .collect();

    let working_dir = action
        .working_dir
        .as_deref()
        .filter(|s| !s.is_empty());

    let wait = wait_mode_from(action.wait, action.timeout_ms);

    let outcome = spawn_process(program, &spawn_args, working_dir, wait)
        .await
        .map_err(|e| {
            AppError::Action(format!(
                "execute: プロセス起動失敗 (program={program} args={expanded_args:?}): {e}"
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
            "execute: {reason} (program={program} args={expanded_args:?})"
        ))),
    }
}

#[cfg(test)]
#[path = "../tests/actions_execute.rs"]
mod tests;
