use super::*;
use crate::test_support::{base_action, make_retry, make_sink, write_file};
use crate::config::ActionType;
use tempfile::tempdir;

fn make_move_action(
    dest: &str,
    overwrite: bool,
    preserve_structure: bool,
    verify_integrity: bool,
) -> ActionConfig {
    let mut a = base_action(ActionType::Move);
    a.destination = Some(dest.to_string());
    a.overwrite = Some(overwrite);
    a.preserve_structure = Some(preserve_structure);
    a.verify_integrity = Some(verify_integrity);
    a
}

#[tokio::test]
async fn moves_single_file() {
    let watch = tempdir().unwrap();
    let dest = tempdir().unwrap();
    let src = watch.path().join("a.txt");
    write_file(&src, b"hello");

    let action = make_move_action(dest.path().to_str().unwrap(), false, false, false);
    let ctx = PlaceholderContext::new(&src, watch.path(), "");

    let result = execute(&action, &src, &ctx, &make_retry(0), &make_sink(), (1, 1)).await.unwrap();
    assert_eq!(result, Some(dest.path().join("a.txt")));
    assert!(dest.path().join("a.txt").exists());
    assert!(!src.exists(), "元ファイルが残っている");
}

#[tokio::test]
async fn overwrite_flag_controls_existing_dest() {
    let watch = tempdir().unwrap();
    let dest = tempdir().unwrap();
    let src = watch.path().join("a.txt");
    write_file(&src, b"new");
    write_file(&dest.path().join("a.txt"), b"old");
    let ctx = PlaceholderContext::new(&src, watch.path(), "");

    // overwrite=false → スキップ（元ファイルを保持）
    let action = make_move_action(dest.path().to_str().unwrap(), false, false, false);
    let result = execute(&action, &src, &ctx, &make_retry(0), &make_sink(), (1, 1)).await.unwrap();
    assert_eq!(result, None);
    assert_eq!(std::fs::read(dest.path().join("a.txt")).unwrap(), b"old");
    assert!(src.exists(), "スキップ時は元ファイルを保持");

    // overwrite=true → 上書きして元ファイル削除
    let action = make_move_action(dest.path().to_str().unwrap(), true, false, false);
    execute(&action, &src, &ctx, &make_retry(0), &make_sink(), (1, 1)).await.unwrap();
    assert_eq!(std::fs::read(dest.path().join("a.txt")).unwrap(), b"new");
    assert!(!src.exists());
}

#[tokio::test]
async fn preserves_subdir_structure() {
    let watch = tempdir().unwrap();
    let dest = tempdir().unwrap();
    let src = watch.path().join("sub/deep/a.txt");
    write_file(&src, b"hello");

    let action = make_move_action(dest.path().to_str().unwrap(), false, true, false);
    let ctx = PlaceholderContext::new(&src, watch.path(), "");

    execute(&action, &src, &ctx, &make_retry(0), &make_sink(), (1, 1)).await.unwrap();
    assert!(dest.path().join("sub/deep/a.txt").exists());
    assert!(!src.exists());
}

#[tokio::test]
async fn moves_directory_recursively() {
    let watch = tempdir().unwrap();
    let dest = tempdir().unwrap();
    let src_dir = watch.path().join("mydir");
    write_file(&src_dir.join("a.txt"), b"a");
    write_file(&src_dir.join("sub/b.txt"), b"b");

    let action = make_move_action(dest.path().to_str().unwrap(), false, false, false);
    let ctx = PlaceholderContext::new(&src_dir, watch.path(), "");

    let result =
        execute(&action, &src_dir, &ctx, &make_retry(0), &make_sink(), (1, 1)).await.unwrap();
    assert_eq!(result, Some(dest.path().join("mydir")));
    assert!(dest.path().join("mydir/a.txt").exists());
    assert!(dest.path().join("mydir/sub/b.txt").exists());
    assert!(!src_dir.exists(), "移動元フォルダが削除されていない");
}

#[tokio::test]
async fn verify_integrity_passes_on_same_volume() {
    let watch = tempdir().unwrap();
    let dest = tempdir().unwrap();
    let src = watch.path().join("a.txt");
    write_file(&src, b"payload for hash");

    let action = make_move_action(dest.path().to_str().unwrap(), false, false, true);
    let ctx = PlaceholderContext::new(&src, watch.path(), "");

    let result = execute(&action, &src, &ctx, &make_retry(0), &make_sink(), (1, 1)).await.unwrap();
    assert!(result.is_some());
    assert!(!src.exists());
}

#[tokio::test]
async fn destination_with_placeholder_expands() {
    let watch = tempdir().unwrap();
    let dest_root = tempdir().unwrap();
    let src = watch.path().join("a.txt");
    write_file(&src, b"hello");

    let dest_template = format!("{}/{{Date}}/{{Time}}", dest_root.path().to_str().unwrap());
    let action = make_move_action(&dest_template, false, false, false);
    let ctx = PlaceholderContext::new(&src, watch.path(), "");

    let result = execute(&action, &src, &ctx, &make_retry(0), &make_sink(), (1, 1)).await.unwrap();
    let dest_path = result.expect("移動成功");
    assert!(dest_path.exists());
    assert_eq!(dest_path.file_name().unwrap(), "a.txt");
    assert!(!src.exists());
}

#[tokio::test]
async fn nonexistent_source_returns_error() {
    let watch = tempdir().unwrap();
    let dest = tempdir().unwrap();
    let src = watch.path().join("nonexistent.txt");

    let action = make_move_action(dest.path().to_str().unwrap(), false, false, false);
    let ctx = PlaceholderContext::new(&src, watch.path(), "");

    let result = execute(&action, &src, &ctx, &make_retry(0), &make_sink(), (1, 1)).await;
    assert!(result.is_err(), "存在しないファイルの move はエラー");
}
