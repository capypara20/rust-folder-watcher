use super::*;
use crate::test_support::{base_action, make_sink};
use crate::config::ActionType;
use tempfile::tempdir;

fn make_action(program: &str, args: Vec<&str>, working_dir: &str) -> ActionConfig {
    let mut a = base_action(ActionType::Execute);
    a.program = Some(program.to_string());
    a.args = Some(args.into_iter().map(|s| s.to_string()).collect());
    a.working_dir = Some(working_dir.to_string());
    a
}

fn make_ctx(src: &std::path::Path, watch: &std::path::Path) -> PlaceholderContext {
    PlaceholderContext::new(src, watch, "")
}

/// プログラム起動・引数内プレースホルダ展開・working_dir 指定をまとめて確認する。
#[cfg(target_os = "windows")]
#[tokio::test]
async fn spawns_program_with_placeholder_and_working_dir() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("report.txt");
    std::fs::write(&src, b"x").unwrap();
    let ctx = make_ctx(&src, dir.path());
    let action = make_action(
        "cmd.exe",
        vec!["/C", "echo {FullName}"],
        dir.path().to_str().unwrap(),
    );
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
