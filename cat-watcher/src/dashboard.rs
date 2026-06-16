//! ブラウザでログをリアルタイム表示するための軽量 HTTP/SSE サーバ。
//!
//! `dashboard` feature 有効時のみコンパイルされる。既存のログ経路（system
//! ロガーの `writer_task`）から [`publish`] でイベントを受け取り、
//! `tokio::sync::broadcast` で各ブラウザ接続へ配信する。配信は SSE
//! （Server-Sent Events）。
//!
//! 設計方針:
//! - **取りこぼし許容**（broadcast）。UI が詰まっても監視・アクションは遅延させない。
//! - ルートは GET 2 本（`/` と `/events`）のみ。薄い自前 HTTP 実装で完結。
//! - 既定は localhost 束縛。ログにパスが出るため外部公開は呼び出し側の責任。

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::sync::Arc;

use regex::Regex;
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;

use crate::logger::{format_events, LogEntry, Logger};

/// 配信チャネルのバッファ件数。これを超えて遅延した接続は Lagged になる。
const CHANNEL_CAPACITY: usize = 512;
/// リングバッファの上限（history がこれより大きくても確保時はここで抑える）。
const BACKLOG_HARD_CAP: usize = 4096;
/// SSE キープアライブ間隔（秒）。切断検知も兼ねる。
const KEEPALIVE_SECS: u64 = 15;
/// 過去ログ検索のヒット件数 既定上限。
const SEARCH_DEFAULT_LIMIT: usize = 500;
/// 過去ログ検索のヒット件数 ハード上限（limit クエリでもこれは超えない）。
const SEARCH_MAX_LIMIT: usize = 2000;
/// 1 ファイルあたりに走査する最大行数（巨大ファイルでの暴走を防ぐ安全弁）。
const SEARCH_MAX_LINES_PER_FILE: usize = 500_000;

/// 埋め込みダッシュボード（単一 HTML）。外部ファイル不要で単一 exe のまま配布できる。
const INDEX_HTML: &str = include_str!("dashboard.html");

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
struct SearchHit {
    kind: &'static str,
    file: String,
    line: String,
}

/// 過去ログ検索のレスポンス。
#[derive(Serialize)]
struct SearchResponse {
    query: String,
    /// 上限に達して打ち切ったか。
    truncated: bool,
    /// 走査したファイル数。
    scanned_files: usize,
    hits: Vec<SearchHit>,
}

