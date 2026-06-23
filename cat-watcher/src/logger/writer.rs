//! ログ書き込みタスク（バッチ収集 → ファイル追記 → ターミナル／ダッシュボード配信）。

use std::path::{Path, PathBuf};

use chrono::Local;
use colored::Colorize;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;

use super::format::{console_print, file_line};
use super::{LogEntry, LogKind};
use crate::config::{LogLevel, LogRotation};

fn build_log_path(log_dir: &str, log_file_name: &str) -> PathBuf {
    let now = Local::now();
    let file_name = log_file_name
        .replace("{Date}", &now.format("%Y%m%d").to_string())
        .replace("{DateTime}", &now.format("%Y%m%d_%H%M%S").to_string());
    PathBuf::from(log_dir).join(file_name)
}

/// ログファイルを **書き込み時だけ** open → 追記 → close する（Issue #46）。
/// バッチ単位でまとめて呼ぶことで、ハンドルを握りっぱなしにせず、かつ
/// open/close のシステムコール回数を抑える。`content` が空なら何もしない。
async fn append_to_file(path: &Path, content: &str) {
    if content.is_empty() {
        return;
    }
    match OpenOptions::new().create(true).append(true).open(path).await {
        Ok(mut f) => {
            if let Err(e) = f.write_all(content.as_bytes()).await {
                let ts = Local::now().format("%Y-%m-%d %H:%M:%S");
                eprintln!("{}", format!("[{ts}] [ERROR] ログ書き込み失敗: {e}").red().bold());
            }
            // f はここで drop され、ファイルが閉じられる（明示 flush で確実に書き出す）
            let _ = f.flush().await;
        }
        Err(e) => {
            let ts = Local::now().format("%Y-%m-%d %H:%M:%S");
            eprintln!(
                "{}",
                format!("[{ts}] [ERROR] ログファイルオープン失敗 ({}): {}", path.display(), e)
                    .red()
                    .bold()
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn writer_task(
    mut rx: mpsc::UnboundedReceiver<LogEntry>,
    log_dir: String,
    log_file_name: String,
    log_rotation: LogRotation,
    kind: LogKind,
    level: LogLevel,
    console: bool,
    enabled: bool,
) {
    let mut current_date = Local::now().format("%Y%m%d").to_string();
    // アクションログのブロック連番（日次ローテで #1 にリセット）。
    let mut block_seq: usize = 0;

    while let Some(first) = rx.recv().await {
        // バッチ収集（Issue #46）: まず 1 件を待ち、キューにたまっている分を
        // try_recv で一気にすくい取る。こうしてバッチ単位で 1 回だけ
        // open → write → close することで、ハンドルを握りっぱなしにせず、
        // かつ open/close のシステムコール回数も抑える。
        let mut batch = vec![first];
        while let Ok(e) = rx.try_recv() {
            batch.push(e);
        }

        let now = Local::now();
        let today = now.format("%Y%m%d").to_string();
        // 日次ローテ: 日付が変わったら block_seq をリセットする。出力先パスは
        // build_log_path が {Date} を差し替えるため自動的に当日のファイルへ向く。
        if matches!(log_rotation, LogRotation::Daily) && today != current_date {
            current_date = today;
            block_seq = 0;
        }
        let ts = now.format("%Y-%m-%d %H:%M:%S").to_string();

        // このバッチで書くファイル行をためるバッファ（最後に 1 回だけ書き出す）。
        let mut file_buf = String::new();
        let mut shutdown = false;

        for entry in &batch {
            if matches!(entry, LogEntry::Shutdown) {
                shutdown = true;
                break;
            }
            if enabled {
                if let Some(line) = file_line(entry, kind, &ts, &level, &mut block_seq) {
                    file_buf.push_str(&line);
                }
            }
            // ダッシュボードへのティー（分岐）。System ロガーは Match / Action /
            // Info-Warn-Error の全種を受信するため、ここ 1 点で重複なく全イベントを
            // 拾える。配信は取りこぼし許容で、UI が詰まっても監視は遅延しない。
            #[cfg(feature = "dashboard")]
            if matches!(kind, LogKind::System) && crate::dashboard::is_active() {
                if let Some(ev) = crate::dashboard::DashEvent::from_log_entry(entry, &ts) {
                    crate::dashboard::publish(ev);
                }
            }
            // ターミナル出力は従来どおり 1 件ずつ即時に行う（System ロガーのみ）。
            if console {
                console_print(entry, &ts, &level);
            }
        }

        // バッチをまとめて 1 回の open → write → close で書き出す。
        if enabled {
            append_to_file(&build_log_path(&log_dir, &log_file_name), &file_buf).await;
        }
        if shutdown {
            break;
        }
    }
}
