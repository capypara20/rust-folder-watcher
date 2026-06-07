use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::{ActionConfig, RetryConfig};
use crate::error::AppError;
use crate::placeholder::PlaceholderContext;

use super::common::{
    expand_action_destination, resolve_dest_path, try_copy_once, walk_files,
};
use super::ActionSink;

/// copy アクションのエントリポイント。
/// 戻り値:
///   - Ok(Some(dest_file_path)) ... 1 ファイル/フォルダ完了。{Destination} 更新用
///   - Ok(None)                 ... スキップ (overwrite=false で既存)
///   - Err(_)                   ... 全リトライ失敗
pub async fn execute(
    action: &ActionConfig,
    src: &Path,
    ctx: &PlaceholderContext,
    retry: &RetryConfig,
    sink: &ActionSink,
    step: (usize, usize),
) -> Result<Option<PathBuf>, AppError> {
    let dest_root = expand_action_destination(action, ctx)?;

    let overwrite = action.overwrite.unwrap_or(false);
    let preserve_structure = action.preserve_structure.unwrap_or(false);
    let verify_integrity = action.verify_integrity.unwrap_or(false);
    let watch_path = Path::new(&ctx.watch_path);

    if src.is_dir() {
        copy_directory_recursive(
            src,
            &dest_root,
            watch_path,
            overwrite,
            preserve_structure,
            verify_integrity,
            retry,
            sink,
            step,
        )
        .await
    } else {
        let dest_file = resolve_dest_path(src, &dest_root, watch_path, preserve_structure)?;
        copy_one_file(src, &dest_file, overwrite, verify_integrity, retry, sink, step).await
    }
}

/// 1 ファイルのコピー（リトライ + BLAKE3 + overwrite スキップ）。
async fn copy_one_file(
    src: &Path,
    dest: &Path,
    overwrite: bool,
    verify_integrity: bool,
    retry: &RetryConfig,
    sink: &ActionSink,
    step: (usize, usize),
) -> Result<Option<PathBuf>, AppError> {
    if dest.exists() && !overwrite {
        sink.warn(step.0, step.1, format!(
            "copy スキップ (overwrite=false で既存): {}",
            dest.display()
        ));
        return Ok(None);
    }

    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| {
            AppError::Action(format!(
                "コピー先のディレクトリの作成に失敗 ({}): {}",
                parent.display(),
                e
            ))
        })?;
    }

    let max_attempts = retry.count.saturating_add(1);
    let interval = Duration::from_millis(retry.interval_ms);

    for attempt in 1..=max_attempts {
        match try_copy_once(src, dest, verify_integrity).await {
            Ok(maybe_hash) => {
                let hash_suffix = maybe_hash
                    .map(|h| format!("  [BLAKE3: {h}]"))
                    .unwrap_or_default();
                sink.ok(step.0, step.1, format!(
                    "コピー完了: {} → {}{}",
                    src.display(), dest.display(), hash_suffix
                ));
                return Ok(Some(dest.to_path_buf()));
            }
            Err(e) => {
                let _ = tokio::fs::remove_file(dest).await;
                if attempt < max_attempts {
                    sink.warn(step.0, step.1, format!(
                        "copy 失敗 ({}回目/{}回): {} → {}: {} (再試行)",
                        attempt, max_attempts, src.display(), dest.display(), e
                    ));
                    tokio::time::sleep(interval).await;
                } else {
                    return Err(AppError::Action(format!(
                        "copy 最終失敗 ({}回試行): {} → {}: {}",
                        max_attempts, src.display(), dest.display(), e
                    )));
                }
            }
        }
    }
    unreachable!("リトライループは必ず return で抜ける");
}

