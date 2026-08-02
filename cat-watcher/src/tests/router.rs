use super::*;
use notify::event::{DataChange, RenameMode};
use std::collections::HashSet;
use tempfile::TempDir;

fn make_rule(watch_path: &str, recursive: bool, patterns: Option<Vec<&str>>) -> CompiledRule {
    let glob_set = patterns.map(|pats| glob_set(&pats));
    CompiledRule {
        name: format!("rule-{}", watch_path),
        enabled: true,
        watch_path: watch_path.to_string(),
        recursive,
        target: WatchTarget::Both,
        include_hidden: false,
        events: vec![Event::Create],
        glob_set,
        exclude_glob_set: None,
        exclude_regex: None,
        exclude_dir_glob_set: None,
        exclude_dir_regex: None,
        dir_glob_set: None,
        dir_regex: None,
        regexes: None,
        actions: vec![],
        detect_logger: None,
        action_logger: None,
    }
}

fn glob_set(patterns: &[&str]) -> GlobSet {
    let mut builder = GlobSetBuilder::new();
    for p in patterns {
        builder.add(Glob::new(p).unwrap());
    }
    builder.build().unwrap()
}

fn create_events(e: Event) -> HashSet<Event> {
    let mut s = HashSet::new();
    s.insert(e);
    s
}

// 2つの監視ディレクトリを用意し、片方で検知したファイルが
// もう片方のルールに誤マッチしないことを確認する（バグ再現テスト）
#[test]
fn test_no_cross_directory_match() {
    let dir_a = TempDir::new().unwrap();
    let dir_b = TempDir::new().unwrap();

    let rule_a = make_rule(dir_a.path().to_str().unwrap(), false, Some(vec!["*.csv"]));
    let rule_b = make_rule(dir_b.path().to_str().unwrap(), false, Some(vec!["*.csv"]));

    let file_in_a = dir_a.path().join("data.csv");
    std::fs::write(&file_in_a, "").unwrap();
    let events = create_events(Event::Create);

    assert!(evaluate_rule(&file_in_a, &events, None, &rule_a), "dir_a のルールはマッチすべき");
    assert!(!evaluate_rule(&file_in_a, &events, None, &rule_b), "dir_b のルールはマッチしてはいけない");
}

// recursive フラグとパターンによるマッチ判定
#[test]
fn test_recursive_flag_and_pattern_matching() {
    let dir = TempDir::new().unwrap();
    let subdir = dir.path().join("sub");
    std::fs::create_dir(&subdir).unwrap();
    let direct = dir.path().join("data.csv");
    let in_sub = subdir.join("data.csv");
    let mismatch = dir.path().join("image.png");
    for f in [&direct, &in_sub, &mismatch] {
        std::fs::write(f, "").unwrap();
    }
    let events = create_events(Event::Create);

    let non_recursive = make_rule(dir.path().to_str().unwrap(), false, Some(vec!["*.csv"]));
    assert!(evaluate_rule(&direct, &events, None, &non_recursive), "直下のファイルはマッチすべき");
    assert!(!evaluate_rule(&in_sub, &events, None, &non_recursive), "recursive=false ではサブディレクトリを除外");
    assert!(!evaluate_rule(&mismatch, &events, None, &non_recursive), "パターン不一致は除外");

    let recursive = make_rule(dir.path().to_str().unwrap(), true, Some(vec!["*.csv"]));
    assert!(evaluate_rule(&in_sub, &events, None, &recursive), "recursive=true ならサブディレクトリもマッチすべき");
}

// to_config_event: Rename / Modify サブタイプのマッピング (#30 回帰)
#[test]
fn test_to_config_event_subtypes() {
    let cases = [
        (EventKind::Modify(ModifyKind::Name(RenameMode::From)), Some(Event::Rename)),
        (EventKind::Modify(ModifyKind::Name(RenameMode::To)), Some(Event::Rename)),
        (EventKind::Modify(ModifyKind::Name(RenameMode::Any)), Some(Event::Rename)),
        (EventKind::Modify(ModifyKind::Data(DataChange::Content)), Some(Event::Modify)),
    ];
    for (kind, expected) in cases {
        assert_eq!(to_config_event(&kind), expected, "kind: {kind:?}");
    }
}

// matches_target: kind ベース判定 (#23 / #24 回帰)
// Remove イベント等ではパスが既に存在しないため、kind だけで判定できる必要がある。
// target=both は kind に関係なく常に true。
#[test]
fn test_matches_target_kind_based() {
    let path = Path::new("/nonexistent/will_not_exist");
    assert!(matches_target(path, &WatchTarget::File, Some(EntryKind::File)));
    assert!(!matches_target(path, &WatchTarget::File, Some(EntryKind::Dir)));
    assert!(matches_target(path, &WatchTarget::Directory, Some(EntryKind::Dir)));
    assert!(!matches_target(path, &WatchTarget::Directory, Some(EntryKind::File)));
    assert!(matches_target(path, &WatchTarget::Both, Some(EntryKind::File)));
    assert!(matches_target(path, &WatchTarget::Both, Some(EntryKind::Dir)));
    assert!(matches_target(path, &WatchTarget::Both, None));
}

