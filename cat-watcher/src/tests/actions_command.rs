use super::*;
use crate::test_support::{base_action, make_sink};
use crate::config::ActionType;
use tempfile::tempdir;

fn make_action(shell: &str, command: &str, working_dir: &str) -> ActionConfig {
    let mut a = base_action(ActionType::Command);
    a.shell = Some(shell.to_string());
    a.command = Some(command.to_string());
    a.working_dir = Some(working_dir.to_string());
    a
}

fn make_ctx(src: &std::path::Path, watch: &std::path::Path) -> PlaceholderContext {
    PlaceholderContext::new(src, watch, "")
}

#[test]
fn build_shell_command_known_and_unknown_shells() {
    assert!(build_shell_command("cmd", "echo test").is_ok());
    assert!(build_shell_command("powershell", "Get-Date").is_ok());
    assert!(build_shell_command("pwsh", "Get-Date").is_ok());
    assert!(build_shell_command("zsh", "echo hi").is_err());
}

#[tokio::test]
async fn unknown_shell_returns_error() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("a.txt");
    std::fs::write(&src, b"x").unwrap();
    let ctx = make_ctx(&src, dir.path());
    let action = make_action("zsh", "echo hi", "");
    let result = execute(&action, &ctx, &make_sink(), (1, 1)).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("不明なシェル"));
}

#[cfg(not(windows))]
#[tokio::test]
async fn bash_and_sh_spawn_successfully() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("a.txt");
    std::fs::write(&src, b"x").unwrap();
    let ctx = make_ctx(&src, dir.path());
    for shell in ["bash", "sh"] {
        let action = make_action(shell, "echo hello", "");
        assert!(execute(&action, &ctx, &make_sink(), (1, 1)).await.is_ok(), "shell={shell}");
    }
}

/// cmd 起動・プレースホルダ展開・working_dir 指定をまとめて確認する。
#[cfg(target_os = "windows")]
#[tokio::test]
async fn cmd_spawns_with_placeholder_and_working_dir() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("report.txt");
    std::fs::write(&src, b"x").unwrap();
    let ctx = make_ctx(&src, dir.path());
    let action = make_action("cmd", "echo {Name}", dir.path().to_str().unwrap());
    assert!(execute(&action, &ctx, &make_sink(), (1, 1)).await.is_ok());
}
