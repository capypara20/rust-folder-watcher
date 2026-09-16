//! 実行ファイルの PATH 解決のテスト。

use super::*;

/// この OS で必ず PATH 上にあるコマンド。
#[cfg(windows)]
const ON_PATH: &str = "cmd";
#[cfg(not(windows))]
const ON_PATH: &str = "sh";

/// 名前だけの指定を PATH から解決できること。
#[test]
fn resolves_bare_name_from_path() {
    match resolve(ON_PATH) {
        Resolved::Found(p) => assert!(p.is_file(), "見つかったはずのパスが実在しない: {}", p.display()),
        other => panic!("{ON_PATH} を PATH から解決できなかった: {other:?}"),
    }
}

/// Windows では拡張子を省略しても PATHEXT で解決できること。
#[cfg(windows)]
#[test]
fn resolves_without_extension_using_pathext() {
    // 拡張子あり・なしのどちらでも同じ実体に辿り着く。
    let without = resolve("cmd");
    let with = resolve("cmd.exe");
    assert!(matches!(without, Resolved::Found(_)), "{without:?}");
    assert!(matches!(with, Resolved::Found(_)), "{with:?}");
}

/// PATH 上に無い名前は NotOnPath になること。
/// ここが Found になってしまうと、起動時チェックが素通りする。
#[test]
fn missing_bare_name_is_not_on_path() {
    let result = resolve("cat-watcher-definitely-not-a-real-command-xyz");
    assert_eq!(result, Resolved::NotOnPath, "{result:?}");
}

/// 区切り文字を含む指定は PATH を探さず、その場所だけを見ること。
///
/// PATH 上に同名の実行ファイルがあっても拾わない（OS の挙動に合わせる）。
#[test]
fn path_like_input_is_not_searched_on_path() {
    let dir = tempfile::tempdir().unwrap();
    // PATH 上に存在する名前を、存在しないディレクトリ配下で指定する
    let missing = dir.path().join("nested").join(ON_PATH);
    let result = resolve(missing.to_str().unwrap());
    assert_eq!(
        result,
        Resolved::MissingAtPath,
        "区切りを含む指定なのに PATH から拾ってしまった: {result:?}"
    );
}

/// 実在するファイルを絶対パスで指定したら解決できること。
#[test]
fn resolves_absolute_path() {
    let dir = tempfile::tempdir().unwrap();
    #[cfg(windows)]
    let name = "tool.exe";
    #[cfg(not(windows))]
    let name = "tool";
    let path = dir.path().join(name);
    std::fs::write(&path, b"x").unwrap();

    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = std::fs::metadata(&path).unwrap().permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(&path, perm).unwrap();
    }

    match resolve(path.to_str().unwrap()) {
        Resolved::Found(found) => assert_eq!(found, path),
        other => panic!("絶対パスを解決できなかった: {other:?}"),
    }
}

/// 空文字は解決できないこと（設定の書き忘れを素通りさせない）。
#[test]
fn empty_input_is_not_resolved() {
    assert_eq!(resolve(""), Resolved::NotOnPath);
    assert_eq!(resolve("   "), Resolved::NotOnPath);
}

/// エラーメッセージ用の PATH 表示が、長すぎず空でもないこと。
///
/// PATH は 40 件を超えることも珍しくなく、全部並べるとエラー本文に埋もれて
/// 「何が見つからないのか」が読めなくなる。先頭だけ出して件数を添える。
#[test]
fn search_path_summary_is_capped() {
    let entries = search_path_entries();
    let summary = search_path_summary();
    assert!(!summary.is_empty());

    if entries.len() > 8 {
        assert!(
            summary.contains("... 他"),
            "件数が多いのに省略されていない: {summary}"
        );
        // 先頭 8 件 + 省略行 + 件数行。全部は並べない。
        assert!(
            summary.lines().count() <= 10,
            "行数が多すぎる: {}",
            summary.lines().count()
        );
    }
    assert!(
        summary.starts_with(&format!("{} 件", entries.len())),
        "件数が先頭に無い: {summary}"
    );
}

