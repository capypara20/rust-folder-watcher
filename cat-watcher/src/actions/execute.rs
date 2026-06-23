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

    let working_dir = action
        .working_dir
        .as_deref()
        .filter(|s| !s.is_empty());

    spawn_detached(program, &expanded_args, working_dir).map_err(|e| {
        AppError::Action(format!(
            "execute: プロセス起動失敗 (program={program} args={expanded_args:?}): {e}"
        ))
    })?;

    sink.ok(step.0, step.1, "起動".to_string());
    Ok(())
}

#[cfg(test)]
#[path = "../tests/actions_execute.rs"]
mod tests;
