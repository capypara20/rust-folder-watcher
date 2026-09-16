use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::{RetryConfig, Transfer};
use crate::error::AppError;
use crate::path_fmt::for_log;
use crate::placeholder::PlaceholderContext;

use super::common::{
    ensure_dest_dir, ensure_parent_dir, expand_destination, relative_to, resolve_dest_path,
    resolve_folder_dest, try_copy_once, walk_entries,
};
use super::ActionSink;

/// エラーメッセージ内でこのアクションを指す表記。
const LABEL: &str = "コピー先";

/// copy アクションのエントリポイント。
/// 戻り値:
///   - Ok(Some(dest_file_path)) ... 1 ファイル/フォルダ完了。{Destination} 更新用
///   - Ok(None)                 ... スキップ（overwrite = false で宛先に同名のファイルがある）
///   - Err(_)                   ... 全リトライ失敗
pub async fn execute(
    opts: &Transfer,
    src: &Path,
    ctx: &PlaceholderContext,
    retry: &RetryConfig,
    sink: &ActionSink,
    step: (usize, usize),
) -> Result<Option<PathBuf>, AppError> {
    let dest_root = expand_destination(opts, ctx);
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
    opts: &Transfer,
    retry: &RetryConfig,
    sink: &ActionSink,
    step: (usize, usize),
) -> Result<Option<PathBuf>, AppError> {
    if dest.exists() && !opts.overwrite {
        sink.warn(step.0, step.1, format!(
            "コピー先に同名のファイルがあるためスキップしました（overwrite = false）: {}",
            for_log(dest)
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
                    for_log(src), for_log(dest), hash_suffix
                ));
                return Ok(Some(dest.to_path_buf()));
            }
            Err(e) => {
                let _ = tokio::fs::remove_file(dest).await;
                if attempt < max_attempts {
                    sink.warn(step.0, step.1, format!(
                        "コピーに失敗しました（{attempt}/{max_attempts} 回目、再試行します）: {} → {}: {e}",
                        for_log(src), for_log(dest)
                    ));
                    tokio::time::sleep(interval).await;
                } else {
                    return Err(AppError::Action(format!(
                        "コピーに失敗しました（{max_attempts} 回試行）: {} → {}: {e}",
                        for_log(src), for_log(dest)
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
    opts: &Transfer,
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

    let mut copied = 0usize;
    for entry in &files {
        let rel = relative_to(entry, src_dir)?;
        let entry_dest = folder_dest.join(rel);
        if copy_one_file(entry, &entry_dest, opts, retry, sink, step).await?.is_some() {
            copied += 1;
        }
    }

    // フォルダ単位の完了もログに残す。中身が空だとファイル 1 件ごとの
    // 行が 1 本も出ず、何も起きなかったように見えてしまうため。
    sink.ok(step.0, step.1, format!(
        "フォルダのコピー完了: {} → {}（ファイル {}/{} 件・サブフォルダ {} 件）",
        for_log(src_dir), for_log(&folder_dest), copied, files.len(), dirs.len()
    ));

    Ok(Some(folder_dest))
}

#[cfg(test)]
#[path = "../tests/actions_copy.rs"]
mod tests;