// kind=None の場合は実ファイルシステムへの fallback (Modify/Rename パス)。
// パスが存在しない場合は判定不能なので target=file / directory どちらにも通さない (厳格)。
// 旧 OS で Delete を確実に拾いたい場合は target=both を使う運用とする。
#[test]
fn test_matches_target_kind_none_fallback() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("real.txt");
    std::fs::write(&file, "").unwrap();
    assert!(matches_target(&file, &WatchTarget::File, None));
    assert!(!matches_target(&file, &WatchTarget::Directory, None));

    let missing = Path::new("/definitely/does/not/exist_xyz_12345");
    assert!(!matches_target(missing, &WatchTarget::File, None));
    assert!(!matches_target(missing, &WatchTarget::Directory, None));
    assert!(matches_target(missing, &WatchTarget::Both, None));
}

// exclude_regex: ファイル名正規表現除外 (#28)
#[test]
fn test_exclude_regex() {
    let dir = TempDir::new().unwrap();
    let excluded = dir.path().join("debug_001.log");
    let passed = dir.path().join("report_001.log");
    std::fs::write(&excluded, "").unwrap();
    std::fs::write(&passed, "").unwrap();

    let mut rule = make_rule(dir.path().to_str().unwrap(), false, None);
    rule.exclude_regex = Some(Regex::new(r"^debug_\d+").unwrap());
    let events = create_events(Event::Create);

    assert!(!evaluate_rule(&excluded, &events, None, &rule));
    assert!(evaluate_rule(&passed, &events, None, &rule));
}

// exclude_dir_patterns: フォルダ名 glob 除外 (#28)
// 直接の親だけでなくネストした途中ディレクトリも対象。
#[test]
fn test_exclude_dir_patterns() {
    let dir = TempDir::new().unwrap();
    let in_excluded = dir.path().join("node_modules/index.js");
    let in_nested = dir.path().join("packages/node_modules/dep.js");
    let in_passed = dir.path().join("src/main.rs");
    for f in [&in_excluded, &in_nested, &in_passed] {
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, "").unwrap();
    }

    let mut rule = make_rule(dir.path().to_str().unwrap(), true, None);
    rule.exclude_dir_glob_set = Some(glob_set(&["node_modules"]));
    let events = create_events(Event::Create);

    assert!(!evaluate_rule(&in_excluded, &events, None, &rule));
    assert!(!evaluate_rule(&in_nested, &events, None, &rule));
    assert!(evaluate_rule(&in_passed, &events, None, &rule));
}

// exclude_dir_regex: フォルダ名正規表現除外 (#28)
#[test]
fn test_exclude_dir_regex() {
    let dir = TempDir::new().unwrap();
    let in_hidden = dir.path().join(".cache/data.bin");
    let in_visible = dir.path().join("data/report.csv");
    for f in [&in_hidden, &in_visible] {
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, "").unwrap();
    }

    let mut rule = make_rule(dir.path().to_str().unwrap(), true, None);
    rule.exclude_dir_regex = Some(Regex::new(r"^\.").unwrap());
    let events = create_events(Event::Create);

    assert!(!evaluate_rule(&in_hidden, &events, None, &rule));
    assert!(evaluate_rule(&in_visible, &events, None, &rule));
}

// dir_patterns: フォルダ名 glob 包含 (#28)
// watch 直下のファイルはディレクトリコンポーネントが存在しないためマッチしない。
// 深いネストでも途中にマッチするフォルダがあれば通る。
#[test]
fn test_dir_patterns() {
    let dir = TempDir::new().unwrap();
    let in_src = dir.path().join("src/main.rs");
    let in_other = dir.path().join("lib/utils.rs");
    let in_nested = dir.path().join("packages/src/components/button.tsx");
    let direct = dir.path().join("root.csv");
    for f in [&in_src, &in_other, &in_nested, &direct] {
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, "").unwrap();
    }

    let mut rule = make_rule(dir.path().to_str().unwrap(), true, None);
    rule.dir_glob_set = Some(glob_set(&["src"]));
    let events = create_events(Event::Create);

    assert!(evaluate_rule(&in_src, &events, None, &rule));
    assert!(!evaluate_rule(&in_other, &events, None, &rule));
    assert!(evaluate_rule(&in_nested, &events, None, &rule));
    assert!(!evaluate_rule(&direct, &events, None, &rule));
}