/// 実行ビットが無いファイルは解決せず、「存在しない」とも区別すること（Unix 固有）。
///
/// ファイルが在るだけで OK にしてしまうと、実行できない設定ファイルや
/// テキストを program に書いても起動時チェックを素通りしてしまう。
/// 一方で「存在しません」と出すと、ファイルが在るのに探し回ることになる。
#[cfg(not(windows))]
#[test]
fn file_without_execute_bit_is_not_resolved() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("script.sh");
    std::fs::write(&path, b"#!/bin/sh\n").unwrap();

    let set_mode = |mode: u32| {
        let mut perm = std::fs::metadata(&path).unwrap().permissions();
        perm.set_mode(mode);
        std::fs::set_permissions(&path, perm).unwrap();
    };

    // 実行ビット無し → 解決しない。「存在しない」ではなく「実行できない」と返す
    set_mode(0o644);
    assert_eq!(
        resolve(path.to_str().unwrap()),
        Resolved::NotExecutable(path.clone()),
        "実行ビットが無いファイルを正しく区別できていない"
    );

    // 実行ビットを付ければ解決する
    set_mode(0o755);
    match resolve(path.to_str().unwrap()) {
        Resolved::Found(found) => assert_eq!(found, path),
        other => panic!("実行ビットを付けたのに解決できない: {other:?}"),
    }
}

/// ディレクトリは実行ファイルとして解決しないこと。
/// PATH 上に同名のディレクトリがあるときに誤検出しないため。
#[test]
fn directory_is_not_resolved_as_executable() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("notanexe");
    std::fs::create_dir(&sub).unwrap();

    assert_eq!(
        resolve(sub.to_str().unwrap()),
        Resolved::MissingAtPath,
        "ディレクトリを実行ファイルとして解決してしまった"
    );
}

// =========================================================
// PATH 探索（環境変数を書き換えずに、ディレクトリ列を直接渡して確かめる）
// =========================================================

/// テスト用に、実行権限の有無を指定してファイルを置く。
/// Windows には実行ビットが無いので、拡張子だけ合わせて置く。
fn place(dir: &Path, name: &str, executable: bool) -> PathBuf {
    #[cfg(windows)]
    let name = format!("{name}.exe");
    let path = dir.join(name);
    std::fs::write(&path, b"x").unwrap();
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = std::fs::metadata(&path).unwrap().permissions();
        perm.set_mode(if executable { 0o755 } else { 0o644 });
        std::fs::set_permissions(&path, perm).unwrap();
    }
    #[cfg(windows)]
    let _ = executable;
    path
}

/// どのディレクトリにも無ければ NotOnPath。
#[test]
fn search_dirs_reports_not_on_path_when_absent() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let dirs = vec![a.path().to_path_buf(), b.path().to_path_buf()];
    assert_eq!(search_dirs(dirs, "tool"), Resolved::NotOnPath);
}

/// 実行できないものしか無ければ、それを NotExecutable として報告すること（Unix 固有）。
/// 「PATH 上に見つかりません」と出すと、ファイルが在るのに PATH を疑うことになる。
#[cfg(not(windows))]
#[test]
fn search_dirs_reports_non_executable_match() {
    let a = tempfile::tempdir().unwrap();
    let path = place(a.path(), "tool", false);
    let dirs = vec![a.path().to_path_buf()];
    assert_eq!(search_dirs(dirs, "tool"), Resolved::NotExecutable(path));
}

/// 手前に実行できない同名ファイルがあっても、後ろに実行できるものがあればそちらを使うこと。
/// OS も実行できないファイルは飛ばして探し続けるので、それに合わせる（Unix 固有）。
#[cfg(not(windows))]
#[test]
fn search_dirs_skips_non_executable_and_keeps_searching() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    place(first.path(), "tool", false);
    let good = place(second.path(), "tool", true);

    let dirs = vec![first.path().to_path_buf(), second.path().to_path_buf()];
    assert_eq!(search_dirs(dirs, "tool"), Resolved::Found(good));
}

/// 先に見つかった実行ファイルが使われること（PATH の順序を守る）。
#[test]
fn search_dirs_respects_order() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let winner = place(first.path(), "tool", true);
    place(second.path(), "tool", true);

    let dirs = vec![first.path().to_path_buf(), second.path().to_path_buf()];
    // Windows は PATHEXT 由来の大文字拡張子（tool.EXE）で見つかることがあるので、
    // ファイル名ではなく「どのディレクトリで見つかったか」で比べる。
    match search_dirs(dirs, "tool") {
        Resolved::Found(found) => assert_eq!(found.parent(), winner.parent()),
        other => panic!("見つかるはずが見つからない: {other:?}"),
    }
}
