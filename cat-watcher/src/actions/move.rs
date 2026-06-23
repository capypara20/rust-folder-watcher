use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::{ActionConfig, RetryConfig};
use crate::error::AppError;
use crate::placeholder::PlaceholderContext;

use super::common::{
    expand_action_destination, resolve_dest_path, try_copy_once, walk_files,
};
use super::ActionSink;

/// move アクションのエントリポイント。
/// 戻り値:
///   - Ok(Some(dest_path)) ... 移動完了。{Destination} 更新用
///   - Ok(None)            ... スキップ (overwrite=false で既存)
///   - Err(_)              ... 全リトライ失敗
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
        move_directory_recursive(
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
        move_one_file(src, &dest_file, overwrite, verify_integrity, retry, sink, step).await
    }
}

/// 1 ファイルの移動（rename → cross-device フォールバック → リトライ）。
async fn move_one_file(
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
            "move スキップ (overwrite=false で既存): {}",
            dest.display()
        ));
        return Ok(None);
    }

    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| {
            AppError::Action(format!(
                "移動先のディレクトリの作成に失敗 ({}): {}",
                parent.display(),
                e
            ))
        })?;
    }

    // まず rename を試みる（同一ボリューム）
    match tokio::fs::rename(src, dest).await {
        Ok(()) => {
            sink.ok(step.0, step.1, format!("移動完了 (rename): {} → {}", src.display(), dest.display()));
            return Ok(Some(dest.to_path_buf()));
        }
        Err(e) if is_cross_device(&e) => {
            sink.note(step.0, step.1, format!(
                "異ボリューム検出: copy フォールバックで移動します {} → {}",
                src.display(),
                dest.display()
            ));
        }
        Err(e) => {
            return Err(AppError::Action(format!(
                "rename 失敗 ({}): {}",
                src.display(),
                e
            )));
        }
    }

    // cross-device フォールバック: コピー + 元ファイル削除（リトライあり）
    let max_attempts = retry.count.saturating_add(1);
    let interval = Duration::from_millis(retry.interval_ms);

    for attempt in 1..=max_attempts {
        match try_copy_once(src, dest, verify_integrity).await {
            Ok(maybe_hash) => {
                tokio::fs::remove_file(src).await.map_err(|e| {
                    AppError::Action(format!(
                        "move: 元ファイルの削除に失敗 ({}): {}",
                        src.display(),
                        e
                    ))
                })?;
                let hash_suffix = maybe_hash
                    .map(|h| format!("  [BLAKE3: {h}]"))
                    .unwrap_or_default();
                sink.ok(step.0, step.1, format!(
                    "移動完了 (copy+delete): {} → {}{}",
                    src.display(),
                    dest.display(),
                    hash_suffix,
                ));
                return Ok(Some(dest.to_path_buf()));
            }
            Err(e) => {
                let _ = tokio::fs::remove_file(dest).await;
                if attempt < max_attempts {
                    sink.warn(step.0, step.1, format!(
                        "move 失敗 ({}回目/{}回): {} → {}: {} (再試行)",
                        attempt, max_attempts, src.display(), dest.display(), e
                    ));
                    tokio::time::sleep(interval).await;
                } else {
                    return Err(AppError::Action(format!(
                        "move 最終失敗 ({}回試行): {} → {}: {}",
                        max_attempts, src.display(), dest.display(), e
                    )));
                }
            }
        }
    }
    unreachable!("リトライループは必ず return で抜ける");
}

/// ディレクトリ再帰移動。配下ファイルを 1 つずつ move_one_file に流し、最後に空ディレクトリを削除する。
#[allow(clippy::too_many_arguments)]
async fn move_directory_recursive(
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
            "移動先フォルダ作成失敗 ({}): {}",
            folder_dest.display(),
            e
        ))
    })?;

    let entries = walk_files(src_dir).await?;

    for entry in &entries {
        let rel = entry
            .strip_prefix(src_dir)
            .map_err(|e| AppError::Action(format!("配下相対パス解決失敗: {}", e)))?;
        let entry_dest = folder_dest.join(rel);
        move_one_file(entry, &entry_dest, overwrite, verify_integrity, retry, sink, step).await?;
    }

    tokio::fs::remove_dir_all(src_dir).await.map_err(|e| {
        AppError::Action(format!(
            "移動元フォルダの削除に失敗 ({}): {}",
            src_dir.display(),
            e
        ))
    })?;

    Ok(Some(folder_dest))
}

/// エラーが cross-device（異ボリューム）起因かどうか判定する。
fn is_cross_device(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        ErrorKind::CrossesDevices | ErrorKind::PermissionDenied
    ) || e.raw_os_error() == Some(17)
        || e.raw_os_error() == Some(0x11)
        || windows_is_cross_device(e)
}

#[cfg(target_os = "windows")]
fn windows_is_cross_device(e: &std::io::Error) -> bool {
    e.raw_os_error() == Some(17)
}

#[cfg(not(target_os = "windows"))]
fn windows_is_cross_device(_e: &std::io::Error) -> bool {
    false
}

#[cfg(test)]
#[path = "../tests/actions_move.rs"]
mod tests;
