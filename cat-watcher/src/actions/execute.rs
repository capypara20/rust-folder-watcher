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

    let mut cmd = tokio::process::Command::new(program);
    cmd.args(&expanded_args)
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    if let Some(dir) = working_dir {
        cmd.current_dir(dir);
    }

    cmd.spawn().map_err(|e| {
        AppError::Action(format!(
            "execute: プロセス起動失敗 (program={program} args={expanded_args:?}): {e}"
        ))
    })?;

    sink.ok(step.0, step.1, "起動".to_string());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ActionType, LogRotation};
    use crate::logger::Logger;
    use std::sync::Arc;
    use tempfile::tempdir;

    fn make_action(program: &str, args: Vec<&str>, working_dir: &str) -> ActionConfig {
        ActionConfig {
            type_: ActionType::Execute,
            destination: None,
            overwrite: None,
            preserve_structure: None,
            verify_integrity: None,
            shell: None,
            command: None,
            working_dir: Some(working_dir.to_string()),
            program: Some(program.to_string()),
            args: Some(args.into_iter().map(|s| s.to_string()).collect()),
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

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn spawns_program_successfully() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        std::fs::write(&src, b"x").unwrap();
        let ctx = make_ctx(&src, dir.path());
        let action = make_action("cmd.exe", vec!["/C", "echo hello"], "");
        assert!(execute(&action, &ctx, &make_sink(), (1, 1)).await.is_ok());
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn placeholder_expands_in_args() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("report.txt");
        std::fs::write(&src, b"x").unwrap();
        let ctx = make_ctx(&src, dir.path());
        let action = make_action("cmd.exe", vec!["/C", "echo {FullName}"], "");
        assert!(execute(&action, &ctx, &make_sink(), (1, 1)).await.is_ok());
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn working_dir_is_set() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        std::fs::write(&src, b"x").unwrap();
        let ctx = make_ctx(&src, dir.path());
        let action = make_action("cmd.exe", vec!["/C", "echo hi"], dir.path().to_str().unwrap());
        assert!(execute(&action, &ctx, &make_sink(), (1, 1)).await.is_ok());
    }

    #[tokio::test]
    async fn nonexistent_program_returns_error() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        std::fs::write(&src, b"x").unwrap();
        let ctx = make_ctx(&src, dir.path());
        let action = make_action("nonexistent_program_xyz.exe", vec![], "");
        let result = execute(&action, &ctx, &make_sink(), (1, 1)).await;
        assert!(result.is_err());
    }
}
