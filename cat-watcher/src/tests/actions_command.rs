use super::*;
use crate::config::ProcessWait;
use crate::test_support::make_sink;
use tempfile::tempdir;

fn make_action(shell: &str, command: &str, working_dir: &str) -> Command {
    Command {
        shell: shell.to_string(),
        command: command.to_string(),
        working_dir: working_dir.to_string(),
        wait: ProcessWait { enabled: false, timeout_ms: None },
    }
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
    assert!(result.unwrap_err().to_string().contains("この OS では使えません"));
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

// =========================================================
// 外部プロセスの終了コードとタイムアウト
// =========================================================

use crate::actions::spawn::{outcome_message, wait_mode_from, SpawnOutcome, WaitMode};

/// wait = true を立てたアクションを作る。
fn with_wait(mut a: Command, timeout_ms: Option<u64>) -> Command {
    a.wait = ProcessWait { enabled: true, timeout_ms };
    a
}

/// 設定値から待ち方を決めるところ。未指定と 0 の扱いがポイント。
#[test]
fn wait_mode_from_resolves_defaults() {
    use std::time::Duration;

    let wait = |enabled, timeout_ms| ProcessWait { enabled, timeout_ms };

    // 待たない設定なら、timeout_ms があっても待たない
    assert_eq!(wait_mode_from(wait(false, None)), WaitMode::Detach);
    assert_eq!(wait_mode_from(wait(false, Some(1000))), WaitMode::Detach);

    // timeout_ms が無ければ無制限（0 は ProcessWait を作る時点で None に揃えてある。
    // その変換は tests/config_action.rs の timeout_zero_and_absent_both_mean_unlimited が守る）
    assert_eq!(
        wait_mode_from(wait(true, None)),
        WaitMode::Wait { timeout: None }
    );

    assert_eq!(
        wait_mode_from(wait(true, Some(1500))),
        WaitMode::Wait {
            timeout: Some(Duration::from_millis(1500))
        }
    );
}

/// 起動結果が「成功として表示する文言」と「失敗理由」に正しく振り分けられること。
#[test]
fn outcome_message_distinguishes_success_and_failure() {
    // wait = false のときの表記は従来どおり
    assert_eq!(outcome_message(SpawnOutcome::Detached).unwrap(), "起動");
    assert!(outcome_message(SpawnOutcome::Exited(Some(0))).is_ok());
    // 終了コードが取れなかった場合は失敗と断定しない
    assert!(outcome_message(SpawnOutcome::Exited(None)).is_ok());

    let err = outcome_message(SpawnOutcome::Exited(Some(3))).unwrap_err();
    assert!(err.contains("exit=3"), "終了コードが出ていない: {err}");
    assert!(outcome_message(SpawnOutcome::TimedOut)
        .unwrap_err()
        .contains("強制終了"));
}

/// この OS で使えるシェルと、成功／失敗／長時間かかるコマンド。
#[cfg(windows)]
const PROBE: (&str, &str, &str, &str) = ("cmd", "exit 0", "exit 1", "ping -n 10 127.0.0.1 > nul");
#[cfg(not(windows))]
const PROBE: (&str, &str, &str, &str) = ("bash", "exit 0", "exit 1", "sleep 10");

/// wait = true なら終了コードを見て、0 以外はアクション失敗にすること。
/// これが入るまで、アクションログの OK は「起動できた」の意味しか無かった。
#[tokio::test]
async fn wait_true_reports_nonzero_exit_as_error() {
    let (shell, ok_cmd, fail_cmd, _) = PROBE;
    let dir = tempdir().unwrap();
    let src = dir.path().join("a.txt");
    std::fs::write(&src, b"x").unwrap();
    let ctx = make_ctx(&src, dir.path());

    let action = with_wait(make_action(shell, fail_cmd, ""), None);
    let err = execute(&action, &ctx, &make_sink(), (1, 1))
        .await
        .expect_err("終了コード 1 は失敗になること");
    assert!(err.to_string().contains("exit=1"), "{err}");
    // shell やコマンドは直前のアクション開始行に出ているので、エラーには重ねない
    // （以前は「アクション実行エラー: command: 異常終了しました (exit=1) (shell=… cmd=…)」だった）
    assert_eq!(err.to_string(), "異常終了しました (exit=1)");

    let action = with_wait(make_action(shell, ok_cmd, ""), None);
    assert!(execute(&action, &ctx, &make_sink(), (1, 1)).await.is_ok());
}

/// wait 未指定（既定 false）なら終了コードを見ないので、失敗するコマンドでも成功扱い。
/// 既定の挙動が従来から変わっていないことの確認。
#[tokio::test]
async fn wait_false_ignores_exit_code() {
    let (shell, _, fail_cmd, _) = PROBE;
    let dir = tempdir().unwrap();
    let src = dir.path().join("a.txt");
    std::fs::write(&src, b"x").unwrap();
    let ctx = make_ctx(&src, dir.path());

    let action = make_action(shell, fail_cmd, "");
    assert!(
        execute(&action, &ctx, &make_sink(), (1, 1)).await.is_ok(),
        "wait 未指定なら終了コードを見ないこと"
    );
}

/// 上限を超えたプロセスは強制終了してアクション失敗にすること。
/// 無限ループなどで終わらなくなったプロセスを放置しないための保険。
#[tokio::test]
async fn wait_with_timeout_kills_long_running_process() {
    let (shell, _, _, slow_cmd) = PROBE;
    let dir = tempdir().unwrap();
    let src = dir.path().join("a.txt");
    std::fs::write(&src, b"x").unwrap();
    let ctx = make_ctx(&src, dir.path());

    let action = with_wait(make_action(shell, slow_cmd, ""), Some(500));
    let err = execute(&action, &ctx, &make_sink(), (1, 1))
        .await
        .expect_err("上限を超えたら失敗になること");
    assert!(err.to_string().contains("強制終了"), "{err}");
}

/// タイムアウトでシェルの「子」まで止まること。
///
/// Windows では親プロセスを終了させても子は生き残る。command アクションは必ず
/// シェルを経由するので、実際の処理をするプロセスは常に孫になる。シェルだけを
/// 殺すと「止めたつもりで処理が続く」状態になり、ログが嘘をつく。
///
/// **目印ファイルを作るのは孫プロセス自身**にしてある。シェルが書く形にすると、
/// シェルを殺しただけで目印が作られなくなり、孫が生きていても検出できない。
#[tokio::test]
async fn timeout_kills_whole_process_tree() {
    use std::time::Duration;

    let dir = tempdir().unwrap();
    let src = dir.path().join("a.txt");
    std::fs::write(&src, b"x").unwrap();
    let ctx = make_ctx(&src, dir.path());

    let marker = dir.path().join("marker.txt");
    // パスはそのまま埋め込む（ソースにバックスラッシュを書かないため）。
    let m = marker.display().to_string();

    // 入れ子のシェルが孫プロセスになる。目印を書くのはその孫。
    #[cfg(windows)]
    let command = format!("cmd /C {Q}ping -n 3 127.0.0.1 > nul & echo done > {m}{Q}", Q = '"');
    // 末尾の `; :` が重要。bash は「単一コマンドだけ」のとき exec で自分を置き換える
    // 最適化をするため、それだと孫プロセスができずテストが空振りになる。
    #[cfg(not(windows))]
    let command = format!("bash -c 'sleep 2; echo done > {m}'; :");

    let action = with_wait(make_action(PROBE.0, &command, ""), Some(500));
    let err = execute(&action, &ctx, &make_sink(), (1, 1))
        .await
        .expect_err("上限を超えたら失敗になること");
    assert!(err.to_string().contains("強制終了"), "{err}");

    // 孫が生きていれば完走して目印を作る。それより長く待ってから確認する。
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert!(
        !marker.exists(),
        "シェルの子プロセスが生き残って処理を続けている: {}",
        marker.display()
    );
}
