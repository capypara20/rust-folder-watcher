use std::path::{Path, PathBuf};

use crate::config::Transfer;
use crate::error::AppError;
use crate::path_fmt::for_log;
use crate::placeholder::{expand_placeholders, PlaceholderContext};

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
            "{label}フォルダ '{}' が存在しません（auto_create = false のため作成しません）",
            for_log(dir)
        )));
    }
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| AppError::Action(format!("{label}フォルダ '{}' を作成できません: {e}", for_log(dir))))
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
        let mut file = std::fs::File::open(&path).map_err(|e| {
            AppError::Action(format!("内容の検証のためにファイル '{}' を開けません: {e}", for_log(&path)))
        })?;
        let mut hasher = blake3::Hasher::new();
        std::io::copy(&mut file, &mut hasher).map_err(|e| {
            AppError::Action(format!("内容の検証のためにファイル '{}' を読み込めません: {e}", for_log(&path)))
        })?;
        Ok(hasher.finalize())
    })
    .await
    .map_err(|e| AppError::Action(format!("内容の検証を完了できません: {e}")))?
}

/// 1 回分のファイルコピー試行（`tokio::fs::copy` + BLAKE3 整合性検証）。
/// verify_integrity=true のとき検証済みハッシュを返す。
/// 失敗時は宛先の削除を行わない。呼び出し側が責任を持つこと。
pub async fn try_copy_once(src: &Path, dest: &Path, verify_integrity: bool) -> Result<Option<blake3::Hash>, AppError> {
    tokio::fs::copy(src, dest)
        .await
        // 呼び出し側が「コピーに失敗しました: 元 → 先: 」と前に付けるので、ここは OS のエラーだけ。
        .map_err(|e| AppError::Action(e.to_string()))?;

    if verify_integrity {
        let src_hash = hash_file_blake3(src).await?;
        let dest_hash = hash_file_blake3(dest).await?;
        if src_hash != dest_hash {
            return Err(AppError::Action(
                "コピー後の内容が元のファイルと一致しません（BLAKE3 で比較）".to_string(),
            ));
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
            .map_err(|_| relative_path_error(src, watch_path))?;
        Ok(dest_root.join(rel))
    } else {
        let file_name = src
            .file_name()
            .ok_or_else(|| AppError::Action(format!("'{}' からファイル名を取り出せません", for_log(src))))?;
        Ok(dest_root.join(file_name))
    }
}

/// `destination` をプレースホルダー展開して `PathBuf` で返す。
pub fn expand_destination(transfer: &Transfer, ctx: &PlaceholderContext) -> PathBuf {
    PathBuf::from(expand_placeholders(&transfer.destination, ctx))
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
    .map_err(|e| AppError::Action(format!("フォルダ '{}' の中身を列挙できません: {e}", for_log(src_dir))))
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
            .ok_or_else(|| AppError::Action(format!("'{}' からフォルダ名を取り出せません", for_log(src_dir))))?;
        Ok(dest_root.join(folder_name))
    }
}

/// `base` からの相対パスを取り出す。取れない場合はアクションエラーにする。
pub(super) fn relative_to<'a>(path: &'a Path, base: &Path) -> Result<&'a Path, AppError> {
    path.strip_prefix(base).map_err(|_| relative_path_error(path, base))
}

/// `path` が `base` の中に無く、相対パスを作れないときのエラー。
/// `preserve_structure = true` で、監視フォルダの外のパスが来たときに起きる。
fn relative_path_error(path: &Path, base: &Path) -> AppError {
    AppError::Action(format!(
        "'{}' は '{}' の中に無いため、フォルダ構造を保ったまま転送できません",
        for_log(path),
        for_log(base)
    ))
}

#[cfg(test)]
#[path = "../tests/actions_common.rs"]
mod tests;
