//! 過去ログ検索（保存済みログファイルの横断検索）と、クエリ文字列の解析。

use std::collections::HashSet;
use std::path::PathBuf;

use regex::Regex;
use serde::Serialize;
use tokio::net::TcpStream;

use super::server::{write_json, write_not_found};
use super::HUB;

/// 過去ログ検索のヒット件数 既定上限。
const SEARCH_DEFAULT_LIMIT: usize = 500;
/// 過去ログ検索のヒット件数 ハード上限（limit クエリでもこれは超えない）。
const SEARCH_MAX_LIMIT: usize = 2000;
/// 1 ファイルあたりに走査する最大行数（巨大ファイルでの暴走を防ぐ安全弁）。
const SEARCH_MAX_LINES_PER_FILE: usize = 500_000;

/// 過去ログ検索の対象（設定から集めたログ出力先）。`file_name` は
/// `{Date}` / `{DateTime}` を含み得るため、検索時は静的部分の前方/後方一致で
/// ローテーション済みファイルもまとめて拾う。
#[derive(Clone)]
pub struct LogSource {
    /// "system" | "detect" | "action"。
    pub kind: &'static str,
    pub dir: String,
    pub file_name: String,
}

/// 過去ログ検索の 1 ヒット。
#[derive(Serialize)]
pub(crate) struct SearchHit {
    pub(crate) kind: &'static str,
    pub(crate) file: String,
    pub(crate) line: String,
}

/// 過去ログ検索のレスポンス。
#[derive(Serialize)]
pub(crate) struct SearchResponse {
    pub(crate) query: String,
    /// 上限に達して打ち切ったか。
    pub(crate) truncated: bool,
    /// 走査したファイル数。
    pub(crate) scanned_files: usize,
    pub(crate) hits: Vec<SearchHit>,
}

/// `GET /search?q=...&regex=0&ci=1&limit=500` を処理し、過去ログ検索の結果を
/// JSON で返す。ライブのメモリバッファを超えて、保存済みログファイルを横断検索する。
pub(super) async fn handle_search(stream: &mut TcpStream, query: &str) -> std::io::Result<()> {
    let Some(hub) = HUB.get() else {
        return write_not_found(stream).await;
    };
    let q = query_param(query, "q").unwrap_or_default();
    let use_regex = query_param(query, "regex").as_deref() == Some("1");
    // ci=0 のときだけ大文字小文字を区別する（既定は区別なし）。
    let case_insensitive = query_param(query, "ci").as_deref() != Some("0");
    let limit = query_param(query, "limit")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(SEARCH_DEFAULT_LIMIT)
        .min(SEARCH_MAX_LIMIT);

    let body = if q.is_empty() {
        let empty = SearchResponse {
            query: String::new(),
            truncated: false,
            scanned_files: 0,
            hits: Vec::new(),
        };
        serde_json::to_string(&empty).unwrap_or_else(|_| "{}".to_string())
    } else {
        let sources = hub.sources.clone();
        // ファイル走査は同期 IO なので blocking スレッドへ逃がす。
        let result = tokio::task::spawn_blocking(move || {
            search_sources(&sources, &q, use_regex, case_insensitive, limit)
        })
        .await
        .unwrap_or_else(|_| SearchResponse {
            query: String::new(),
            truncated: false,
            scanned_files: 0,
            hits: Vec::new(),
        });
        serde_json::to_string(&result).unwrap_or_else(|_| "{}".to_string())
    };
    write_json(stream, &body).await
}

/// クエリ文字列から指定キーの値を取り出して percent-decode する。
pub(crate) fn query_param(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        if k == key {
            Some(percent_decode(v))
        } else {
            None
        }
    })
}

