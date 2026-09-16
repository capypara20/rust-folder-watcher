//! ログ書き込みタスク（バッチ収集 → ファイル追記 → ターミナル／ダッシュボード配信）。

use std::path::{Path, PathBuf};

use chrono::{Local, NaiveDateTime};
use colored::Colorize;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;

use super::format::{console_print, file_line};
use super::{LogEntry, LogKind};
use crate::config::{LogLevel, LogRotation};

/// 書き込み先のログファイル。ファイル名を**いつ決め直すか**を受け持つ。
///
/// 以前は書き出すたびに現在時刻で `{Date}` / `{DateTime}` を置き換えていた（#100）。
/// そのため `rotation = "never"` でも日付が変わると別ファイルになり、
/// `{DateTime}` にいたっては書き出すたびに別ファイルになっていた。
///
/// | rotation | ファイル名を決めるとき |
/// |---|---|
/// | `never` | 起動時の 1 回だけ。以降は変えない |
/// | `daily` | 起動時と、日付が変わったとき |
///
/// 時刻は引数で受け取る（テストで日付の変わり目を作れるようにするため）。
struct LogFile {
    dir: String,
    file_name: String,
    rotation: LogRotation,
    /// ファイル名を決めた日（`YYYYMMDD`）。
    date: String,
    path: PathBuf,
}

impl LogFile {
    fn new(dir: &str, file_name: &str, rotation: LogRotation, now: NaiveDateTime) -> Self {
        Self {
            dir: dir.to_string(),
            file_name: file_name.to_string(),
            rotation,
            date: date_of(now),
            path: build_log_path(dir, file_name, now),
        }
    }

    /// 日付が変わっていれば新しい日へ切り替え、`true` を返す。
    ///
    /// `never` では何もしない（ファイル名も連番も起動時のまま）。
    fn roll_if_new_day(&mut self, now: NaiveDateTime) -> bool {
        if !matches!(self.rotation, LogRotation::Daily) || date_of(now) == self.date {
            return false;
        }
        self.date = date_of(now);
        self.path = build_log_path(&self.dir, &self.file_name, now);
        true
    }
}

fn date_of(now: NaiveDateTime) -> String {
    now.format("%Y%m%d").to_string()
}

fn build_log_path(log_dir: &str, log_file_name: &str, now: NaiveDateTime) -> PathBuf {
    let file_name = log_file_name
        .replace("{Date}", &date_of(now))
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
    let mut log_file = LogFile::new(&log_dir, &log_file_name, log_rotation, Local::now().naive_local());
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
        // 日次ローテ: 日付が変わったらファイル名を決め直し、連番を #1 に戻す。
        if log_file.roll_if_new_day(now.naive_local()) {
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
            append_to_file(&log_file.path, &file_buf).await;
        }
        if shutdown {
            break;
        }
    }
}

#[cfg(test)]
#[path = "../tests/logger_writer.rs"]
mod tests;