/// ディレクトリ再帰コピー。配下ファイルを 1 つずつ copy_one_file に流す。
#[allow(clippy::too_many_arguments)]
async fn copy_directory_recursive(
    src_dir: &Path,
    dest_root: &Path,
    watch_path: &Path,
    overwrite: bool,
    preserve_structure: bool,
    verify_integrity: bool,
    retry: &RetryConfig,
    sink: &ActionSink,
    step: (usize, usize),
) -> Result<Option<PathBuf>, AppError> {
    let folder_dest = if preserve_structure {
        let rel = src_dir
            .strip_prefix(watch_path)
            .map_err(|e| AppError::Action(format!("relative_path の解決に失敗: {}", e)))?;
        dest_root.join(rel)
    } else {
        let folder_name = src_dir
            .file_name()
            .ok_or_else(|| AppError::Action("フォルダ名の取得に失敗".to_string()))?;
        dest_root.join(folder_name)
    };

    tokio::fs::create_dir_all(&folder_dest).await.map_err(|e| {
        AppError::Action(format!(
            "コピー先フォルダ作成失敗 ({}): {}",
            folder_dest.display(),
            e
        ))
    })?;

    let entries = walk_files(src_dir).await?;

    for entry in entries {
        let rel = entry
            .strip_prefix(src_dir)
            .map_err(|e| AppError::Action(format!("配下相対パス解決失敗: {}", e)))?;
        let entry_dest = folder_dest.join(rel);
        copy_one_file(&entry, &entry_dest, overwrite, verify_integrity, retry, sink, step).await?;
    }

    Ok(Some(folder_dest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ActionType, LogRotation};
    use crate::logger::Logger;
    use std::io::Write;
    use std::sync::Arc;
    use tempfile::tempdir;

    fn make_retry(count: u32) -> RetryConfig {
        RetryConfig { count, interval_ms: 10 }
    }

    fn make_copy_action(
        dest: &str,
        overwrite: bool,
        preserve_structure: bool,
        verify_integrity: bool,
    ) -> ActionConfig {
        ActionConfig {
            type_: ActionType::Copy,
            destination: Some(dest.to_string()),
            overwrite: Some(overwrite),
            preserve_structure: Some(preserve_structure),
            verify_integrity: Some(verify_integrity),
            working_dir: None,
            shell: None,
            command: None,
            program: None,
            args: None,
            message: None,
        }
    }

    fn write_file(path: &Path, body: &[u8]) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut f = std::fs::File::create(path).unwrap();
        f.write_all(body).unwrap();
    }

    fn make_sink() -> ActionSink {
        let dir = tempdir().unwrap();
        let (logger, _) = Logger::for_action(
            dir.path().to_str().unwrap().to_string(),
            "test.log".to_string(),
            LogRotation::Never,
        )
        .unwrap();
        std::mem::forget(dir);
        ActionSink::new(Arc::new(logger), None)
    }

    #[tokio::test]
    async fn copies_single_file_flat() {
        let watch = tempdir().unwrap();
        let dest = tempdir().unwrap();
        let src = watch.path().join("a.txt");
        write_file(&src, b"hello");

        let action = make_copy_action(dest.path().to_str().unwrap(), false, false, false);
        let retry = make_retry(0);
        let ctx = PlaceholderContext::new(&src, watch.path(), "");

        let result = execute(&action, &src, &ctx, &retry, &make_sink(), (1, 1)).await.unwrap();
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
        let retry = make_retry(0);
        let ctx = PlaceholderContext::new(&src, watch.path(), "");

        execute(&action, &src, &ctx, &retry, &make_sink(), (1, 1)).await.unwrap();
        assert!(dest.path().join("sub/deep/a.txt").exists());
    }

    #[tokio::test]
    async fn skips_when_overwrite_false_and_exists() {
        let watch = tempdir().unwrap();
        let dest = tempdir().unwrap();
        let src = watch.path().join("a.txt");
        write_file(&src, b"new");
        let dest_file = dest.path().join("a.txt");
        write_file(&dest_file, b"old");

        let action = make_copy_action(dest.path().to_str().unwrap(), false, false, false);
        let retry = make_retry(0);
        let ctx = PlaceholderContext::new(&src, watch.path(), "");

        let result = execute(&action, &src, &ctx, &retry, &make_sink(), (1, 1)).await.unwrap();
        assert_eq!(result, None);
        assert_eq!(std::fs::read(&dest_file).unwrap(), b"old");
    }

    #[tokio::test]
    async fn overwrites_when_overwrite_true() {
        let watch = tempdir().unwrap();
        let dest = tempdir().unwrap();
        let src = watch.path().join("a.txt");
        write_file(&src, b"new");
        let dest_file = dest.path().join("a.txt");
        write_file(&dest_file, b"old");

        let action = make_copy_action(dest.path().to_str().unwrap(), true, false, false);
        let retry = make_retry(0);
        let ctx = PlaceholderContext::new(&src, watch.path(), "");

        execute(&action, &src, &ctx, &retry, &make_sink(), (1, 1)).await.unwrap();
        assert_eq!(std::fs::read(&dest_file).unwrap(), b"new");
    }

    #[tokio::test]
    async fn verify_integrity_passes_for_identical_content() {
        let watch = tempdir().unwrap();
        let dest = tempdir().unwrap();
        let src = watch.path().join("a.txt");
        write_file(&src, b"some payload to hash");

        let action = make_copy_action(dest.path().to_str().unwrap(), false, false, true);
        let retry = make_retry(0);
        let ctx = PlaceholderContext::new(&src, watch.path(), "");

        let result = execute(&action, &src, &ctx, &retry, &make_sink(), (1, 1)).await.unwrap();
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
        let retry = make_retry(0);
        let ctx = PlaceholderContext::new(&src_dir, watch.path(), "");

        let result = execute(&action, &src_dir, &ctx, &retry, &make_sink(), (1, 1)).await.unwrap();
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

        let dest_template = format!(
            "{}/{{Date}}/TESTDATA/{{Time}}",
            dest_root.path().to_str().unwrap()
        );
        let action = make_copy_action(&dest_template, false, false, false);
        let retry = make_retry(0);
        let ctx = PlaceholderContext::new(&src, watch.path(), "");

        let result = execute(&action, &src, &ctx, &retry, &make_sink(), (1, 1)).await.unwrap();
        let dest_path = result.expect("コピー成功");

        assert!(dest_path.exists(), "コピー先ファイルが存在しない: {}", dest_path.display());
        assert_eq!(dest_path.file_name().unwrap(), "a.txt");
        let parent = dest_path.parent().unwrap();
        assert_eq!(parent.file_name().unwrap().to_str().unwrap().len(), 6);
        let grandparent = parent.parent().unwrap();
        assert_eq!(grandparent.file_name().unwrap(), "TESTDATA");
        let great_grandparent = grandparent.parent().unwrap();
        assert_eq!(great_grandparent.file_name().unwrap().to_str().unwrap().len(), 8);
    }

    #[tokio::test]
    async fn destination_placeholder_expands_in_dest() {
        let watch = tempdir().unwrap();
        let dest = tempdir().unwrap();
        let src = watch.path().join("a.txt");
        write_file(&src, b"hello");

        let dest_template = format!("{}/{{BaseName}}", dest.path().to_str().unwrap());
        let action = make_copy_action(&dest_template, false, false, false);
        let retry = make_retry(0);
        let ctx = PlaceholderContext::new(&src, watch.path(), "");

        let result = execute(&action, &src, &ctx, &retry, &make_sink(), (1, 1)).await.unwrap();
        let expected = dest.path().join("a").join("a.txt");
        assert_eq!(result.as_deref(), Some(expected.as_path()));
        assert!(expected.exists());
    }
}
