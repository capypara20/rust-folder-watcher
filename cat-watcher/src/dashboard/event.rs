//! ブラウザへ配信する 1 イベント（[`DashEvent`]）と、ログエントリからの変換。

use serde::Serialize;

use crate::logger::{format_events, LogEntry};

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
    /// （`Shutdown`）は `None`。`seq` は [`super::publish`] 側で採番するのでここでは 0。
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
