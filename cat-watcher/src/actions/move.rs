use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::{ActionConfig, RetryConfig};
use crate::error::AppError;
use crate::placeholder::PlaceholderContext;

use super::common::{
    ensure_dest_dir, ensure_parent_dir, expand_action_destination, resolve_dest_path,
    try_copy_once, walk_entries, TransferOptions,
};
use super::copy::{relative_to, resolve_folder_dest};
use super::ActionSink;

/// エラーメッセージ内でこのアクションを指す表記。
const LABEL: &str = "移動先";

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
    let opts = TransferOptions::from_action(action);
    let watch_path = Path::new(&ctx.watch_path);

    if src.is_dir() {
        move_directory_recursive(src, &dest_root, watch_path, opts, retry, sink, step).await
    } else {
        let dest_file = resolve_dest_path(src, &dest_root, watch_path, opts.preserve_structure)?;
        move_one_file(src, &dest_file, opts, retry, sink, step).await
    }
}

/// 1 ファイルの移動（rename → cross-device フォールバック → リトライ）。
async fn move_one_file(
    src: &Path,
    dest: &Path,
    opts: TransferOptions,
    retry: &RetryConfig,
    sink: &ActionSink,
    step: (usize, usize),
) -> Result<Option<PathBuf>, AppError> {
    if dest.exists() && !opts.overwrite {
        sink.warn(step.0, step.1, format!(
            "move スキップ (overwrite=false で既存): {}",
            dest.display()
        ));
        return Ok(None);
    }

    ensure_parent_dir(dest, opts.auto_create, LABEL).await?;

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
        match try_copy_once(src, dest, opts.verify_integrity).await {
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

/// ディレクトリ再帰移動。
///
/// 空のサブフォルダも宛先に再現してから、配下ファイルを 1 つずつ移動する。
/// 最後に移動元を削除するが、`overwrite = false` で 1 件でもスキップした場合は
/// **削除しない**（移動できていないファイルを消してしまわないため）。
async fn move_directory_recursive(
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

    // 中身が空のサブフォルダも移動先に残すため、先にディレクトリ構造だけ作る。
    for dir in &dirs {
        let rel = relative_to(dir, src_dir)?;
        ensure_dest_dir(&folder_dest.join(rel), opts.auto_create, LABEL).await?;
    }

    let mut skipped = 0usize;
    for entry in &files {
        let rel = relative_to(entry, src_dir)?;
        let entry_dest = folder_dest.join(rel);
        if move_one_file(entry, &entry_dest, opts, retry, sink, step)
            .await?
            .is_none()
        {
            skipped += 1;
        }
    }

    if skipped > 0 {
        sink.warn(step.0, step.1, format!(
            "移動元フォルダは削除しませんでした（overwrite=false でスキップしたファイルが {} 件残っています）: {}",
            skipped,
            src_dir.display()
        ));
        return Ok(Some(folder_dest));
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
///
/// - Windows: 異なるボリューム間の `MoveFileW` は ERROR_NOT_SAME_DEVICE (17)。
/// - Unix: EXDEV。どちらも Rust が `ErrorKind::CrossesDevices` に正規化する
///   （生の errno も見るのは古い Rust / 想定外の経路への保険）。
/// - `PermissionDenied`: ネットワーク共有をまたぐ移動で ERROR_ACCESS_DENIED が
///   返ることがあるため、copy フォールバックの対象に含める。本当に権限が無ければ
///   フォールバック先の copy も失敗するので、取りこぼしより誤判定を選んでいる。
fn is_cross_device(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        ErrorKind::CrossesDevices | ErrorKind::PermissionDenied
    ) || e.raw_os_error() == Some(17)
}

#[cfg(test)]
#[path = "../tests/actions_move.rs"]
mod tests;
