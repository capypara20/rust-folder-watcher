use std::path::{Path, PathBuf};

use crate::config::ActionConfig;
use crate::error::AppError;
use crate::placeholder::{expand_placeholders, PlaceholderContext};

/// BLAKE3 ハッシュ計算（同期 IO を spawn_blocking に逃がす）。
pub async fn hash_file_blake3(path: &Path) -> Result<blake3::Hash, AppError> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<blake3::Hash, AppError> {
        let mut file = std::fs::File::open(&path)
            .map_err(|e| AppError::FileHash(format!("ファイルオープン失敗 ({}): {}", path.display(), e)))?;
        let mut hasher = blake3::Hasher::new();
        std::io::copy(&mut file, &mut hasher)
            .map_err(|e| AppError::FileHash(format!("読み込み失敗 ({}): {}", path.display(), e)))?;
        Ok(hasher.finalize())
    })
    .await
    .map_err(|e| AppError::FileHash(format!("ハッシュ計算タスク失敗: {}", e)))?
}

/// 1 回分のファイルコピー試行（`tokio::fs::copy` + BLAKE3 整合性検証）。
/// verify_integrity=true のとき検証済みハッシュを返す。
/// 失敗時は宛先の削除を行わない。呼び出し側が責任を持つこと。
pub async fn try_copy_once(src: &Path, dest: &Path, verify_integrity: bool) -> Result<Option<blake3::Hash>, AppError> {
    tokio::fs::copy(src, dest)
        .await
        .map_err(|e| AppError::Action(format!("ファイルのコピーに失敗: {}", e)))?;

    if verify_integrity {
        let src_hash = hash_file_blake3(src).await?;
        let dest_hash = hash_file_blake3(dest).await?;
        if src_hash != dest_hash {
            return Err(AppError::FileHash(format!(
                "BLAKE3 不一致: src={} dest={}",
                src.display(),
                dest.display()
            )));
        }
        Ok(Some(src_hash))
    } else {
        Ok(None)
    }
}

/// 通常ファイルの宛先パスを算出する。
/// `preserve_structure=true` のとき `watch_path` からの相対パスを `dest_root` に結合する。
pub fn resolve_dest_path(
    src: &Path,
    dest_root: &Path,
    watch_path: &Path,
    preserve_structure: bool,
) -> Result<PathBuf, AppError> {
    if preserve_structure {
        let rel = src
            .strip_prefix(watch_path)
            .map_err(|e| AppError::Action(format!("relative_path の解決に失敗: {}", e)))?;
        Ok(dest_root.join(rel))
    } else {
        let file_name = src
            .file_name()
            .ok_or_else(|| AppError::Action("ファイル名の取得に失敗".to_string()))?;
        Ok(dest_root.join(file_name))
    }
}

/// `action.destination` をプレースホルダー展開して `PathBuf` で返す。
pub fn expand_action_destination(
    action: &ActionConfig,
    ctx: &PlaceholderContext,
) -> Result<PathBuf, AppError> {
    let raw = action
        .destination
        .as_deref()
        .ok_or_else(|| AppError::Action("destination が未指定".to_string()))?;
    let expanded = expand_placeholders(raw, ctx)?;
    Ok(PathBuf::from(expanded))
}

/// `src_dir` 配下のファイルを再帰的に列挙して返す（`walkdir` を `spawn_blocking` で実行）。
pub async fn walk_files(src_dir: &Path) -> Result<Vec<PathBuf>, AppError> {
    let src = src_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        walkdir::WalkDir::new(&src)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .map(|e| e.path().to_path_buf())
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|e| AppError::Action(format!("walkdir タスク失敗: {}", e)))
}

#[cfg(test)]
#[path = "common_tests.rs"]
mod tests;
