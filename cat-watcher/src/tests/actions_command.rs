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

// この OS で使えるシェルは全部通り、それ以外は弾かれること。
// 使える値は VALID_SHELLS（Windows: cmd/powershell/pwsh、それ以外: bash/sh/pwsh）。
#[test]
fn build_shell_command_accepts_valid_shells_only() {
    for shell in VALID_SHELLS {
        assert!(build_shell_command(shell, "echo test").is_ok(), "shell={shell}");
        // 他の設定値と同じく大文字小文字は区別しない
        assert!(
            build_shell_command(&shell.to_uppercase(), "echo test").is_ok(),
            "shell={shell}（大文字）"
        );
    }
    assert!(build_shell_command("zsh", "echo hi").is_err());
    // 他 OS 用のシェル名も、この OS で起動できないなら弾く
    let foreign = if cfg!(windows) { "bash" } else { "cmd" };
    assert!(build_shell_command(foreign, "echo hi").is_err(), "shell={foreign}");
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

// =========================================================
// cmd.exe のクオート規則（Issue #76）
// =========================================================

/// cmd はコマンド全体をダブルクオートで囲んだ Raw 引数として渡すこと。
///
/// cmd.exe は argv 規則ではなく独自の規則でコマンドラインを解釈し、
/// バックスラッシュをエスケープ文字として扱わない。argv 規則でクオートすると
/// エスケープ用のバックスラッシュが文字として残り、コマンドが壊れる。
#[cfg(windows)]
#[test]
fn cmd_passes_command_raw_and_wrapped_in_quotes() {
    let command = r#"echo RAN > "C:\out\ran.txt""#;
    let (program, args) = build_shell_command("cmd", command).unwrap();

    assert_eq!(program, "cmd.exe");
    assert_eq!(args.len(), 2);
    assert_eq!(args[0], SpawnArg::Quoted("/C".to_string()));
    assert_eq!(args[1], SpawnArg::Raw(format!("\"{command}\"")));

    match &args[1] {
        SpawnArg::Raw(s) => assert!(
            !s.contains("\\\""),
            "cmd へ渡すコマンドにエスケープが入ってはいけない: {s}"
        ),
        other => panic!("Raw を期待したが {other:?} だった"),
    }
}

/// powershell は従来どおり argv 規則でクオートすること（デグレ防止）。
/// powershell.exe は argv 規則で引数を解釈するので、Raw にしてはいけない。
#[cfg(windows)]
#[test]
fn powershell_args_stay_quoted() {
    let (program, args) = build_shell_command("powershell", "echo hi").unwrap();
    assert_eq!(program, "powershell.exe");
    assert_eq!(
        args,
        vec![
            SpawnArg::Quoted("-NoProfile".to_string()),
            SpawnArg::Quoted("-Command".to_string()),
            SpawnArg::Quoted("echo hi".to_string()),
        ]
    );
}

/// bash / sh も argv 規則でクオートすること。
#[cfg(not(windows))]
#[test]
fn bash_args_stay_quoted() {
    let (program, args) = build_shell_command("bash", "echo hi").unwrap();
    assert_eq!(program, "bash");
    assert_eq!(
        args,
        vec![
            SpawnArg::Quoted("-c".to_string()),
            SpawnArg::Quoted("echo hi".to_string()),
        ]
    );
}
