use std::collections::HashSet;
use std::path::PathBuf;

use chrono::Local;
use colored::Colorize;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;

use crate::config::{LogLevel, LogRotation, SystemLogConfig};
use crate::error::AppError;
use unicode_width::UnicodeWidthStr;

const SEPARATOR: &str = "──────────────────────────────────────────────────────────────";

/// システムログ level カラムの表示列幅（"ERROR" = 5 列）。
const SYS_LEVEL_WIDTH: usize = 5;
/// 検知ログ events カラムの表示列幅。
const DETECT_EVENTS_WIDTH: usize = 20;
/// アクションログ ステップカラムの表示列幅（"10. copy" 程度を想定）。
const ACTION_STEP_WIDTH: usize = 9;

/// ログの種類。`writer_task` がこの種別で整形を分岐する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogKind {
    /// システムログ（全体1本・ライフサイクル＋システムエラー）。
    System,
    /// 検知ログ（ルール別・検知イベントのみ）。
    Detect,
    /// アクションログ（ルール別・ブロック構造）。
    Action,
}

/// 表示列幅 (East Asian Width, CJK モード) で左寄せパディングする。
/// Rust 標準の `format!("{:<width$}")` は char 数で揃えるため、
/// '│' '═' などの罫線記号で表示時に列幅がズレる。
/// 本プロジェクトは日本語ロケール (CJK) を主用途とするため、
/// East Asian Ambiguous 文字を 2 列幅として扱う `width_cjk()` を使う。
fn pad_left_display(s: &str, total_cols: usize) -> String {
    let w = UnicodeWidthStr::width_cjk(s);
    if w >= total_cols {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + (total_cols - w));
    out.push_str(s);
    for _ in 0..(total_cols - w) {
        out.push(' ');
    }
    out
}

#[derive(Debug)]
// `total` は将来 "N/total" 表示に使う余地を残して保持する。`Warn` は
// システム警告用の API として残す（現状の呼び出し元はまだ無い）。
#[allow(dead_code)]
pub enum LogEntry {
    /// 検知（1ファイル/フォルダのマッチ）。
    Match {
        rule_name: String,
        path: String,
        events: HashSet<crate::config::Event>,
    },
    /// アクションログのブロック開始セパレータ。
    ActionBlockStart {
        path: String,
        events: HashSet<crate::config::Event>,
        action_count: usize,
    },
    /// アクションチェーン ステップ開始。
    Action {
        index: usize,
        total: usize,
        action_type: String,
        detail: String,
    },
    /// アクションチェーン ステップ完了（成功）。
    ActionOk {
        index: usize,
        total: usize,
        msg: String,
    },
    /// アクションチェーン ステップ失敗。
    ActionErr {
        index: usize,
        total: usize,
        msg: String,
    },
    /// アクションチェーン ステップ警告（スキップ・リトライ等）。
    ActionWarn {
        index: usize,
        total: usize,
        msg: String,
    },
    /// アクションチェーン 補足情報（別ボリュームへの copy+delete 等）。
    ActionNote {
        index: usize,
        total: usize,
        msg: String,
    },
    /// 通常情報。
    Info(String),
    /// 警告。
    Warn(String),
    /// エラー。
    Error(String),
    /// チャネルをクローズしてロガーを終了させる。
    Shutdown,
}

pub struct Logger {
    tx: mpsc::UnboundedSender<LogEntry>,
}

impl Logger {
    /// システムログ用ロガー。`allow_console` が false（サービスモード等）なら
    /// 設定の console によらずコンソール出力を無効化する。
    pub fn new_system(
        cfg: &SystemLogConfig,
        allow_console: bool,
    ) -> Result<(Self, tokio::task::JoinHandle<()>), AppError> {
        #[cfg(windows)]
        colored::control::set_virtual_terminal(true).ok();
        colored::control::set_override(true);

        let console = cfg.console && allow_console;
        let (tx, rx) = mpsc::unbounded_channel();
        let handle = tokio::spawn(writer_task(
            rx,
            cfg.dir.clone(),
            cfg.file_name.clone(),
            cfg.rotation.clone(),
            LogKind::System,
            cfg.level.clone(),
            console,
            cfg.enabled,
        ));
        Ok((Self { tx }, handle))
    }

