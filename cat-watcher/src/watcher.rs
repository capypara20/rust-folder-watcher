use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use notify::{recommended_watcher, Event, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use crate::config::{Global, Rule};
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
    global: &Global,
    log: Arc<Logger>,
) -> Result<(), AppError> {
    let (tx, rx) = mpsc::channel::<notify::Result<Event>>(100);

    let mut watcher = recommended_watcher(move |res| {
        let _ = tx.blocking_send(res);
    })
    .map_err(|e| AppError::Watch(format!("watcher 作成失敗: {}", e)))?;

    let mut watch_map: HashMap<PathBuf, RecursiveMode> = HashMap::new();
    for rule in rules {
        if !rule.enabled {
            continue;
        }

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

        log.info(format!(
            "監視ルール [{}]  パス={}  イベント={}  サブフォルダ={}",
            rule.name,
            canonical_display,
            events_str,
            recursive_str,
        ));

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

    let (compiled_rules, rule_log_handles) = crate::router::compile_rules(rules, global)?;
    crate::router::run_router(rx, &compiled_rules, global, Arc::clone(&log)).await?;

    // ルール別ロガーをシャットダウン
    for rule in &compiled_rules {
        if let Some(rl) = &rule.rule_logger {
            rl.shutdown();
        }
    }
    for handle in rule_log_handles {
        let _ = handle.await;
    }
    Ok(())
}
