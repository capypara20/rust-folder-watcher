//! 非同期ロガー。`LogEntry` をチャネルへ送り、[`writer`] タスクが
//! ファイル追記・ターミナル出力・ダッシュボード配信へ振り分ける。
//!
//! - [`format`] 各 `LogEntry` の整形（ファイル行・ターミナル）とレベル判定。
//! - [`writer`] チャネル受信ループ（バッチ収集・日次ローテ・ファイル書き込み）。

mod format;
mod writer;

use std::collections::HashSet;

use tokio::sync::mpsc;

use crate::config::{Event, LogLevel, LogRotation, SystemLogConfig};
use crate::error::AppError;
use writer::writer_task;

// ダッシュボードが検知イベントの表示名生成に使う。
#[cfg(feature = "dashboard")]
pub(crate) use format::format_events;

// テスト（`tests` は `super::*` でこのモジュールを参照する）が使う整形ヘルパと
// 表示幅の検証に必要な拡張トレイトをスコープへ持ち込む。
#[cfg(test)]
use format::*;
#[cfg(test)]
use unicode_width::UnicodeWidthStr;

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

#[derive(Debug)]
// `total` は将来 "N/total" 表示に使う余地を残して保持する。`Warn` は
// システム警告用の API として残す（現状の呼び出し元はまだ無い）。
#[allow(dead_code)]
pub enum LogEntry {
    /// 検知（1ファイル/フォルダのマッチ）。
    Match {
        rule_name: String,
        path: String,
        events: HashSet<Event>,
    },
    /// アクションログのブロック開始セパレータ。
    ActionBlockStart {
        path: String,
        events: HashSet<Event>,
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
        events: HashSet<Event>,
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
        events: HashSet<Event>,
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

#[cfg(test)]
#[path = "../tests/logger.rs"]
mod tests;