/// 最小限の percent-decode（`%XX` と `+` → 空白）。
pub(crate) fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => match (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                (Some(h), Some(l)) => {
                    out.push((h << 4) | l);
                    i += 3;
                }
                _ => {
                    out.push(bytes[i]);
                    i += 1;
                }
            },
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// 設定ログ出力先から、検索対象となる実ファイルを列挙する（新しい順）。
/// `file_name` の静的部分（最初の `{` より前 / 最後の `}` より後）で前後一致させ、
/// 日次ローテーション済みのファイルもまとめて拾う。
fn discover_log_files(src: &LogSource) -> Vec<PathBuf> {
    let (prefix, suffix) = static_affixes(&src.file_name);
    let mut files: Vec<PathBuf> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&src.dir) {
        for entry in rd.flatten() {
            if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.len() >= prefix.len() + suffix.len()
                && name.starts_with(&prefix)
                && name.ends_with(&suffix)
            {
                files.push(entry.path());
            }
        }
    }
    // 更新時刻の新しい順（最近のログを先に走査して上限に当てる）。
    files.sort_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
    files.reverse();
    files
}

/// `file_name` の静的な前置き・後置きを取り出す（プレースホルダーを除いた両端）。
pub(crate) fn static_affixes(file_name: &str) -> (String, String) {
    let prefix = match file_name.find('{') {
        Some(i) => file_name[..i].to_string(),
        None => file_name.to_string(),
    };
    let suffix = match file_name.rfind('}') {
        Some(i) => file_name[i + 1..].to_string(),
        None => String::new(),
    };
    (prefix, suffix)
}

/// 全ログソースを走査して、クエリに一致する行を集める（同期 IO・blocking 前提）。
pub(crate) fn search_sources(
    sources: &[LogSource],
    query: &str,
    use_regex: bool,
    case_insensitive: bool,
    limit: usize,
) -> SearchResponse {
    use std::io::BufRead;

    let mut hits: Vec<SearchHit> = Vec::new();
    let mut truncated = false;
    let mut scanned_files = 0usize;
    let mut seen: HashSet<PathBuf> = HashSet::new();

    // マッチ判定の準備。regex 指定が不正なら空結果を返す。
    let re = if use_regex {
        let pattern = if case_insensitive {
            format!("(?i){query}")
        } else {
            query.to_string()
        };
        match Regex::new(&pattern) {
            Ok(re) => Some(re),
            Err(_) => {
                return SearchResponse {
                    query: query.to_string(),
                    truncated: false,
                    scanned_files: 0,
                    hits,
                }
            }
        }
    } else {
        None
    };
    let needle = if case_insensitive {
        query.to_lowercase()
    } else {
        query.to_string()
    };

    'outer: for src in sources {
        for path in discover_log_files(src) {
            if !seen.insert(path.clone()) {
                continue; // 複数ソースが同じファイルを指す場合の二重走査を防ぐ。
            }
            scanned_files += 1;
            let Ok(file) = std::fs::File::open(&path) else { continue };
            let reader = std::io::BufReader::new(file);
            let file_name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();

            // ファイル内は新しい行を先に出したいので、一旦ためてから反転する。
            let mut file_hits: Vec<SearchHit> = Vec::new();
            for (n, line) in reader.lines().enumerate() {
                if n >= SEARCH_MAX_LINES_PER_FILE {
                    break;
                }
                let Ok(line) = line else { continue };
                let is_match = match &re {
                    Some(re) => re.is_match(&line),
                    None if case_insensitive => line.to_lowercase().contains(&needle),
                    None => line.contains(&needle),
                };
                if is_match {
                    file_hits.push(SearchHit {
                        kind: src.kind,
                        file: file_name.clone(),
                        line: line.trim_end().to_string(),
                    });
                }
            }
            for hit in file_hits.into_iter().rev() {
                if hits.len() >= limit {
                    truncated = true;
                    break 'outer;
                }
                hits.push(hit);
            }
        }
    }

    SearchResponse {
        query: query.to_string(),
        truncated,
        scanned_files,
        hits,
    }
}