    /// 検知ログ用ロガー（ファイル出力のみ・level フィルタなし）。
    pub fn for_detect(
        dir: String,
        file_name: String,
        rotation: LogRotation,
    ) -> Result<(Self, tokio::task::JoinHandle<()>), AppError> {
        let (tx, rx) = mpsc::unbounded_channel();
        let handle = tokio::spawn(writer_task(
            rx,
            dir,
            file_name,
            rotation,
            LogKind::Detect,
            LogLevel::Info,
            false,
            true,
        ));
        Ok((Self { tx }, handle))
    }

    /// アクションログ用ロガー（ファイル出力のみ・ブロック構造）。
    pub fn for_action(
        dir: String,
        file_name: String,
        rotation: LogRotation,
    ) -> Result<(Self, tokio::task::JoinHandle<()>), AppError> {
        let (tx, rx) = mpsc::unbounded_channel();
        let handle = tokio::spawn(writer_task(
            rx,
            dir,
            file_name,
            rotation,
            LogKind::Action,
            LogLevel::Info,
            false,
            true,
        ));
        Ok((Self { tx }, handle))
    }

    pub fn log_match(
        &self,
        rule_name: impl Into<String>,
        path: impl Into<String>,
        events: HashSet<crate::config::Event>,
    ) {
        let _ = self.tx.send(LogEntry::Match {
            rule_name: rule_name.into(),
            path: path.into(),
            events,
        });
    }

    pub fn log_block_start(
        &self,
        path: impl Into<String>,
        events: HashSet<crate::config::Event>,
        action_count: usize,
    ) {
        let _ = self.tx.send(LogEntry::ActionBlockStart {
            path: path.into(),
            events,
            action_count,
        });
    }

    pub fn log_action(
        &self,
        index: usize,
        total: usize,
        action_type: impl Into<String>,
        detail: impl Into<String>,
    ) {
        let _ = self.tx.send(LogEntry::Action {
            index,
            total,
            action_type: action_type.into(),
            detail: detail.into(),
        });
    }

    pub fn log_action_ok(&self, index: usize, total: usize, msg: impl Into<String>) {
        let _ = self.tx.send(LogEntry::ActionOk {
            index,
            total,
            msg: msg.into(),
        });
    }

    pub fn log_action_err(&self, index: usize, total: usize, msg: impl Into<String>) {
        let _ = self.tx.send(LogEntry::ActionErr {
            index,
            total,
            msg: msg.into(),
        });
    }

    pub fn log_action_warn(&self, index: usize, total: usize, msg: impl Into<String>) {
        let _ = self.tx.send(LogEntry::ActionWarn {
            index,
            total,
            msg: msg.into(),
        });
    }

    pub fn log_action_note(&self, index: usize, total: usize, msg: impl Into<String>) {
        let _ = self.tx.send(LogEntry::ActionNote {
            index,
            total,
            msg: msg.into(),
        });
    }

    pub fn info(&self, msg: impl Into<String>) {
        let _ = self.tx.send(LogEntry::Info(msg.into()));
    }

    #[allow(dead_code)]
    pub fn warn(&self, msg: impl Into<String>) {
        let _ = self.tx.send(LogEntry::Warn(msg.into()));
    }

    pub fn error(&self, msg: impl Into<String>) {
        let _ = self.tx.send(LogEntry::Error(msg.into()));
    }