// ── include_hidden (#69) ──────────────────────────────────────────────
// 「隠し」の作り方は OS で違うため、テスト側もプラットフォームごとに分ける。
//   Windows: FILE_ATTRIBUTE_HIDDEN 属性を立てる
//   それ以外: ファイル名を "." 始まりにする

/// 隠しエントリを 1 つ作り、そのパスを返す。
#[cfg(windows)]
fn make_hidden_file(dir: &Path) -> PathBuf {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{SetFileAttributesW, FILE_ATTRIBUTE_HIDDEN};

    let path = dir.join("secret.txt");
    std::fs::write(&path, "").unwrap();
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: wide は NUL 終端された有効な UTF-16 バッファ。
    let ok = unsafe { SetFileAttributesW(wide.as_ptr(), FILE_ATTRIBUTE_HIDDEN) };
    assert!(ok != 0, "隠し属性の付与に失敗");
    path
}

#[cfg(not(windows))]
fn make_hidden_file(dir: &Path) -> PathBuf {
    let path = dir.join(".secret.txt");
    std::fs::write(&path, "").unwrap();
    path
}

// include_hidden = false のとき隠しエントリは除外され、通常ファイルは通る。
#[test]
fn test_include_hidden_false_excludes_hidden_entry() {
    let dir = TempDir::new().unwrap();
    let hidden = make_hidden_file(dir.path());
    let visible = dir.path().join("open.txt");
    std::fs::write(&visible, "").unwrap();

    let rule = make_rule(dir.path().to_str().unwrap(), false, None);
    assert!(!rule.include_hidden, "make_rule の既定は include_hidden=false");
    let events = create_events(Event::Create);

    assert!(!evaluate_rule(&hidden, &events, None, &rule), "隠しエントリは除外されるべき");
    assert!(evaluate_rule(&visible, &events, None, &rule), "通常ファイルは通るべき");
}

// include_hidden = true なら隠しエントリも検知対象になる。
#[test]
fn test_include_hidden_true_allows_hidden_entry() {
    let dir = TempDir::new().unwrap();
    let hidden = make_hidden_file(dir.path());

    let mut rule = make_rule(dir.path().to_str().unwrap(), false, None);
    rule.include_hidden = true;
    let events = create_events(Event::Create);

    assert!(evaluate_rule(&hidden, &events, None, &rule));
}

// 削除イベントのようにパスが既に消えている場合は属性を取得できない。
// 取りこぼしを避けるため「隠しではない」とみなして処理対象に残す。
#[test]
fn test_include_hidden_missing_path_is_not_treated_as_hidden() {
    let dir = TempDir::new().unwrap();
    let deleted = dir.path().join("already_deleted.txt");

    let rule = make_rule(dir.path().to_str().unwrap(), false, None);
    let events = create_events(Event::Create);

    assert!(evaluate_rule(&deleted, &events, None, &rule));
}

// 隠しフォルダ配下の通常ファイルは、親を遡って除外しない（設計書 §14.5）。
#[test]
fn test_include_hidden_does_not_walk_up_parents() {
    let dir = TempDir::new().unwrap();
    let hidden_dir = {
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use windows_sys::Win32::Storage::FileSystem::{
                SetFileAttributesW, FILE_ATTRIBUTE_HIDDEN,
            };
            let d = dir.path().join("hiddendir");
            std::fs::create_dir(&d).unwrap();
            let wide: Vec<u16> = d.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
            // SAFETY: wide は NUL 終端された有効な UTF-16 バッファ。
            unsafe { SetFileAttributesW(wide.as_ptr(), FILE_ATTRIBUTE_HIDDEN) };
            d
        }
        #[cfg(not(windows))]
        {
            let d = dir.path().join(".hiddendir");
            std::fs::create_dir(&d).unwrap();
            d
        }
    };
    let inner = hidden_dir.join("normal.txt");
    std::fs::write(&inner, "").unwrap();

    let rule = make_rule(dir.path().to_str().unwrap(), true, None);
    let events = create_events(Event::Create);

    assert!(evaluate_rule(&inner, &events, None, &rule), "親が隠しでもファイル自体が隠しでなければ通す");
    assert!(!evaluate_rule(&hidden_dir, &events, None, &rule), "隠しフォルダ自体は除外");
}

// dir_regex: フォルダ名正規表現包含 (#28)
#[test]
fn test_dir_regex() {
    let dir = TempDir::new().unwrap();
    let in_reports = dir.path().join("reports_2024/data.csv");
    let in_other = dir.path().join("archives/data.csv");
    for f in [&in_reports, &in_other] {
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, "").unwrap();
    }

    let mut rule = make_rule(dir.path().to_str().unwrap(), true, None);
    rule.dir_regex = Some(Regex::new(r"^reports_\d+$").unwrap());
    let events = create_events(Event::Create);

    assert!(evaluate_rule(&in_reports, &events, None, &rule));
    assert!(!evaluate_rule(&in_other, &events, None, &rule));
}
