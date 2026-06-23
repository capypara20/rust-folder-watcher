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
#[path = "../tests/actions_copy.rs"]
mod tests;