    pub fn shutdown(&self) {
        let _ = self.tx.send(LogEntry::Shutdown);
    }
}

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
async fn append_to_file(path: &PathBuf, content: &str) {
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

/// アクション種別を短縮名に変換する（copy/move/cmd/exec/log）。
fn action_type_short(action_type: &str) -> &str {
    match action_type {
        "copy" => "copy",
        "move" => "move",
        "command" => "cmd",
        "execute" => "exec",
        "log" => "log",
        other => {
            if other.len() <= 4 { other } else { &other[..4] }
        }
    }
}

/// システムログに書く level ラベル。書き込み対象外（検知/アクション系）は None。
fn sys_level_label(entry: &LogEntry) -> Option<&'static str> {
    match entry {
        LogEntry::Info(_) => Some("INFO"),
        LogEntry::Warn(_) => Some("WARN"),
        LogEntry::Error(_) => Some("ERROR"),
        _ => None,
    }
}

/// システムログ content カラム。
fn sys_content(entry: &LogEntry) -> String {
    match entry {
        LogEntry::Info(msg) | LogEntry::Warn(msg) | LogEntry::Error(msg) => msg.clone(),
        _ => String::new(),
    }
}

/// アクションログのステップ番号・ラベル・本文を取り出す。Action系以外は None。
fn action_step_parts(entry: &LogEntry) -> Option<(usize, String, String)> {
    match entry {
        LogEntry::Action { index, action_type, detail, .. } => {
            Some((*index, action_type_short(action_type).to_string(), detail.replace('\n', r"\n")))
        }
        LogEntry::ActionOk { index, msg, .. } => Some((*index, "OK".to_string(), msg.clone())),
        LogEntry::ActionErr { index, msg, .. } => Some((*index, "ERR".to_string(), msg.clone())),
        LogEntry::ActionWarn { index, msg, .. } => Some((*index, "WARN".to_string(), msg.clone())),
        LogEntry::ActionNote { index, msg, .. } => Some((*index, "--".to_string(), msg.clone())),
        _ => None,
    }
}

/// アクションログのステップカラム文字列を生成する。
/// `index == 0` は番号なし（ルール階層のチェーン全体エラー等）で描画する。
fn render_step_col(index: usize, label: &str) -> String {
    let s = if index == 0 {
        label.to_string()
    } else {
        format!("{index}. {label}")
    };
    pad_left_display(&s, ACTION_STEP_WIDTH)
}

/// アクションログのブロック開始セパレータ行を生成する。
fn render_block_start(
    seq: usize,
    ts: &str,
    path: &str,
    events: &HashSet<crate::config::Event>,
    action_count: usize,
) -> String {
    format!(
        "═══ #{seq}  {ts}  {path}  ({})  actions={action_count} ═══\n",
        format_events(events)
    )
}

/// 検知ログの1行を生成する。
fn render_detect_line(ts: &str, events: &HashSet<crate::config::Event>, path: &str) -> String {
    format!(
        "{} │ {} │ {}\n",
        ts,
        pad_left_display(&format_events(events), DETECT_EVENTS_WIDTH),
        path
    )
}

