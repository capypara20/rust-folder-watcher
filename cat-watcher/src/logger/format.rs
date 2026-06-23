//! ログエントリの整形（ファイル行・ターミナル出力）とレベル判定。

use std::collections::HashSet;

use colored::Colorize;
use unicode_width::UnicodeWidthStr;

use super::{LogEntry, LogKind};
use crate::config::{Event, LogLevel};

pub(crate) const SEPARATOR: &str = "──────────────────────────────────────────────────────────────";

/// システムログ level カラムの表示列幅（"ERROR" = 5 列）。
pub(crate) const SYS_LEVEL_WIDTH: usize = 5;
/// 検知ログ events カラムの表示列幅。
pub(crate) const DETECT_EVENTS_WIDTH: usize = 20;
/// アクションログ ステップカラムの表示列幅（"10. copy" 程度を想定）。
pub(crate) const ACTION_STEP_WIDTH: usize = 9;

/// 表示列幅 (East Asian Width, CJK モード) で左寄せパディングする。
/// Rust 標準の `format!("{:<width$}")` は char 数で揃えるため、
/// '│' '═' などの罫線記号で表示時に列幅がズレる。
/// 本プロジェクトは日本語ロケール (CJK) を主用途とするため、
/// East Asian Ambiguous 文字を 2 列幅として扱う `width_cjk()` を使う。
pub(crate) fn pad_left_display(s: &str, total_cols: usize) -> String {
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
pub(crate) fn sys_level_label(entry: &LogEntry) -> Option<&'static str> {
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
pub(crate) fn action_step_parts(entry: &LogEntry) -> Option<(usize, String, String)> {
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
pub(crate) fn render_step_col(index: usize, label: &str) -> String {
    let s = if index == 0 {
        label.to_string()
    } else {
        format!("{index}. {label}")
    };
    pad_left_display(&s, ACTION_STEP_WIDTH)
}

/// アクションログのブロック開始セパレータ行を生成する。
pub(crate) fn render_block_start(
    seq: usize,
    ts: &str,
    path: &str,
    events: &HashSet<Event>,
    action_count: usize,
) -> String {
    format!(
        "═══ #{seq}  {ts}  {path}  ({})  actions={action_count} ═══\n",
        format_events(events)
    )
}

/// 検知ログの1行を生成する。
pub(crate) fn render_detect_line(ts: &str, events: &HashSet<Event>, path: &str) -> String {
    format!(
        "{} │ {} │ {}\n",
        ts,
        pad_left_display(&format_events(events), DETECT_EVENTS_WIDTH),
        path
    )
}

/// 1 エントリのファイル出力行を生成する（kind 別フォーマット）。
/// 書き込み対象外なら None。`block_seq` はアクションブロック開始で +1 する。
pub(crate) fn file_line(
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
pub(crate) fn console_print(entry: &LogEntry, ts: &str, level: &LogLevel) {
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

pub(crate) fn format_events(events: &HashSet<Event>) -> String {
    let mut names: Vec<&str> = events
        .iter()
        .map(|e| match e {
            Event::Create => "Create",
            Event::Modify => "Modify",
            Event::Delete => "Delete",
            Event::Rename => "Rename",
        })
        .collect();
    names.sort();
    names.join(",")
}
