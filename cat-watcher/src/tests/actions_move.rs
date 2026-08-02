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

// overwrite=false でスキップしたファイルがあるのに移動元フォルダごと
// remove_dir_all していたバグの回帰テスト。スキップ分が消えてはいけない。
#[tokio::test]
async fn skipped_files_are_not_deleted_with_source_folder() {
    let watch = tempdir().unwrap();
    let dest = tempdir().unwrap();
    let src_dir = watch.path().join("mydir");
    write_file(&src_dir.join("keep.txt"), b"source");
    write_file(&src_dir.join("moved.txt"), b"moved");
    // 宛先に同名ファイルを置いておく → overwrite=false なので keep.txt はスキップされる
    write_file(&dest.path().join("mydir/keep.txt"), b"existing");

    let action = make_move_action(dest.path().to_str().unwrap(), false, false, false);
    let ctx = PlaceholderContext::new(&src_dir, watch.path(), "");

    execute(&action, &src_dir, &ctx, &make_retry(0), &make_sink(), (1, 1)).await.unwrap();

    assert!(src_dir.join("keep.txt").exists(), "スキップしたファイルを消してはいけない");
    assert_eq!(std::fs::read(src_dir.join("keep.txt")).unwrap(), b"source");
    assert_eq!(std::fs::read(dest.path().join("mydir/keep.txt")).unwrap(), b"existing");
    // 移動できたファイルはちゃんと移っている
    assert!(dest.path().join("mydir/moved.txt").exists());
    assert!(!src_dir.join("moved.txt").exists());
}

// 中身が空のサブフォルダも移動先に再現される（ファイルだけ見ていると消えてしまう）。
#[tokio::test]
async fn empty_subdirectories_are_preserved() {
    let watch = tempdir().unwrap();
    let dest = tempdir().unwrap();
    let src_dir = watch.path().join("mydir");
    write_file(&src_dir.join("a.txt"), b"a");
    std::fs::create_dir_all(src_dir.join("empty/nested")).unwrap();

    let action = make_move_action(dest.path().to_str().unwrap(), false, false, false);
    let ctx = PlaceholderContext::new(&src_dir, watch.path(), "");

    execute(&action, &src_dir, &ctx, &make_retry(0), &make_sink(), (1, 1)).await.unwrap();

    assert!(dest.path().join("mydir/empty/nested").is_dir(), "空フォルダが失われている");
    assert!(!src_dir.exists());
}

// auto_create = false のとき、宛先フォルダが無ければ作らずにエラーにする (#68)。
#[tokio::test]
async fn auto_create_false_errors_when_destination_missing() {
    let watch = tempdir().unwrap();
    let dest_root = tempdir().unwrap();
    let src = watch.path().join("a.txt");
    write_file(&src, b"hello");

    let missing = dest_root.path().join("not_created_yet");
    let mut action = make_move_action(missing.to_str().unwrap(), false, false, false);
    action.auto_create = Some(false);
    let ctx = PlaceholderContext::new(&src, watch.path(), "");

    let result = execute(&action, &src, &ctx, &make_retry(0), &make_sink(), (1, 1)).await;
    assert!(result.is_err(), "auto_create=false なら宛先を作らずエラー");
    assert!(!missing.exists(), "フォルダを作ってしまっている");
    assert!(src.exists(), "元ファイルは残るべき");
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
