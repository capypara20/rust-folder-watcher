use std::path::{Path, PathBuf};

use crate::config::ActionConfig;
use crate::error::AppError;
use crate::placeholder::{expand_placeholders, PlaceholderContext};

/// copy / move が参照する設定値をまとめたもの。
/// `ActionConfig` の Option を 1 か所で既定値へ落とし込み、
/// 下位の関数へは解決済みの値だけを渡す。
#[derive(Clone, Copy)]
pub struct TransferOptions {
    /// 宛先に同名ファイルがあるとき上書きするか。false ならスキップ。
    pub overwrite: bool,
    /// 監視ルートからの相対パス構造を宛先にも作るか。
    pub preserve_structure: bool,
    /// コピー後に BLAKE3 で内容を検証するか。
    pub verify_integrity: bool,
    /// 宛先フォルダが無いときに自動作成するか。
    /// 設定読み込み時に global.toml の既定値が焼き込まれている。
    pub auto_create: bool,
}

impl TransferOptions {
    pub fn from_action(action: &ActionConfig) -> Self {
        Self {
            overwrite: action.overwrite.unwrap_or(false),
            preserve_structure: action.preserve_structure.unwrap_or(false),
            verify_integrity: action.verify_integrity.unwrap_or(false),
            auto_create: action.auto_create.unwrap_or(true),
        }
    }
}

/// コピー/移動先のディレクトリを用意する。
///
/// `auto_create = true` なら無ければ再帰的に作る。`false` なら作らずエラーにする
/// （typo で予期しない場所へ書き込むのを防ぎたいとき用）。
/// `label` はエラーメッセージ用の表記（"コピー先" / "移動先"）。
pub async fn ensure_dest_dir(dir: &Path, auto_create: bool, label: &str) -> Result<(), AppError> {
    if dir.as_os_str().is_empty() || dir.is_dir() {
        return Ok(());
    }
    if !auto_create {
        return Err(AppError::Action(format!(
            "{label}フォルダが存在しません（auto_create = false のため自動作成しません）: {}",
            dir.display()
        )));
    }
    tokio::fs::create_dir_all(dir).await.map_err(|e| {
        AppError::Action(format!(
            "{label}フォルダの作成に失敗 ({}): {}",
            dir.display(),
            e
        ))
    })
}

/// 宛先ファイルの親ディレクトリを用意する。中身は [`ensure_dest_dir`] と同じ。
pub async fn ensure_parent_dir(dest: &Path, auto_create: bool, label: &str) -> Result<(), AppError> {
    match dest.parent() {
        Some(parent) => ensure_dest_dir(parent, auto_create, label).await,
        None => Ok(()),
    }
}

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

/// `src_dir` 配下を再帰的に列挙し、`(サブディレクトリ, ファイル)` に分けて返す
/// （`walkdir` を `spawn_blocking` で実行）。`src_dir` 自身は含まない。
///
/// ディレクトリを別で返すのは、中身が空のサブフォルダを宛先にも再現するため。
/// ファイルだけを見ていると空フォルダが宛先に作られず、move では移動元ごと
/// 消えて失われてしまう。
pub async fn walk_entries(src_dir: &Path) -> Result<(Vec<PathBuf>, Vec<PathBuf>), AppError> {
    let src = src_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let mut dirs = Vec::new();
        let mut files = Vec::new();
        for entry in walkdir::WalkDir::new(&src).min_depth(1).into_iter().flatten() {
            if entry.file_type().is_dir() {
                dirs.push(entry.path().to_path_buf());
            } else {
                files.push(entry.path().to_path_buf());
            }
        }
        (dirs, files)
    })
    .await
    .map_err(|e| AppError::Action(format!("walkdir タスク失敗: {}", e)))
}

#[cfg(test)]
#[path = "../tests/actions_common.rs"]
mod tests;
