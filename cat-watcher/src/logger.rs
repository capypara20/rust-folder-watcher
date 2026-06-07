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

async fn open_log_file(path: &PathBuf) -> Option<tokio::fs::File> {
    match OpenOptions::new().create(true).append(true).open(path).await {
        Ok(f) => Some(f),
        Err(e) => {
            let ts = Local::now().format("%Y-%m-%d %H:%M:%S");
            eprintln!(
                "{}",
                format!("[{ts}] [ERROR] ログファイルオープン失敗 ({}): {}", path.display(), e)
                    .red()
                    .bold()
            );
            None
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
    let log_path = build_log_path(&log_dir, &log_file_name);
    let mut file = if enabled { open_log_file(&log_path).await } else { None };
    // アクションログのブロック連番（日次ローテで #1 にリセット）。
    let mut block_seq: usize = 0;

    while let Some(entry) = rx.recv().await {
        if matches!(entry, LogEntry::Shutdown) {
            break;
        }

        let now = Local::now();
        let today = now.format("%Y%m%d").to_string();
        if enabled && matches!(log_rotation, LogRotation::Daily) && today != current_date {
            if let Some(mut f) = file.take() {
                let _ = f.flush().await;
            }
            current_date = today;
            block_seq = 0;
            let new_path = build_log_path(&log_dir, &log_file_name);
            file = open_log_file(&new_path).await;
        }

        let ts = now.format("%Y-%m-%d %H:%M:%S").to_string();

        // ─── ファイルログ（kind 別フォーマット）─────────────────────────────
        if enabled {
            match kind {
                LogKind::System => {
                    if let Some(lbl) = sys_level_label(&entry) {
                        if level_enabled_for_entry(&level, &entry) {
                            let line = format!(
                                "{} │ {} │ {}\n",
                                ts,
                                pad_left_display(lbl, SYS_LEVEL_WIDTH),
                                sys_content(&entry)
                            );
                            write_file(&mut file, &line).await;
                        }
                    }
                }
                LogKind::Detect => {
                    if let LogEntry::Match { events, path, .. } = &entry {
                        let line = render_detect_line(&ts, events, path);
                        write_file(&mut file, &line).await;
                    }
                }
                LogKind::Action => match &entry {
                    LogEntry::ActionBlockStart { path, events, action_count } => {
                        block_seq += 1;
                        let line = render_block_start(block_seq, &ts, path, events, *action_count);
                        write_file(&mut file, &line).await;
                    }
                    _ => {
                        if let Some((idx, label, content)) = action_step_parts(&entry) {
                            let line = format!(
                                "{} │ {} │ {}\n",
                                ts,
                                render_step_col(idx, &label),
                                content
                            );
                            write_file(&mut file, &line).await;
                        }
                    }
                },
            }
        }

        // ─── ターミナル出力（System ロガーのみ・カラー付き従来フォーマット）───
        if console {
            match &entry {
                LogEntry::Match { rule_name, path, events } => {
                    if !level_enabled(&level, &LogLevel::Info) { continue; }
                    let event_str = format_events(events);
                    let term_line = format!(
                        "{}\n{} {}",
                        SEPARATOR.bright_green().dimmed(),
                        format!("[{ts}] [MATCH]").bright_green().bold(),
                        format!("  ルール={rule_name} | パス={path} | {event_str}")
                    );
                    println!("{}", term_line);
                }

                LogEntry::Action { index, total, action_type, detail } => {
                    if !level_enabled(&level, &LogLevel::Info) { continue; }
                    let term_line = format!(
                        "{} {}",
                        format!("[{ts}] [ACTION]").blue().bold(),
                        format!("  ({index}/{total}) {action_type}  {detail}")
                    );
                    println!("{}", term_line);
                }

                LogEntry::ActionOk { msg, .. } => {
                    if !level_enabled(&level, &LogLevel::Info) { continue; }
                    let term_line = format!(
                        "{} {}",
                        format!("[{ts}] [OK]    ").green().bold(),
                        msg
                    );
                    println!("{}", term_line);
                }

                LogEntry::ActionNote { msg, .. } => {
                    if !level_enabled(&level, &LogLevel::Info) { continue; }
                    let term_line = format!(
                        "{} {}",
                        format!("[{ts}] [INFO]").cyan(),
                        msg
                    );
                    println!("{}", term_line);
                }

                LogEntry::ActionWarn { msg, .. } => {
                    if !level_enabled(&level, &LogLevel::Warn) { continue; }
                    let term_line = format!(
                        "{} {}",
                        format!("[{ts}] [WARN]").yellow().bold(),
                        msg
                    );
                    println!("{}", term_line);
                }

                LogEntry::ActionErr { msg, .. } => {
                    if !level_enabled(&level, &LogLevel::Error) { continue; }
                    let term_line = format!(
                        "{} {}",
                        format!("[{ts}] [ERROR]").red().bold(),
                        msg
                    );
                    eprintln!("{}", term_line);
                }

                LogEntry::Info(msg) => {
                    if !level_enabled(&level, &LogLevel::Info) { continue; }
                    let term_line = format!(
                        "{} {}",
                        format!("[{ts}] [INFO]").cyan(),
                        msg
                    );
                    println!("{}", term_line);
                }

                LogEntry::Warn(msg) => {
                    if !level_enabled(&level, &LogLevel::Warn) { continue; }
                    let term_line = format!(
                        "{} {}",
                        format!("[{ts}] [WARN]").yellow().bold(),
                        msg
                    );
                    println!("{}", term_line);
                }

                LogEntry::Error(msg) => {
                    if !level_enabled(&level, &LogLevel::Error) { continue; }
                    let term_line = format!(
                        "{} {}",
                        format!("[{ts}] [ERROR]").red().bold(),
                        msg
                    );
                    eprintln!("{}", term_line);
                }

                // ブロック開始セパレータはファイル専用（コンソールには出さない）
                LogEntry::ActionBlockStart { .. } => {}
                LogEntry::Shutdown => unreachable!(),
            }
        }
    }

    if let Some(mut f) = file {
        let _ = f.flush().await;
    }
}

async fn write_file(file: &mut Option<tokio::fs::File>, line: &str) {
    if let Some(f) = file {
        if let Err(e) = f.write_all(line.as_bytes()).await {
            let ts = Local::now().format("%Y-%m-%d %H:%M:%S");
            eprintln!("{}", format!("[{ts}] [ERROR] ログ書き込み失敗: {e}").red().bold());
        }
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

fn format_events(events: &HashSet<crate::config::Event>) -> String {
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
mod tests {
    use super::*;
    use crate::config::Event;

    fn events(list: &[Event]) -> HashSet<Event> {
        list.iter().cloned().collect()
    }

    #[test]
    fn test_pad_left_display_ascii() {
        let s = pad_left_display("INFO", SYS_LEVEL_WIDTH);
        assert_eq!(s, "INFO ");
        assert_eq!(UnicodeWidthStr::width_cjk(s.as_str()), SYS_LEVEL_WIDTH);
    }

    #[test]
    fn test_pad_left_display_overflow_not_truncated() {
        let s = pad_left_display("VERYLONGTEXT", 5);
        assert_eq!(s, "VERYLONGTEXT");
    }

    /// ステップカラムはペア番号付き（`1. copy` ↔ `1. OK`）で揃う。
    #[test]
    fn test_render_step_col_numbered_pair() {
        let start = render_step_col(1, "copy");
        let ok = render_step_col(1, "OK");
        assert!(start.starts_with("1. copy"));
        assert!(ok.starts_with("1. OK"));
        assert_eq!(UnicodeWidthStr::width_cjk(start.as_str()), ACTION_STEP_WIDTH);
        assert_eq!(UnicodeWidthStr::width_cjk(ok.as_str()), ACTION_STEP_WIDTH);
    }

    /// index 0 は番号なしで描画する（チェーン全体エラー等）。
    #[test]
    fn test_render_step_col_index_zero_no_number() {
        let s = render_step_col(0, "ERR");
        assert!(s.starts_with("ERR"));
        assert!(!s.contains('.'));
    }

    /// ブロック開始セパレータに連番・パス・イベント・アクション数が入る。
    #[test]
    fn test_render_block_start_contains_fields() {
        let line = render_block_start(
            1,
            "2026-06-07 10:05:30",
            "C:/watch/csv/data.csv",
            &events(&[Event::Create]),
            2,
        );
        assert!(line.starts_with("═══ #1"));
        assert!(line.contains("C:/watch/csv/data.csv"));
        assert!(line.contains("(Create)"));
        assert!(line.contains("actions=2"));
    }

    /// 検知ログは 3 カラム（ts │ events │ path）で出力する。
    #[test]
    fn test_render_detect_line_three_columns() {
        let line = render_detect_line(
            "2026-06-07 10:05:30",
            &events(&[Event::Create, Event::Modify]),
            "C:/watch/a.csv",
        );
        assert_eq!(line.matches('│').count(), 2);
        assert!(line.contains("Create,Modify"));
        assert!(line.contains("C:/watch/a.csv"));
    }

    /// システムログには Info/Warn/Error のみラベルが付き、検知/アクション系は None。
    #[test]
    fn test_sys_level_label_skips_action_entries() {
        assert_eq!(sys_level_label(&LogEntry::Info("x".into())), Some("INFO"));
        assert_eq!(sys_level_label(&LogEntry::Warn("x".into())), Some("WARN"));
        assert_eq!(sys_level_label(&LogEntry::Error("x".into())), Some("ERROR"));
        assert_eq!(
            sys_level_label(&LogEntry::Match {
                rule_name: "r".into(),
                path: "p".into(),
                events: events(&[Event::Create]),
            }),
            None
        );
        assert_eq!(
            sys_level_label(&LogEntry::ActionOk { index: 1, total: 1, msg: "ok".into() }),
            None
        );
        assert_eq!(
            sys_level_label(&LogEntry::ActionErr { index: 1, total: 1, msg: "e".into() }),
            None
        );
    }

    /// action_step_parts が各 Action 系エントリを正しく分解する。
    #[test]
    fn test_action_step_parts_labels() {
        let ok = LogEntry::ActionOk { index: 2, total: 3, msg: "done".into() };
        assert_eq!(action_step_parts(&ok), Some((2, "OK".to_string(), "done".to_string())));
        let err = LogEntry::ActionErr { index: 1, total: 2, msg: "boom".into() };
        assert_eq!(action_step_parts(&err), Some((1, "ERR".to_string(), "boom".to_string())));
        let cmd = LogEntry::Action {
            index: 1,
            total: 1,
            action_type: "command".into(),
            detail: "shell=cmd".into(),
        };
        assert_eq!(action_step_parts(&cmd), Some((1, "cmd".to_string(), "shell=cmd".to_string())));
        // Match は Action 系ではない
        let m = LogEntry::Match {
            rule_name: "r".into(),
            path: "p".into(),
            events: events(&[Event::Create]),
        };
        assert_eq!(action_step_parts(&m), None);
    }
}
