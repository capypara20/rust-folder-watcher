use super::*;
use crate::test_support::write_file;
use tempfile::tempdir;

#[tokio::test]
async fn hash_is_stable_and_reflects_content() {
    let dir = tempdir().unwrap();
    let a = dir.path().join("a.bin");
    let b = dir.path().join("b.bin");
    write_file(&a, b"aaa");
    write_file(&b, b"bbb");
    // 同一ファイルは同一ハッシュ、内容が異なればハッシュも異なる
    assert_eq!(
        hash_file_blake3(&a).await.unwrap(),
        hash_file_blake3(&a).await.unwrap()
    );
    assert_ne!(
        hash_file_blake3(&a).await.unwrap(),
        hash_file_blake3(&b).await.unwrap()
    );
}

#[tokio::test]
async fn try_copy_once_copies_file() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src.txt");
    let dest = dir.path().join("dest.txt");
    write_file(&src, b"hello");
    try_copy_once(&src, &dest, false).await.unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), b"hello");
}

#[tokio::test]
async fn try_copy_once_verify_integrity_ok() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src.txt");
    let dest = dir.path().join("dest.txt");
    write_file(&src, b"payload");
    try_copy_once(&src, &dest, true).await.unwrap();
    assert!(dest.exists());
}

#[test]
fn resolve_dest_path_flat_and_preserve() {
    let src = Path::new("/watch/sub/a.txt");
    let dest_root = Path::new("/dest");
    let watch = Path::new("/watch");
    assert_eq!(
        resolve_dest_path(src, dest_root, watch, false).unwrap(),
        PathBuf::from("/dest/a.txt")
    );
    assert_eq!(
        resolve_dest_path(src, dest_root, watch, true).unwrap(),
        PathBuf::from("/dest/sub/a.txt")
    );
}

#[tokio::test]
async fn walk_entries_separates_dirs_and_files() {
    let dir = tempdir().unwrap();
    write_file(&dir.path().join("a.txt"), b"a");
    write_file(&dir.path().join("sub/b.txt"), b"b");
    write_file(&dir.path().join("sub/deep/c.txt"), b"c");
    // 中身が空のフォルダも「ディレクトリ」として列挙される必要がある
    // （宛先に再現しないと move で消えてしまうため）。
    std::fs::create_dir(dir.path().join("empty")).unwrap();

    let (dirs, files) = walk_entries(dir.path()).await.unwrap();

    assert_eq!(files.len(), 3);
    let mut dir_names: Vec<String> = dirs
        .iter()
        .map(|d| d.strip_prefix(dir.path()).unwrap().to_string_lossy().replace('\\', "/"))
        .collect();
    dir_names.sort();
    assert_eq!(dir_names, vec!["empty", "sub", "sub/deep"]);
}

#[tokio::test]
async fn ensure_dest_dir_creates_only_when_auto_create() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("a/b/c");

    // auto_create=false → 作らずエラー
    assert!(ensure_dest_dir(&target, false, "コピー先").await.is_err());
    assert!(!target.exists());

    // auto_create=true → 再帰的に作る
    ensure_dest_dir(&target, true, "コピー先").await.unwrap();
    assert!(target.is_dir());

    // 既に存在する場合は auto_create=false でも通る
    ensure_dest_dir(&target, false, "コピー先").await.unwrap();
}
