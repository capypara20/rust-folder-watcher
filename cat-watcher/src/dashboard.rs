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

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::sync::Arc;

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

/// 埋め込みダッシュボード（単一 HTML）。外部ファイル不要で単一 exe のまま配布できる。
const INDEX_HTML: &str = include_str!("dashboard.html");

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

/// 配信ハブ。broadcast 送信者・再生用リングバッファ・連番を持つ。
struct Hub {
    tx: broadcast::Sender<DashEvent>,
    backlog: Mutex<VecDeque<DashEvent>>,
    history: usize,
    seq: AtomicU64,
}

static HUB: OnceLock<Hub> = OnceLock::new();

/// ダッシュボードのハブを初期化する（多重呼び出しは 2 回目以降を無視）。
/// サーバを起動する前に 1 度だけ呼ぶ。
pub fn init(history: usize) {
    let (tx, _rx) = broadcast::channel(CHANNEL_CAPACITY);
    let _ = HUB.set(Hub {
        tx,
        backlog: Mutex::new(VecDeque::with_capacity(history.min(BACKLOG_HARD_CAP))),
        history: history.min(BACKLOG_HARD_CAP),
        seq: AtomicU64::new(0),
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

    // "GET /path HTTP/1.1" の path 部分を取り出す。
    let path = request_line.split_whitespace().nth(1).unwrap_or("/");
    let mut stream = reader.into_inner();

    match path {
        "/" | "/index.html" => write_html(&mut stream).await,
        "/events" => stream_events(stream).await,
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
