//! エラー型（`error.rs`）のテスト。
//!
//! 守りたいのは 2 つ。
//! - 終了コードが「起動前（設定）」と「実行中」で正しく分かれること
//! - 表示がそれだけで意味の通る文になっていて、どのファイルの話か分かること

use super::*;
use crate::config::problem::Location;

/// 場所を気にしないテスト用の問題。
fn problem(message: &str) -> Problem {
    Problem::new(Location::File, message)
}

fn io_error() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::NotFound, "見つかりません")
}

/// 起動前（使い方・設定）の失敗はすべて CONFIG になること。
///
/// 以前は設定ファイルが読めないだけの失敗が `Io` になり、
/// 「ログの初期化に失敗」を意味する終了コードで報告されていた。
#[test]
fn setup_failures_use_config_exit_code() {
    let setup = [
        AppError::Usage("x".into()),
        AppError::ConfigRead {
            path: PathBuf::from("global.toml"),
            source: io_error(),
        },
        AppError::ConfigParse {
            path: PathBuf::from("rules.toml"),
            message: "x".into(),
        },
        AppError::ConfigInvalid(vec![InvalidFile::new(Path::new("rules.toml"), vec![problem("x")])]),
        AppError::TemplateWrite {
            path: PathBuf::from("global.toml"),
            source: io_error(),
        },
    ];
    for e in &setup {
        assert_eq!(e.exit_code(), exit_code::CONFIG, "{e:?}");
    }
}

/// 実行中の失敗はすべて RUNTIME になること。
///
/// 以前はサービス登録や非同期ランタイムの作成失敗が `Config` になり、
/// 設定の問題として報告されていた。
#[test]
fn runtime_failures_use_runtime_exit_code() {
    let runtime = [
        AppError::Runtime("x".into()),
        AppError::Watch("x".into()),
        AppError::Action("x".into()),
        AppError::FileHash("x".into()),
    ];
    for e in &runtime {
        assert_eq!(e.exit_code(), exit_code::RUNTIME, "{e:?}");
    }
}

/// 0 は成功を意味するので、2 つの終了コードは 0 でなく、互いに区別できること。
#[test]
fn exit_codes_are_nonzero_and_distinct() {
    assert_ne!(exit_code::CONFIG, 0);
    assert_ne!(exit_code::RUNTIME, 0);
    assert_ne!(exit_code::CONFIG, exit_code::RUNTIME);
}

/// 読み込みに失敗したとき、どのファイルかが表示に入ること。
#[test]
fn config_read_names_the_file() {
    let e = AppError::ConfigRead {
        path: PathBuf::from("my-rules.toml"),
        source: io_error(),
    };
    let text = e.to_string();
    assert!(text.contains("my-rules.toml"), "{text}");
    assert!(text.contains("見つかりません"), "元の原因が落ちている: {text}");
}

/// 書式エラーでも、どのファイルかが表示に入ること（行番号だけでは足りない）。
#[test]
fn config_parse_names_the_file() {
    let e = AppError::ConfigParse {
        path: PathBuf::from("my-global.toml"),
        message: "line 2".into(),
    };
    let text = e.to_string();
    assert!(text.contains("my-global.toml"), "{text}");
    assert!(text.contains("line 2"), "{text}");
}

/// 内容の問題は、ファイルごとに件数と番号付きの一覧で出ること。
#[test]
fn config_invalid_lists_problems_per_file() {
    let e = AppError::ConfigInvalid(vec![
        InvalidFile::new(Path::new("global.toml"), vec![problem("g1")]),
        InvalidFile::new(Path::new("rules.toml"), vec![problem("r1"), problem("r2")]),
    ]);
    let text = e.to_string();

    assert!(text.contains("1 件の問題があります: global.toml"), "{text}");
    assert!(text.contains("2 件の問題があります: rules.toml"), "{text}");
    // 番号の後ろに場所の見出し、次の行に内容が来る
    assert!(text.contains("[1] （ファイル全体）"), "{text}");
    assert!(text.contains("[2] （ファイル全体）"), "{text}");
    for message in ["g1", "r1", "r2"] {
        assert!(text.lines().any(|l| l.trim() == message), "{message} が 1 行になっていない: {text}");
    }
    // global の一覧の後に rules の一覧が来る（混ざらない）
    assert!(text.find("g1").unwrap() < text.find("rules.toml").unwrap(), "{text}");
}

/// 表示に前置きが重ならないこと。
///
/// 以前は「実行エラー: バリデーションエラー: バリデーションエラーが 2 件…」と
/// 同じ意味の前置きが 3 つ並んでいた。
#[test]
fn config_invalid_has_no_stacked_prefixes() {
    let e = AppError::ConfigInvalid(vec![InvalidFile::new(Path::new("rules.toml"), vec![problem("x")])]);
    let text = e.to_string();
    assert!(!text.contains("エラー:"), "前置きが付いている: {text}");
}
