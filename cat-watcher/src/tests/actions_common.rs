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
async fn walk_files_returns_all_files() {
    let dir = tempdir().unwrap();
    write_file(&dir.path().join("a.txt"), b"a");
    write_file(&dir.path().join("sub/b.txt"), b"b");
    write_file(&dir.path().join("sub/deep/c.txt"), b"c");
    let files = walk_files(dir.path()).await.unwrap();
    assert_eq!(files.len(), 3);
}