/// ブラウザへ配信する 1 イベント。JSON 化してそのまま SSE の `data:` に載せる。
#[derive(Clone, Serialize)]
pub struct DashEvent {
    /// 連番（接続側での順序確認・重複排除用）。
    pub seq: u64,
    /// 表示用タイムスタンプ（`YYYY-MM-DD HH:MM:SS`）。
    pub ts: String,
    /// 種別: `"system"` | `"detect"` | `"action"`。
    pub kind: &'static str,
    /// レベル/種類: `info|warn|error|match|block|action|ok|note`。
    pub level: &'static str,
    /// 検知ルール名（あれば）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
    /// 対象パス（あれば）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// イベント種別の文字列（`"Create,Modify"` 等。あれば）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub events: Option<String>,
    /// 本文メッセージ（あれば）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// 種別・レベルだけ決めて、他フィールドを空にした雛形を作る。
fn base(ts: &str, kind: &'static str, level: &'static str) -> DashEvent {
    DashEvent {
        seq: 0,
        ts: ts.to_string(),
        kind,
        level,
        rule: None,
        path: None,
        events: None,
        message: None,
    }
}

impl DashEvent {
    /// system ロガーが受信した [`LogEntry`] を表示用イベントへ変換する。
    ///
    /// 種別はロガーの種類ではなく **エントリの内容**で判定する（system ロガーは
    /// Match / Action 系 / Info-Warn-Error の全種を受信するため）。配信対象外
    /// （`Shutdown`）は `None`。`seq` は [`publish`] 側で採番するのでここでは 0。
    pub fn from_log_entry(entry: &LogEntry, ts: &str) -> Option<Self> {
        let ev = match entry {
            LogEntry::Match { rule_name, path, events } => DashEvent {
                rule: Some(rule_name.clone()),
                path: Some(path.clone()),
                events: Some(format_events(events)),
                ..base(ts, "detect", "match")
            },
            LogEntry::ActionBlockStart { path, events, action_count } => DashEvent {
                path: Some(path.clone()),
                events: Some(format_events(events)),
                message: Some(format!("アクション {action_count} 件を実行")),
                ..base(ts, "action", "block")
            },
            LogEntry::Action { index, total, action_type, detail } => DashEvent {
                message: Some(format!("({index}/{total}) {action_type}  {detail}")),
                ..base(ts, "action", "action")
            },
            LogEntry::ActionOk { index, total, msg } => DashEvent {
                message: Some(format!("({index}/{total}) {msg}")),
                ..base(ts, "action", "ok")
            },
            LogEntry::ActionErr { index, total, msg } => DashEvent {
                message: Some(format!("({index}/{total}) {msg}")),
                ..base(ts, "action", "error")
            },
            LogEntry::ActionWarn { index, total, msg } => DashEvent {
                message: Some(format!("({index}/{total}) {msg}")),
                ..base(ts, "action", "warn")
            },
            LogEntry::ActionNote { index, total, msg } => DashEvent {
                message: Some(format!("({index}/{total}) {msg}")),
                ..base(ts, "action", "note")
            },
            LogEntry::Info(msg) => DashEvent { message: Some(msg.clone()), ..base(ts, "system", "info") },
            LogEntry::Warn(msg) => DashEvent { message: Some(msg.clone()), ..base(ts, "system", "warn") },
            LogEntry::Error(msg) => DashEvent { message: Some(msg.clone()), ..base(ts, "system", "error") },
            LogEntry::Shutdown => return None,
        };
        Some(ev)
    }
}

/// 配信ハブ。broadcast 送信者・再生用リングバッファ・連番・検索対象を持つ。
struct Hub {
    tx: broadcast::Sender<DashEvent>,
    backlog: Mutex<VecDeque<DashEvent>>,
    history: usize,
    seq: AtomicU64,
    /// 過去ログ検索の対象（設定から集めたログ出力先）。
    sources: Vec<LogSource>,
}

static HUB: OnceLock<Hub> = OnceLock::new();

/// ダッシュボードのハブを初期化する（多重呼び出しは 2 回目以降を無視）。
/// サーバを起動する前に 1 度だけ呼ぶ。`sources` は過去ログ検索の対象。
pub fn init(history: usize, sources: Vec<LogSource>) {
    let (tx, _rx) = broadcast::channel(CHANNEL_CAPACITY);
    let _ = HUB.set(Hub {
        tx,
        backlog: Mutex::new(VecDeque::with_capacity(history.min(BACKLOG_HARD_CAP))),
        history: history.min(BACKLOG_HARD_CAP),
        seq: AtomicU64::new(0),
        sources,
    });
}

/// ハブが初期化済み（＝ダッシュボード有効）かどうか。
/// [`publish`] 前の安価なチェック用（無効時は `DashEvent` 生成すら避けられる）。
pub fn is_active() -> bool {
    HUB.get().is_some()
}

/// イベントを配信する。`seq` を採番し、リングバッファへ積み、broadcast へ送る。
/// 購読者が居なくても（送信失敗でも）問題ない。
pub fn publish(mut ev: DashEvent) {
    let Some(hub) = HUB.get() else { return };
    ev.seq = hub.seq.fetch_add(1, Ordering::Relaxed);
    if hub.history > 0 {
        if let Ok(mut backlog) = hub.backlog.lock() {
            while backlog.len() >= hub.history {
                backlog.pop_front();
            }
            backlog.push_back(ev.clone());
        }
    }
    // 受信者ゼロでも Err になるだけなので無視する。
    let _ = hub.tx.send(ev);
}

/// 設定からダッシュボードを初期化し、HTTP/SSE サーバを別タスクで起動する。
/// `dashboard` が未設定 / `enabled=false` のときは何もしない。
/// CLI 起動（`main`）と Windows サービス起動（`service`）の両方から呼ぶ共通入口。
/// tokio ランタイム上で呼ぶこと（内部で `tokio::spawn` する）。
pub fn start(global: &crate::config::GlobalConfig, rules: &[crate::config::Rule], log: Arc<Logger>) {
    let Some(dash) = &global.dashboard else { return };
    if !dash.enabled {
        return;
    }
    // 過去ログ検索の対象を設定から集める（システムログ＋有効なルール別ログ）。
    let mut sources = vec![LogSource {
        kind: "system",
        dir: global.system_log.dir.clone(),
        file_name: global.system_log.file_name.clone(),
    }];
    for rule in rules {
        if let Some(rule_log) = &rule.log {
            if let Some(detect) = &rule_log.detect {
                if detect.enabled {
                    sources.push(LogSource {
                        kind: "detect",
                        dir: detect.dir.clone(),
                        file_name: detect.file_name.clone(),
                    });
                }
            }
            if let Some(action) = &rule_log.action {
                if action.enabled {
                    sources.push(LogSource {
                        kind: "action",
                        dir: action.dir.clone(),
                        file_name: action.file_name.clone(),
                    });
                }
            }
        }
    }
    init(dash.history, sources);
    let bind = dash.bind.clone();
    tokio::spawn(async move { serve(bind, log).await });
}

/// HTTP/SSE サーバを起動する。`bind` は `"127.0.0.1:8080"` 形式。
/// 失敗してもプロセスは落とさず、system ログに記録して戻る。
pub async fn serve(bind: String, log: Arc<Logger>) {
    let listener = match TcpListener::bind(&bind).await {
        Ok(l) => l,
        Err(e) => {
            log.error(format!("ダッシュボードの起動に失敗しました（bind={bind}）: {e}"));
            return;
        }
    };
    log.info(format!("ダッシュボードを起動しました: http://{bind}/"));

    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                tokio::spawn(async move {
                    // 接続単位のエラーはそのまま閉じるだけ（クライアント都合の切断が大半）。
                    let _ = handle_connection(stream).await;
                });
            }
            Err(e) => {
                log.warn(format!("ダッシュボード接続の受理に失敗: {e}"));
            }
        }
    }
}

