//! 軽量 HTTP/SSE サーバ本体（接続受理・ルーティング・レスポンス書き出し）。

use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;

use super::event::DashEvent;
use super::search::handle_search;
use super::HUB;
use crate::logger::Logger;

/// SSE キープアライブ間隔（秒）。切断検知も兼ねる。
const KEEPALIVE_SECS: u64 = 15;

/// 埋め込みダッシュボード（単一 HTML）。外部ファイル不要で単一 exe のまま配布できる。
const INDEX_HTML: &str = include_str!("dashboard.html");

/// HTTP/SSE サーバを起動する。`bind` は `"127.0.0.1:8080"` 形式。
/// 失敗してもプロセスは落とさず、system ログに記録して戻る。
pub(super) async fn serve(bind: String, log: Arc<Logger>) {
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
pub(super) async fn write_not_found(stream: &mut TcpStream) -> std::io::Result<()> {
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

/// JSON レスポンス（200）を書き出す。
pub(super) async fn write_json(stream: &mut TcpStream, body: &str) -> std::io::Result<()> {
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
