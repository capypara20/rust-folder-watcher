//! 実行ファイルを PATH から解決する。
//!
//! `command` の `shell` や `execute` の `program` が実際に起動できるかを、
//! 設定のバリデーション時に確かめるために使う。起動時に弾かないと、検知が
//! 起きるたびに同じ失敗を繰り返すことになる。
//!
//! **サービスとして動かす場合は、exe の探索に使われるのがサービスの PATH
//! （システム PATH）である点に注意。** ログオンユーザーの PATH ではない。
//! `run_as_logged_in_user = true` でも、子プロセスへ渡す環境ブロックの PATH と
//! exe を探すときの PATH は別物で、後者は呼び出し元（サービス）のものが使われる。
//! そのため「PATH は引き継がれているのに exe が見つからない」という状態が起きる。

use std::path::{Path, PathBuf};

/// 実行ファイルの解決結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    /// 見つかった。
    Found(PathBuf),
    /// パス指定（絶対パス、または区切り文字を含む相対パス）だが、そこに無い。
    MissingAtPath,
    /// 名前だけの指定で、PATH 上に見つからなかった。
    NotOnPath,
}

/// 実行ファイルを解決する。
///
/// 区切り文字を含む指定は「その場所を見る」だけで PATH は探さない
/// （OS のプロセス起動と同じ扱いにするため）。
pub fn resolve(program: &str) -> Resolved {
    if program.trim().is_empty() {
        return Resolved::NotOnPath;
    }

    let has_separator =
        program.contains(std::path::MAIN_SEPARATOR) || program.contains('/');
    if has_separator {
        let path = Path::new(program);
        return match first_existing(path.parent().unwrap_or(Path::new("")), path) {
            Some(found) => Resolved::Found(found),
            None => Resolved::MissingAtPath,
        };
    }

    match search_on_path(program) {
        Some(found) => Resolved::Found(found),
        None => Resolved::NotOnPath,
    }
}

/// 実際に探索した PATH のディレクトリ一覧。
pub fn search_path_entries() -> Vec<String> {
    match std::env::var_os("PATH") {
        Some(paths) => std::env::split_paths(&paths)
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.display().to_string())
            .collect(),
        None => Vec::new(),
    }
}

/// エラーメッセージ用に PATH を整形する。
///
/// PATH は 40 件を超えることも珍しくなく、全部並べるとエラー本文が埋もれて
/// 肝心の「何が見つからないのか」が読めなくなる。先頭だけ出して件数を添える。
pub fn search_path_summary() -> String {
    const SHOW: usize = 8;
    let entries = search_path_entries();
    if entries.is_empty() {
        return "(PATH が設定されていません)".to_string();
    }

    let mut out = format!("{} 件", entries.len());
    for dir in entries.iter().take(SHOW) {
        out.push_str("\n      ");
        out.push_str(dir);
    }
    if entries.len() > SHOW {
        out.push_str(&format!("\n      ... 他 {} 件", entries.len() - SHOW));
    }
    out
}

/// PATH の各ディレクトリから実行ファイルを探す。
fn search_on_path(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&paths) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        if let Some(found) = first_existing(&dir, Path::new(name)) {
            return Some(found);
        }
    }
    None
}

/// `dir` の下で `name` の候補を順に試し、最初に見つかったものを返す。
fn first_existing(dir: &Path, name: &Path) -> Option<PathBuf> {
    let file_name = name.file_name()?;
    let stem = file_name.to_str()?;
    for candidate in candidate_names(stem) {
        let full = dir.join(&candidate);
        if is_executable_file(&full) {
            return Some(full);
        }
    }
    None
}

/// 試すファイル名の候補。
///
/// Windows は拡張子を省略して書けるので、`PATHEXT` の拡張子を順に付けて試す。
/// 既に拡張子が付いていればそのままも試す。
fn candidate_names(name: &str) -> Vec<String> {
    #[cfg(windows)]
    {
        let mut names = Vec::new();
        if Path::new(name).extension().is_some() {
            names.push(name.to_string());
        }
        for ext in path_extensions() {
            names.push(format!("{name}{ext}"));
        }
        names
    }
    #[cfg(not(windows))]
    {
        vec![name.to_string()]
    }
}

/// `PATHEXT` の拡張子一覧。未設定なら Windows の既定値を使う。
#[cfg(windows)]
fn path_extensions() -> Vec<String> {
    std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
        .split(';')
        .map(|e| e.trim())
        .filter(|e| !e.is_empty())
        .map(|e| e.to_string())
        .collect()
}

/// 実行できるファイルか。Unix では実行ビットも見る。
fn is_executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        return std::fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false);
    }
    #[cfg(windows)]
    true
}

#[cfg(test)]
#[path = "tests/exe_path.rs"]
mod tests;
