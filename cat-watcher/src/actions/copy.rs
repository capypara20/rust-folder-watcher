use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::{ActionConfig, RetryConfig};
use crate::error::AppError;
use crate::placeholder::PlaceholderContext;

use super::common::{
    ensure_dest_dir, ensure_parent_dir, expand_action_destination, resolve_dest_path,
    try_copy_once, walk_entries, TransferOptions,
};
use super::ActionSink;

/// エラーメッセージ内でこのアクションを指す表記。
const LABEL: &str = "コピー先";

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
    let opts = TransferOptions::from_action(action);
    let watch_path = Path::new(&ctx.watch_path);

    if src.is_dir() {
        copy_directory_recursive(src, &dest_root, watch_path, opts, retry, sink, step).await
    } else {
        let dest_file = resolve_dest_path(src, &dest_root, watch_path, opts.preserve_structure)?;
        copy_one_file(src, &dest_file, opts, retry, sink, step).await
    }
}

/// 1 ファイルのコピー（リトライ + BLAKE3 + overwrite スキップ）。
async fn copy_one_file(
    src: &Path,
    dest: &Path,
    opts: TransferOptions,
    retry: &RetryConfig,
    sink: &ActionSink,
    step: (usize, usize),
) -> Result<Option<PathBuf>, AppError> {
    if dest.exists() && !opts.overwrite {
        sink.warn(step.0, step.1, format!(
            "copy スキップ (overwrite=false で既存): {}",
            dest.display()
        ));
        return Ok(None);
    }

    ensure_parent_dir(dest, opts.auto_create, LABEL).await?;

    let max_attempts = retry.count.saturating_add(1);
    let interval = Duration::from_millis(retry.interval_ms);

    for attempt in 1..=max_attempts {
        match try_copy_once(src, dest, opts.verify_integrity).await {
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

/// ディレクトリ再帰コピー。空のサブフォルダも宛先に作ってから、
/// 配下ファイルを 1 つずつ copy_one_file に流す。
async fn copy_directory_recursive(
    src_dir: &Path,
    dest_root: &Path,
    watch_path: &Path,
    opts: TransferOptions,
    retry: &RetryConfig,
    sink: &ActionSink,
    step: (usize, usize),
) -> Result<Option<PathBuf>, AppError> {
    let folder_dest = resolve_folder_dest(src_dir, dest_root, watch_path, opts.preserve_structure)?;
    ensure_dest_dir(&folder_dest, opts.auto_create, LABEL).await?;

    let (dirs, files) = walk_entries(src_dir).await?;

    // 中身が空のサブフォルダも宛先に残すため、先にディレクトリ構造だけ作る。
    for dir in &dirs {
        let rel = relative_to(dir, src_dir)?;
        ensure_dest_dir(&folder_dest.join(rel), opts.auto_create, LABEL).await?;
    }

    for entry in &files {
        let rel = relative_to(entry, src_dir)?;
        let entry_dest = folder_dest.join(rel);
        copy_one_file(entry, &entry_dest, opts, retry, sink, step).await?;
    }

    Ok(Some(folder_dest))
}

/// フォルダごと転送するときの宛先フォルダを決める。
/// copy / move で同じ規則なので共有する。
pub(super) fn resolve_folder_dest(
    src_dir: &Path,
    dest_root: &Path,
    watch_path: &Path,
    preserve_structure: bool,
) -> Result<PathBuf, AppError> {
    if preserve_structure {
        Ok(dest_root.join(relative_to(src_dir, watch_path)?))
    } else {
        let folder_name = src_dir
            .file_name()
            .ok_or_else(|| AppError::Action("フォルダ名の取得に失敗".to_string()))?;
        Ok(dest_root.join(folder_name))
    }
}

/// `base` からの相対パスを取り出す。取れない場合はアクションエラーにする。
pub(super) fn relative_to<'a>(path: &'a Path, base: &Path) -> Result<&'a Path, AppError> {
    path.strip_prefix(base)
        .map_err(|e| AppError::Action(format!("相対パスの解決に失敗 ({}): {}", path.display(), e)))
}

#[cfg(test)]
#[path = "../tests/actions_copy.rs"]
mod tests;