/// 1 接続を処理する。リクエストライン（1 行目）だけ見てルーティングする。
async fn handle_connection(stream: TcpStream) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream);

    let mut request_line = String::new();
    if reader.read_line(&mut request_line).await? == 0 {
        return Ok(());
    }
    // 残りのリクエストヘッダは空行まで読み飛ばす。
    let mut header = String::new();
    loop {
        header.clear();
        let n = reader.read_line(&mut header).await?;
        if n == 0 || header == "\r\n" || header == "\n" {
            break;
        }
    }

    // "GET /path?query HTTP/1.1" の path と query を取り出す。
    let raw_path = request_line.split_whitespace().nth(1).unwrap_or("/");
    let (path, query) = raw_path.split_once('?').unwrap_or((raw_path, ""));
    let query = query.to_string();
    let mut stream = reader.into_inner();

    match path {
        "/" | "/index.html" => write_html(&mut stream).await,
        "/events" => stream_events(stream).await,
        "/search" => handle_search(&mut stream, &query).await,
        _ => write_not_found(&mut stream).await,
    }
}

/// ダッシュボード HTML を返す。
async fn write_html(stream: &mut TcpStream) -> std::io::Result<()> {
    let body = INDEX_HTML.as_bytes();
    let header = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Cache-Control: no-cache\r\n\
         Connection: close\r\n\
         \r\n",
        body.len()
    );
    stream.write_all(header.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.flush().await
}

