//! Windows サービス起動まわりのテスト。

use super::*;

/// SCM へ報告する終了コードが、CLI と同じ値で、段階ごとに分かれること。
///
/// 従来は失敗が常に `Win32(1)`（「ファンクションが間違っています」）で、
/// `sc query` を見ても設定エラーなのか実行時エラーなのか分からなかった。
#[test]
fn exit_code_distinguishes_failure_kinds() {
    use crate::error::exit_code;

    let config = AppError::Usage("x".into());
    let runtime = AppError::Runtime("x".into());
    assert_eq!(exit_code_for(&config), exit_code::CONFIG as u32);
    assert_eq!(exit_code_for(&runtime), exit_code::RUNTIME as u32);

    // 0 は成功を意味するので、失敗に 0 を返してはいけない。
    for e in [config, runtime] {
        assert_ne!(exit_code_for(&e), 0, "失敗なのに成功扱いの終了コード");
    }
}

/// 記録先の候補が必ず 1 つ以上あり、固定のファイル名で終わること。
///
/// ここが空になると、起動失敗の内容がどこにも残らなくなる。
#[test]
fn startup_error_log_paths_are_not_empty() {
    let paths = startup_error_log_paths();
    assert!(!paths.is_empty(), "書き込み先の候補が 1 つも無い");
    for p in &paths {
        assert_eq!(
            p.file_name().and_then(|n| n.to_str()),
            Some(STARTUP_ERROR_LOG),
            "候補のファイル名が違う: {}",
            p.display()
        );
    }
}

/// 追記モードであること。2 回目の失敗で 1 回目の記録が消えない。
#[test]
fn append_text_appends_instead_of_truncating() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.log");

    append_text(&path, "1 回目\n").unwrap();
    append_text(&path, "2 回目\n").unwrap();

    let body = std::fs::read_to_string(&path).unwrap();
    assert!(body.contains("1 回目"), "1 回目が消えている: {body}");
    assert!(body.contains("2 回目"), "2 回目が書けていない: {body}");
}
