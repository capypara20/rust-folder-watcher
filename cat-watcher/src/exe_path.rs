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
    /// ファイルは在るが実行権限が無い（Unix の実行ビット）。
    ///
    /// 「存在しない」と区別するのは、対処がまったく違うため。
    /// こちらは `chmod +x` だけで直る。Windows には実行ビットが無いので出ない。
    NotExecutable(PathBuf),
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
        return match lookup(path.parent().unwrap_or(Path::new("")), path) {
            Lookup::Executable(found) => Resolved::Found(found),
            Lookup::NotExecutable(found) => Resolved::NotExecutable(found),
            Lookup::Missing => Resolved::MissingAtPath,
        };
    }

    search_on_path(program)
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
///
/// OS と同じく、実行できないファイルは飛ばして先のディレクトリを探し続ける。
/// 最後まで実行できるものが無かったときだけ、途中で見かけた
/// 「実行権限の無い同名ファイル」を報告する（それが原因の可能性が高いため）。
fn search_on_path(name: &str) -> Resolved {
    match std::env::var_os("PATH") {
        Some(paths) => search_dirs(std::env::split_paths(&paths), name),
        None => Resolved::NotOnPath,
    }
}

/// 与えられたディレクトリを順に探す。
///
/// 環境変数の PATH から切り離してあるのはテストのため。テストは並列に走るので、
/// PATH を書き換えると無関係なテストまで巻き込む。
fn search_dirs(dirs: impl IntoIterator<Item = PathBuf>, name: &str) -> Resolved {
    let mut not_executable = None;
    for dir in dirs {
        if dir.as_os_str().is_empty() {
            continue;
        }
        match lookup(&dir, Path::new(name)) {
            Lookup::Executable(found) => return Resolved::Found(found),
            Lookup::NotExecutable(found) => {
                not_executable.get_or_insert(found);
            }
            Lookup::Missing => {}
        }
    }
    match not_executable {
        Some(found) => Resolved::NotExecutable(found),
        None => Resolved::NotOnPath,
    }
}

/// 1 つのディレクトリを見た結果。
enum Lookup {
    Executable(PathBuf),
    NotExecutable(PathBuf),
    Missing,
}

/// `dir` の下で `name` の候補を順に試す。
///
/// 実行できるものが見つかればそれを返す。無ければ、実行権限の無い
/// ファイルを見かけていればそれを、何も無ければ `Missing` を返す。
fn lookup(dir: &Path, name: &Path) -> Lookup {
    let Some(stem) = name.file_name().and_then(|f| f.to_str()) else {
        return Lookup::Missing;
    };
    let mut not_executable = None;
    for candidate in candidate_names(stem) {
        let full = dir.join(&candidate);
        if !full.is_file() {
            continue;
        }
        if has_execute_permission(&full) {
            return Lookup::Executable(full);
        }
        not_executable.get_or_insert(full);
    }
    match not_executable {
        Some(found) => Lookup::NotExecutable(found),
        None => Lookup::Missing,
    }
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

/// ファイルに実行権限があるか。呼び出し側でファイルであることは確認済み。
///
/// Unix は実行ビットを見る。Windows には実行ビットが無く、
/// 起動できるかは拡張子（PATHEXT）で決まるので常に true。
fn has_execute_permission(path: &Path) -> bool {
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(windows)]
    {
        let _ = path;
        true
    }
}

#[cfg(test)]
#[path = "tests/exe_path.rs"]
mod tests;