/// 404 を返す。
async fn write_not_found(stream: &mut TcpStream) -> std::io::Result<()> {
    let body = b"404 Not Found";
    let header = format!(
        "HTTP/1.1 404 Not Found\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n",
        body.len()
    );
    stream.write_all(header.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.flush().await
}

/// SSE ストリームを開始する。接続時にリングバッファを再生し、以降は broadcast を中継する。
async fn stream_events(mut stream: TcpStream) -> std::io::Result<()> {
    let Some(hub) = HUB.get() else {
        return write_not_found(&mut stream).await;
    };

    let header = "HTTP/1.1 200 OK\r\n\
                  Content-Type: text/event-stream; charset=utf-8\r\n\
                  Cache-Control: no-cache\r\n\
                  Connection: keep-alive\r\n\
                  \r\n";
    stream.write_all(header.as_bytes()).await?;

    // 先に購読してから backlog を取得することで、境目のイベントを取りこぼさない。
    // backlog と購読の両方に現れた分は seq で重複排除する。
    let mut rx = hub.tx.subscribe();
    let backlog: Vec<DashEvent> = hub
        .backlog
        .lock()
        .map(|b| b.iter().cloned().collect())
        .unwrap_or_default();

    let mut last_seq: Option<u64> = None;
    for ev in &backlog {
        last_seq = Some(ev.seq);
        write_sse_event(&mut stream, ev).await?;
    }

    let mut keepalive = tokio::time::interval(std::time::Duration::from_secs(KEEPALIVE_SECS));
    keepalive.tick().await; // 初回の即時 tick を消費（接続直後の無駄打ちを避ける）。

    loop {
        tokio::select! {
            recv = rx.recv() => match recv {
                Ok(ev) => {
                    // backlog 再生分との重複を seq で抑止する。
                    if last_seq.is_none_or(|s| ev.seq > s) {
                        last_seq = Some(ev.seq);
                        write_sse_event(&mut stream, &ev).await?;
                    }
                }
                // バッファ超過で取りこぼした。コメント行で通知して継続する。
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    stream.write_all(format!(": lagged {n}\n\n").as_bytes()).await?;
                }
                // 送信側が消えた（通常は起こらない）。
                Err(broadcast::error::RecvError::Closed) => break,
            },
            _ = keepalive.tick() => {
                // コメント行（`:` 始まり）は SSE では無視される。切断検知も兼ねる。
                stream.write_all(b": keepalive\n\n").await?;
                stream.flush().await?;
            }
        }
    }
    Ok(())
}

/// 1 イベントを SSE フレーム（`data: {json}\n\n`）として書き出す。
async fn write_sse_event(stream: &mut TcpStream, ev: &DashEvent) -> std::io::Result<()> {
    // serde_json は制御文字をエスケープするため、出力は必ず 1 行に収まる。
    let json = serde_json::to_string(ev).unwrap_or_else(|_| "{}".to_string());
    stream.write_all(format!("data: {json}\n\n").as_bytes()).await?;
    stream.flush().await
}

/// `GET /search?q=...&regex=0&ci=1&limit=500` を処理し、過去ログ検索の結果を
/// JSON で返す。ライブのメモリバッファを超えて、保存済みログファイルを横断検索する。
async fn handle_search(stream: &mut TcpStream, query: &str) -> std::io::Result<()> {
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

/// JSON レスポンス（200）を書き出す。
async fn write_json(stream: &mut TcpStream, body: &str) -> std::io::Result<()> {
    let bytes = body.as_bytes();
    let header = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: application/json; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Cache-Control: no-cache\r\n\
         Connection: close\r\n\
         \r\n",
        bytes.len()
    );
    stream.write_all(header.as_bytes()).await?;
    stream.write_all(bytes).await?;
    stream.flush().await
}

/// クエリ文字列から指定キーの値を取り出して percent-decode する。
fn query_param(query: &str, key: &str) -> Option<String> {
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
fn percent_decode(s: &str) -> String {
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
fn static_affixes(file_name: &str) -> (String, String) {
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
fn search_sources(
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

#[cfg(test)]
#[path = "dashboard_tests.rs"]
mod tests;
