//! Windows サービス起動まわりのテスト。

use super::*;

/// 終了コードが原因の種類ごとに分かれること。
///
/// 従来は失敗が常に `Win32(1)`（「ファンクションが間違っています」）で、
/// `sc query` を見ても設定エラーなのか実行時エラーなのか分からなかった。
#[test]
fn exit_code_distinguishes_failure_kinds() {
    assert_eq!(
        exit_code_for(&AppError::Config("x".into())),
        exit_code::CONFIG
    );
    assert_eq!(
        exit_code_for(&AppError::Validation("x".into())),
        exit_code::CONFIG
    );
    assert_eq!(
        exit_code_for(&AppError::TomlParse("x".into())),
        exit_code::CONFIG
    );
    assert_eq!(
        exit_code_for(&AppError::Io(std::io::Error::other("x"))),
        exit_code::LOG
    );
    assert_eq!(
        exit_code_for(&AppError::Watch("x".into())),
        exit_code::RUNTIME
    );

    // 0 は成功を意味するので、失敗に 0 を返してはいけない。
    for e in [AppError::Config("x".into()), AppError::Watch("x".into())] {
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