#[allow(clippy::too_many_arguments)]
async fn writer_task(
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
        loop {
            match rx.try_recv() {
                Ok(e) => batch.push(e),
                Err(_) => break,
            }
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

/// 1 エントリのファイル出力行を生成する（kind 別フォーマット）。
/// 書き込み対象外なら None。`block_seq` はアクションブロック開始で +1 する。
fn file_line(
    entry: &LogEntry,
    kind: LogKind,
    ts: &str,
    level: &LogLevel,
    block_seq: &mut usize,
) -> Option<String> {
    match kind {
        LogKind::System => {
            let lbl = sys_level_label(entry)?;
            if !level_enabled_for_entry(level, entry) {
                return None;
            }
            Some(format!(
                "{} │ {} │ {}\n",
                ts,
                pad_left_display(lbl, SYS_LEVEL_WIDTH),
                sys_content(entry)
            ))
        }
        LogKind::Detect => match entry {
            LogEntry::Match { events, path, .. } => Some(render_detect_line(ts, events, path)),
            _ => None,
        },
        LogKind::Action => match entry {
            LogEntry::ActionBlockStart { path, events, action_count } => {
                *block_seq += 1;
                Some(render_block_start(*block_seq, ts, path, events, *action_count))
            }
            _ => action_step_parts(entry).map(|(idx, label, content)| {
                format!("{} │ {} │ {}\n", ts, render_step_col(idx, &label), content)
            }),
        },
    }
}

/// 1 エントリをターミナルへカラー付きで出力する（従来フォーマットを維持）。
fn console_print(entry: &LogEntry, ts: &str, level: &LogLevel) {
    match entry {
        LogEntry::Match { rule_name, path, events } => {
            if !level_enabled(level, &LogLevel::Info) { return; }
            let event_str = format_events(events);
            println!(
                "{}\n{} {}",
                SEPARATOR.bright_green().dimmed(),
                format!("[{ts}] [MATCH]").bright_green().bold(),
                format!("  ルール={rule_name} | パス={path} | {event_str}")
            );
        }

        LogEntry::Action { index, total, action_type, detail } => {
            if !level_enabled(level, &LogLevel::Info) { return; }
            println!(
                "{} {}",
                format!("[{ts}] [ACTION]").blue().bold(),
                format!("  ({index}/{total}) {action_type}  {detail}")
            );
        }

        LogEntry::ActionOk { msg, .. } => {
            if !level_enabled(level, &LogLevel::Info) { return; }
            println!("{} {}", format!("[{ts}] [OK]    ").green().bold(), msg);
        }

        LogEntry::ActionNote { msg, .. } => {
            if !level_enabled(level, &LogLevel::Info) { return; }
            println!("{} {}", format!("[{ts}] [INFO]").cyan(), msg);
        }

        LogEntry::ActionWarn { msg, .. } => {
            if !level_enabled(level, &LogLevel::Warn) { return; }
            println!("{} {}", format!("[{ts}] [WARN]").yellow().bold(), msg);
        }

        LogEntry::ActionErr { msg, .. } => {
            if !level_enabled(level, &LogLevel::Error) { return; }
            eprintln!("{} {}", format!("[{ts}] [ERROR]").red().bold(), msg);
        }

        LogEntry::Info(msg) => {
            if !level_enabled(level, &LogLevel::Info) { return; }
            println!("{} {}", format!("[{ts}] [INFO]").cyan(), msg);
        }

        LogEntry::Warn(msg) => {
            if !level_enabled(level, &LogLevel::Warn) { return; }
            println!("{} {}", format!("[{ts}] [WARN]").yellow().bold(), msg);
        }

        LogEntry::Error(msg) => {
            if !level_enabled(level, &LogLevel::Error) { return; }
            eprintln!("{} {}", format!("[{ts}] [ERROR]").red().bold(), msg);
        }

        // ブロック開始セパレータはファイル専用（コンソールには出さない）
        LogEntry::ActionBlockStart { .. } => {}
        LogEntry::Shutdown => {}
    }
}

fn level_enabled_for_entry(current: &LogLevel, entry: &LogEntry) -> bool {
    let required = match entry {
        LogEntry::Warn(_) => &LogLevel::Warn,
        LogEntry::Error(_) => &LogLevel::Error,
        _ => &LogLevel::Info,
    };
    level_enabled(current, required)
}

fn level_enabled(current: &LogLevel, required: &LogLevel) -> bool {
    level_to_u8(current) <= level_to_u8(required)
}

fn level_to_u8(level: &LogLevel) -> u8 {
    match level {
        LogLevel::Trace => 0,
        LogLevel::Debug => 1,
        LogLevel::Info => 2,
        LogLevel::Warn => 3,
        LogLevel::Error => 4,
    }
}

pub(crate) fn format_events(events: &HashSet<crate::config::Event>) -> String {
    let mut names: Vec<&str> = events
        .iter()
        .map(|e| match e {
            crate::config::Event::Create => "Create",
            crate::config::Event::Modify => "Modify",
            crate::config::Event::Delete => "Delete",
            crate::config::Event::Rename => "Rename",
        })
        .collect();
    names.sort();
    names.join(",")
}

#[cfg(test)]
#[path = "logger_tests.rs"]
mod tests;
