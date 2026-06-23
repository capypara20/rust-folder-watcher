use super::*;
use crate::test_support::{base_action, make_retry, make_sink, write_file};
use crate::config::ActionType;
use tempfile::tempdir;

fn make_copy_action(
    dest: &str,
    overwrite: bool,
    preserve_structure: bool,
    verify_integrity: bool,
) -> ActionConfig {
    let mut a = base_action(ActionType::Copy);
    a.destination = Some(dest.to_string());
    a.overwrite = Some(overwrite);
    a.preserve_structure = Some(preserve_structure);
    a.verify_integrity = Some(verify_integrity);
    a
}

#[tokio::test]
async fn copies_single_file_flat() {
    let watch = tempdir().unwrap();
    let dest = tempdir().unwrap();
    let src = watch.path().join("a.txt");
    write_file(&src, b"hello");

    let action = make_copy_action(dest.path().to_str().unwrap(), false, false, false);
    let ctx = PlaceholderContext::new(&src, watch.path(), "");

    let result = execute(&action, &src, &ctx, &make_retry(0), &make_sink(), (1, 1)).await.unwrap();
    let dest_file = dest.path().join("a.txt");
    assert!(dest_file.exists());
    assert_eq!(result, Some(dest_file));
}

#[tokio::test]
async fn preserves_subdir_structure() {
    let watch = tempdir().unwrap();
    let dest = tempdir().unwrap();
    let src = watch.path().join("sub/deep/a.txt");
    write_file(&src, b"hello");

    let action = make_copy_action(dest.path().to_str().unwrap(), false, true, false);
    let ctx = PlaceholderContext::new(&src, watch.path(), "");

    execute(&action, &src, &ctx, &make_retry(0), &make_sink(), (1, 1)).await.unwrap();
    assert!(dest.path().join("sub/deep/a.txt").exists());
}

#[tokio::test]
async fn overwrite_flag_controls_existing_dest() {
    let watch = tempdir().unwrap();
    let dest = tempdir().unwrap();
    let src = watch.path().join("a.txt");
    write_file(&src, b"new");
    let dest_file = dest.path().join("a.txt");
    write_file(&dest_file, b"old");
    let ctx = PlaceholderContext::new(&src, watch.path(), "");

    // overwrite=false → スキップ（None を返し既存内容を保持）
    let action = make_copy_action(dest.path().to_str().unwrap(), false, false, false);
    let result = execute(&action, &src, &ctx, &make_retry(0), &make_sink(), (1, 1)).await.unwrap();
    assert_eq!(result, None);
    assert_eq!(std::fs::read(&dest_file).unwrap(), b"old");

    // overwrite=true → 上書き
    let action = make_copy_action(dest.path().to_str().unwrap(), true, false, false);
    execute(&action, &src, &ctx, &make_retry(0), &make_sink(), (1, 1)).await.unwrap();
    assert_eq!(std::fs::read(&dest_file).unwrap(), b"new");
}

#[tokio::test]
async fn verify_integrity_passes_for_identical_content() {
    let watch = tempdir().unwrap();
    let dest = tempdir().unwrap();
    let src = watch.path().join("a.txt");
    write_file(&src, b"some payload to hash");

    let action = make_copy_action(dest.path().to_str().unwrap(), false, false, true);
    let ctx = PlaceholderContext::new(&src, watch.path(), "");

    let result = execute(&action, &src, &ctx, &make_retry(0), &make_sink(), (1, 1)).await.unwrap();
    assert!(result.is_some());
    assert!(dest.path().join("a.txt").exists());
}

#[tokio::test]
async fn copies_directory_recursively() {
    let watch = tempdir().unwrap();
    let dest = tempdir().unwrap();
    let src_dir = watch.path().join("mydir");
    write_file(&src_dir.join("a.txt"), b"a");
    write_file(&src_dir.join("sub/b.txt"), b"b");

    let action = make_copy_action(dest.path().to_str().unwrap(), false, false, false);
    let ctx = PlaceholderContext::new(&src_dir, watch.path(), "");

    let result =
        execute(&action, &src_dir, &ctx, &make_retry(0), &make_sink(), (1, 1)).await.unwrap();
    assert_eq!(result, Some(dest.path().join("mydir")));
    assert!(dest.path().join("mydir/a.txt").exists());
    assert!(dest.path().join("mydir/sub/b.txt").exists());
}

#[tokio::test]
async fn destination_with_multiple_placeholders_creates_intermediate_dirs() {
    let watch = tempdir().unwrap();
    let dest_root = tempdir().unwrap();
    let src = watch.path().join("a.txt");
    write_file(&src, b"hello");

    let dest_template =
        format!("{}/{{Date}}/TESTDATA/{{Time}}", dest_root.path().to_str().unwrap());
    let action = make_copy_action(&dest_template, false, false, false);
    let ctx = PlaceholderContext::new(&src, watch.path(), "");

    let result = execute(&action, &src, &ctx, &make_retry(0), &make_sink(), (1, 1)).await.unwrap();
    let dest_path = result.expect("コピー成功");

    assert!(dest_path.exists(), "コピー先ファイルが存在しない: {}", dest_path.display());
    assert_eq!(dest_path.file_name().unwrap(), "a.txt");
    let parent = dest_path.parent().unwrap();
    assert_eq!(parent.file_name().unwrap().to_str().unwrap().len(), 6); // {Time}
    let grandparent = parent.parent().unwrap();
    assert_eq!(grandparent.file_name().unwrap(), "TESTDATA");
    let great_grandparent = grandparent.parent().unwrap();
    assert_eq!(great_grandparent.file_name().unwrap().to_str().unwrap().len(), 8); // {Date}
}

#[tokio::test]
async fn destination_placeholder_expands_in_dest() {
    let watch = tempdir().unwrap();
    let dest = tempdir().unwrap();
    let src = watch.path().join("a.txt");
    write_file(&src, b"hello");

    let dest_template = format!("{}/{{BaseName}}", dest.path().to_str().unwrap());
    let action = make_copy_action(&dest_template, false, false, false);
    let ctx = PlaceholderContext::new(&src, watch.path(), "");

    let result = execute(&action, &src, &ctx, &make_retry(0), &make_sink(), (1, 1)).await.unwrap();
    let expected = dest.path().join("a").join("a.txt");
    assert_eq!(result.as_deref(), Some(expected.as_path()));
    assert!(expected.exists());
}
