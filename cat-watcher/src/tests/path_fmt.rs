//! ログ用パス表記のテスト。

use super::*;

/// 設定由来の `/` と OS 由来の区切りが混ざったパスが、1 種類に揃うこと。
/// これが元の不具合（`C:/watch\file.txt` のような表示）。
#[test]
fn mixed_separators_become_consistent() {
    let mixed = format!("C:/watch{}file.txt", std::path::MAIN_SEPARATOR);
    let out = normalize(&mixed);

    #[cfg(windows)]
    {
        assert_eq!(out, "C:\u{5c}watch\u{5c}file.txt");
        assert!(!out.contains('/'), "スラッシュが残っている: {out}");
    }
    #[cfg(not(windows))]
    {
        // Unix は元から / 区切りなので変化しない。
        assert_eq!(out, mixed);
    }
}

/// Unix ではバックスラッシュを置き換えないこと。
///
/// Unix ではバックスラッシュがファイル名に使える文字なので、区切りとみなして
/// 置き換えるとファイル名を壊してしまう。
#[cfg(not(windows))]
#[test]
fn backslash_in_filename_is_preserved_on_unix() {
    let name = "/tmp/odd\u{5c}name.txt";
    assert_eq!(normalize(name), name, "ファイル名のバックスラッシュを壊している");
}

/// Path からでも同じ結果になること。
#[test]
fn for_log_matches_normalize() {
    let path = std::path::PathBuf::from("C:/a/b.txt");
    assert_eq!(for_log(&path), normalize(&path.display().to_string()));
}

/// 区切りを含まないパスはそのまま返ること。
#[test]
fn plain_name_is_unchanged() {
    assert_eq!(normalize("file.txt"), "file.txt");
}
