use super::*;
use notify::event::CreateKind;
use std::collections::HashMap;
use std::fs;
use tempfile::tempdir;

/// 列挙結果のパス集合と、ディレクトリ判定された種別を取り出すヘルパ。
fn paths_and_dirs(events: &[Event]) -> (Vec<String>, Vec<String>) {
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    for ev in events {
        let p = ev.paths[0].file_name().unwrap().to_string_lossy().to_string();
        match ev.kind {
            EventKind::Create(CreateKind::Folder) => dirs.push(p),
            EventKind::Create(CreateKind::File) => files.push(p),
            _ => {}
        }
    }
    files.sort();
    dirs.sort();
    (files, dirs)
}

#[test]
fn recursive_scan_lists_all_files_and_dirs_excluding_root() {
    let dir = tempdir().unwrap();
    let root = dir.path().to_path_buf();
    fs::write(root.join("a.txt"), b"a").unwrap();
    fs::create_dir(root.join("sub")).unwrap();
    fs::write(root.join("sub/b.txt"), b"b").unwrap();

    let mut map = HashMap::new();
    map.insert(root.clone(), RecursiveMode::Recursive);

    let events = collect_existing_events(&map);
    let (files, dirs) = paths_and_dirs(&events);

    assert_eq!(files, vec!["a.txt".to_string(), "b.txt".to_string()]);
    assert_eq!(dirs, vec!["sub".to_string()]);
    // ルート自身は列挙されない
    assert!(events.iter().all(|e| e.paths[0] != root));
}

#[test]
fn non_recursive_scan_stays_at_top_level() {
    let dir = tempdir().unwrap();
    let root = dir.path().to_path_buf();
    fs::write(root.join("top.txt"), b"t").unwrap();
    fs::create_dir(root.join("sub")).unwrap();
    fs::write(root.join("sub/deep.txt"), b"d").unwrap();

    let mut map = HashMap::new();
    map.insert(root.clone(), RecursiveMode::NonRecursive);

    let events = collect_existing_events(&map);
    let (files, dirs) = paths_and_dirs(&events);

    // 直下の top.txt と sub のみ。配下の deep.txt は含まれない。
    assert_eq!(files, vec!["top.txt".to_string()]);
    assert_eq!(dirs, vec!["sub".to_string()]);
}

#[test]
fn empty_dir_yields_no_events() {
    let dir = tempdir().unwrap();
    let mut map = HashMap::new();
    map.insert(dir.path().to_path_buf(), RecursiveMode::Recursive);
    assert!(collect_existing_events(&map).is_empty());
}

#[test]
fn all_emitted_events_are_create() {
    let dir = tempdir().unwrap();
    let root = dir.path().to_path_buf();
    fs::write(root.join("x.dat"), b"x").unwrap();
    let mut map = HashMap::new();
    map.insert(root, RecursiveMode::Recursive);

    let events = collect_existing_events(&map);
    assert!(!events.is_empty());
    assert!(events
        .iter()
        .all(|e| matches!(e.kind, EventKind::Create(_))));
}
