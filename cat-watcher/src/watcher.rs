use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use notify::event::CreateKind;
use notify::{recommended_watcher, Event, EventKind, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use walkdir::WalkDir;

use crate::config::{RetryConfig, Rule};
use crate::error::AppError;
use crate::logger::Logger;

fn strip_unc_prefix(path: &PathBuf) -> String {
    let s = path.display().to_string();
    // canonicalize() は UNC パスを \\?\UNC\server\share 形式に変換する。
    // \\?\UNC\ → \\ に変換して \\server\share の通常 UNC 形式に戻す。
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{}", rest);
    }
    // ローカルパスの拡張形式 \\?\C:\... → C:\... に変換。
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        return rest.to_string();
    }
    s
}

pub async fn start_watching(
    rules: &[Rule],
    retry: &RetryConfig,
    scan_on_start: bool,
    log: Arc<Logger>,
) -> Result<(), AppError> {
    let (tx, rx) = mpsc::channel::<notify::Result<Event>>(100);

    // 起動時スキャンも同じ検知チャネルへ合流させるため、送信ハンドルを複製しておく。
    let scan_tx = tx.clone();

    let mut watcher = recommended_watcher(move |res| {
        let _ = tx.blocking_send(res);
    })
    .map_err(|e| AppError::Watch(format!("watcher 作成失敗: {}", e)))?;

    let mut watch_map: HashMap<PathBuf, RecursiveMode> = HashMap::new();
    for rule in rules {
        let path = PathBuf::from(&rule.watch.path);
        let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
        let canonical_display = strip_unc_prefix(&canonical);
        let mode = if rule.watch.recursive {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };

        let events_str = rule
            .watch
            .events
            .iter()
            .map(|e| match e {
                crate::config::Event::Create => "作成",
                crate::config::Event::Modify => "変更",
                crate::config::Event::Delete => "削除",
                crate::config::Event::Rename => "リネーム",
            })
            .collect::<Vec<_>>()
            .join(", ");

        let recursive_str = if rule.watch.recursive { "あり" } else { "なし" };
        let status = if rule.enabled { "有効" } else { "無効" };

        log.info(format!(
            "監視ルール [{}] ({})  パス={}  イベント={}  サブフォルダ={}",
            rule.name,
            status,
            canonical_display,
            events_str,
            recursive_str,
        ));

        // 無効ルールは一覧に記載するだけで、フィルタ詳細表示と監視登録は行わない。
        if !rule.enabled {
            continue;
        }

        // 包含ファイルフィルタ
        if let Some(pats) = &rule.watch.patterns {
            log.info(format!("  包含ファイル: {}", pats.join(", ")));
        } else if let Some(re) = &rule.watch.regex {
            log.info(format!("  包含ファイル: regex: {re}"));
        }

        // 除外ファイルフィルタ
        if !rule.watch.exclude_patterns.is_empty() {
            log.info(format!("  除外ファイル: {}", rule.watch.exclude_patterns.join(", ")));
        } else if let Some(re) = &rule.watch.exclude_regex {
            log.info(format!("  除外ファイル: regex: {re}"));
        }

        // 包含フォルダフィルタ
        if !rule.watch.dir_patterns.is_empty() {
            log.info(format!("  包含フォルダ: {}", rule.watch.dir_patterns.join(", ")));
        } else if let Some(re) = &rule.watch.dir_regex {
            log.info(format!("  包含フォルダ: regex: {re}"));
        }

        // 除外フォルダフィルタ
        if !rule.watch.exclude_dir_patterns.is_empty() {
            log.info(format!("  除外フォルダ: {}", rule.watch.exclude_dir_patterns.join(", ")));
        } else if let Some(re) = &rule.watch.exclude_dir_regex {
            log.info(format!("  除外フォルダ: regex: {re}"));
        }

        watch_map
            .entry(path)
            .and_modify(|existing| {
                if mode == RecursiveMode::Recursive {
                    *existing = RecursiveMode::Recursive;
                }
            })
            .or_insert(mode);
    }

    for (path, mode) in &watch_map {
        watcher.watch(path, *mode).map_err(|e| {
            AppError::Watch(format!("watcher 監視登録失敗 ({}): {}", path.display(), e))
        })?;
    }

    // 監視登録「後」に起動時スキャンを開始する。先に watch を張ることで、
    // スキャン中に新規到着したファイルも取りこぼさない（既存分と二重に
    // 投入されても、router 側のデバウンスが同一パスを 1 件に束ねる）。
    if scan_on_start {
        spawn_startup_scan(watch_map.clone(), scan_tx, Arc::clone(&log));
    } else {
        drop(scan_tx);
    }

    let (compiled_rules, rule_log_handles) = crate::router::compile_rules(rules)?;
    crate::router::run_router(rx, &compiled_rules, retry, Arc::clone(&log)).await?;

    // ルール別ロガー（検知・アクション）をシャットダウン
    for rule in &compiled_rules {
        if let Some(dl) = &rule.detect_logger {
            dl.shutdown();
        }
        if let Some(al) = &rule.action_logger {
            al.shutdown();
        }
    }
    for handle in rule_log_handles {
        let _ = handle.await;
    }
    Ok(())
}

/// 監視対象に既に存在するエントリを列挙し、Create イベントに変換する。
///
/// 各監視ルートを、その登録モード（recursive / non-recursive）と同じ深さで走査する。
/// ルート自身は除外（min_depth=1）。実際にどのルールが処理するか（patterns / events=
/// create を含むか等）は router の `evaluate_rule` が最終判定するため、ここでは
/// 種別（File/Folder）だけ付けて素直に列挙する。
fn collect_existing_events(watch_map: &HashMap<PathBuf, RecursiveMode>) -> Vec<Event> {
    let mut events = Vec::new();
    for (path, mode) in watch_map {
        let recursive = matches!(mode, RecursiveMode::Recursive);
        let mut walker = WalkDir::new(path).min_depth(1);
        if !recursive {
            walker = walker.max_depth(1);
        }
        for entry in walker.into_iter().filter_map(|e| e.ok()) {
            let kind = if entry.file_type().is_dir() {
                EventKind::Create(CreateKind::Folder)
            } else {
                EventKind::Create(CreateKind::File)
            };
            events.push(Event::new(kind).add_path(entry.path().to_path_buf()));
        }
    }
    events
}

/// 起動時スキャンを別スレッドで実行し、既存エントリを検知チャネルへ投入する。
///
/// walkdir は同期 API なので、tokio ランタイムをブロックしないよう専用スレッドで回し、
/// 監視コールバックと同じ `blocking_send`（容量超過時は router の排出待ちで自然に
/// バックプレッシャ）でチャネルへ送る。
fn spawn_startup_scan(
    watch_map: HashMap<PathBuf, RecursiveMode>,
    tx: mpsc::Sender<notify::Result<Event>>,
    log: Arc<Logger>,
) {
    let _ = std::thread::Builder::new()
        .name("cat-watcher startup scan".to_string())
        .spawn(move || {
            let events = collect_existing_events(&watch_map);
            let total = events.len();
            let mut sent = 0usize;
            for ev in events {
                // 受信側（router）が終了していたら打ち切る。
                if tx.blocking_send(Ok(ev)).is_err() {
                    break;
                }
                sent += 1;
            }
            if total > 0 {
                log.info(format!(
                    "起動時スキャン: 既存エントリ {sent} 件を検知キューに投入しました"
                ));
            }
        });
}

#[cfg(test)]
#[path = "watcher_tests.rs"]
mod tests;
