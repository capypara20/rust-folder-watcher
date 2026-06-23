//! ブラウザでログをリアルタイム表示するための軽量 HTTP/SSE サーバ。
//!
//! `dashboard` feature 有効時のみコンパイルされる。既存のログ経路（system
//! ロガーの `writer_task`）から [`publish`] でイベントを受け取り、
//! `tokio::sync::broadcast` で各ブラウザ接続へ配信する。配信は SSE
//! （Server-Sent Events）。
//!
//! 設計方針:
//! - **取りこぼし許容**（broadcast）。UI が詰まっても監視・アクションは遅延させない。
//! - ルートは GET 3 本（`/`・`/events`・`/search`）のみ。薄い自前 HTTP 実装で完結。
//! - 既定は localhost 束縛。ログにパスが出るため外部公開は呼び出し側の責任。
//!
//! 構成:
//! - [`event`]  配信イベント [`DashEvent`] とログエントリからの変換。
//! - [`server`] HTTP/SSE サーバ本体（接続受理・ルーティング）。
//! - [`search`] 保存済みログファイルの横断検索。

mod event;
mod search;
mod server;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::{Mutex, OnceLock};

use tokio::sync::broadcast;

use crate::logger::Logger;

pub use event::DashEvent;
use search::LogSource;

// テスト（`tests` は `super::*` でこのモジュールを参照する）が使う検索ヘルパ。
#[cfg(test)]
use search::{percent_decode, query_param, search_sources, static_affixes};

/// 配信チャネルのバッファ件数。これを超えて遅延した接続は Lagged になる。
const CHANNEL_CAPACITY: usize = 512;
/// リングバッファの上限（history がこれより大きくても確保時はここで抑える）。
const BACKLOG_HARD_CAP: usize = 4096;

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
fn init(history: usize, sources: Vec<LogSource>) {
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
    tokio::spawn(async move { server::serve(bind, log).await });
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
