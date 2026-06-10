use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use notify::EventKind;
use notify::event::{CreateKind, ModifyKind, RemoveKind};
use globset::{Glob, GlobSet, GlobSetBuilder};
use regex::Regex;
use crate::{config::{ActionConfig, Event, RetryConfig, Rule, WatchTarget}, error::AppError};
use crate::logger::Logger;

/// イベントが指すエントリの種別。
/// notify サブタイプ (Create/Remove のみ確実) から取得。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EntryKind {
    File,
    Dir,
}

/// notify のイベントサブタイプから EntryKind を取得する。
///
/// Windows 10 1709+ では `notify-fork` が `ReadDirectoryChangesExW` を使い、
/// Create / Remove で File/Folder の種別を返してくれるためここで判別できる。
/// Modify(Any) / Modify(Name(_)) は種別なし (Modify はそもそも File にしか起きない、
/// Rename はパスがまだ存在するので呼び出し側で `path.is_file/is_dir()` で解決する)。
///
/// 旧 Windows (Win10 旧版 / Server 2016 等) や notify が `Any` を返す経路では None。
fn kind_from_notify_event(event: &notify::Event) -> Option<EntryKind> {
    match &event.kind {
        EventKind::Create(CreateKind::File) | EventKind::Remove(RemoveKind::File) => {
            Some(EntryKind::File)
        }
        EventKind::Create(CreateKind::Folder) | EventKind::Remove(RemoveKind::Folder) => {
            Some(EntryKind::Dir)
        }
        _ => None,
    }
}

pub struct CompiledRule{
	pub name: String,
	pub enabled: bool,
	pub watch_path: String,
	pub recursive: bool,
	pub target: WatchTarget,
	pub include_hidden: bool,
	pub events: Vec<Event>,
	pub glob_set: Option<GlobSet>,
	pub exclude_glob_set: Option<GlobSet>,
	pub exclude_regex: Option<Regex>,
	pub exclude_dir_glob_set: Option<GlobSet>,
	pub exclude_dir_regex: Option<Regex>,
	pub dir_glob_set: Option<GlobSet>,
	pub dir_regex: Option<Regex>,
	pub regexes: Option<Regex>,
	pub actions: Vec<ActionConfig>,
	pub detect_logger: Option<Arc<Logger>>,
	pub action_logger: Option<Arc<Logger>>,
}

pub fn compile_rules(rules: &[Rule]) -> Result<(Vec<CompiledRule>, Vec<tokio::task::JoinHandle<()>>), AppError> {
	let mut compiled_rules = Vec::new();
	let mut log_handles = Vec::new();
	for rule in rules{
		let glob_set = if let Some(patterns) = &rule.watch.patterns {
			let mut builder = GlobSetBuilder::new();
			for p in patterns {
				builder.add(Glob::new(p).map_err(|e| AppError::Watch(e.to_string()))?);
			}
			Some(builder.build().map_err(|e| AppError::Watch(e.to_string()))?)
		} else {
			None
		};
		// exclude_patterns → GlobSet
		// regex → Regex
		// CompiledRule を生成して compiled_rules に追加
		let exclude_glob_set = if !rule.watch.exclude_patterns.is_empty() {
			let mut builder = GlobSetBuilder::new();
			for p in &rule.watch.exclude_patterns {
				builder.add(Glob::new(p).map_err(|e| AppError::Watch(e.to_string()))?);
			}
			Some(builder.build().map_err(|e| AppError::Watch(e.to_string()))?)
		} else {
			None
		};

		let regexes = if let Some(re_str) = &rule.watch.regex {
			Some(Regex::new(re_str).map_err(|e| AppError::Watch(e.to_string()))?)
		} else {
			None
		};

		let exclude_regex = if let Some(re_str) = &rule.watch.exclude_regex {
			Some(Regex::new(re_str).map_err(|e| AppError::Watch(e.to_string()))?)
		} else {
			None
		};

		let exclude_dir_glob_set = if !rule.watch.exclude_dir_patterns.is_empty() {
			let mut builder = GlobSetBuilder::new();
			for p in &rule.watch.exclude_dir_patterns {
				builder.add(Glob::new(p).map_err(|e| AppError::Watch(e.to_string()))?);
			}
			Some(builder.build().map_err(|e| AppError::Watch(e.to_string()))?)
		} else {
			None
		};

		let exclude_dir_regex = if let Some(re_str) = &rule.watch.exclude_dir_regex {
			Some(Regex::new(re_str).map_err(|e| AppError::Watch(e.to_string()))?)
		} else {
			None
		};

		let dir_glob_set = if !rule.watch.dir_patterns.is_empty() {
			let mut builder = GlobSetBuilder::new();
			for p in &rule.watch.dir_patterns {
				builder.add(Glob::new(p).map_err(|e| AppError::Watch(e.to_string()))?);
			}
			Some(builder.build().map_err(|e| AppError::Watch(e.to_string()))?)
		} else {
			None
		};

		let dir_regex = if let Some(re_str) = &rule.watch.dir_regex {
			Some(Regex::new(re_str).map_err(|e| AppError::Watch(e.to_string()))?)
		} else {
			None
		};
		
		// 検知ログ・アクションログをそれぞれ個別に生成する（global からの
		// フォールバックは廃止。各ターゲットが dir/file_name/rotation を必須で持つ）。
		let mut detect_logger = None;
		let mut action_logger = None;
		if let Some(rule_log) = &rule.log {
			if let Some(detect) = &rule_log.detect {
				if detect.enabled {
					let (logger, handle) = Logger::for_detect(
						detect.dir.clone(),
						detect.file_name.clone(),
						detect.rotation.clone(),
					)?;
					log_handles.push(handle);
					detect_logger = Some(Arc::new(logger));
				}
			}
			if let Some(action) = &rule_log.action {
				if action.enabled {
					let (logger, handle) = Logger::for_action(
						action.dir.clone(),
						action.file_name.clone(),
						action.rotation.clone(),
					)?;
					log_handles.push(handle);
					action_logger = Some(Arc::new(logger));
				}
			}
		}

		compiled_rules.push(CompiledRule {
			name: rule.name.clone(),
			enabled: rule.enabled,
			watch_path: rule.watch.path.clone(),
			recursive: rule.watch.recursive,
			target: rule.watch.target.clone(),
			include_hidden: rule.watch.include_hidden,
			events: rule.watch.events.clone(),
			glob_set,
			exclude_glob_set,
			exclude_regex,
			exclude_dir_glob_set,
			exclude_dir_regex,
			dir_glob_set,
			dir_regex,
			regexes,
			actions: rule.actions.clone(),
			detect_logger,
			action_logger,
		});
	}
	Ok((compiled_rules, log_handles))
}

fn evaluate_rule(
    path: &Path,
    detected_events: &HashSet<Event>,
    kind: Option<EntryKind>,
    rule: &CompiledRule,
) -> bool {
	if !rule.enabled { return false; }
    if !matches_target(path, &rule.target, kind) { return false; }
    if !matches_hidden(path, rule.include_hidden) { return false; }

    let watch_path = Path::new(&rule.watch_path);
    if rule.recursive {
        if !path.starts_with(watch_path) { return false; }
    } else {
        if path.parent() != Some(watch_path) { return false; }
    }

    if !matches_pattern(path, rule) { return false; }
    if !matches_events(detected_events, &rule.events) { return false; }

    true
}

/// target フィルタ: file/directory/both の判定。
///
/// kind は notify サブタイプ (CreateKind / RemoveKind) から取得済み。
/// - Create / Remove は notify-fork が File/Folder を通知 → kind=Some
/// - Modify / Rename は kind=None → パスが存在するので path.is_file/is_dir() で判定
/// - 旧 Windows (Win10 1709 未満 / Server 2016 等) で Remove(Any) かつ kind=None
///   の場合はパスが消えていて判定不能。target=file / target=directory には
///   マッチさせず、target=both を使う運用とする (README で案内予定)。
fn matches_target(path: &Path, target: &WatchTarget, kind: Option<EntryKind>) -> bool {
    match target {
        WatchTarget::Both => true,
        WatchTarget::File => match kind {
            Some(EntryKind::File) => true,
            Some(EntryKind::Dir) => false,
            None => path.is_file(),
        },
        WatchTarget::Directory => match kind {
            Some(EntryKind::Dir) => true,
            Some(EntryKind::File) => false,
            None => path.is_dir(),
        },
    }
}

/// include_hidden フィルタ（Phase 12 まではスタブ）
fn matches_hidden(_path: &Path, _include_hidden: bool) -> bool {
    true  // 今は常に通過
}

/// patterns / exclude_patterns / regex / exclude_dir_* マッチ
fn matches_pattern(path: &Path, rule: &CompiledRule) -> bool{
	let file_name = match path.file_name().and_then(|n| n.to_str()) {
		Some(name) => name,
		None => return false,
	};

	if let Some(glob_set) = &rule.glob_set {
		if !glob_set.is_match(file_name) {
			return false;
		}
	}

	if let Some(exclude_glob_set) = &rule.exclude_glob_set {
		if exclude_glob_set.is_match(file_name) {
			return false;
		}
	}

	if let Some(exclude_regex) = &rule.exclude_regex {
		if exclude_regex.is_match(file_name) {
			return false;
		}
	}

	if let Some(regexes) = &rule.regexes {
		if !regexes.is_match(file_name) {
			return false;
		}
	}

	// ディレクトリ名包含: watch_path からの相対パスの親コンポーネントの少なくとも1つがマッチすること
	if rule.dir_glob_set.is_some() || rule.dir_regex.is_some() {
		let watch_root = Path::new(&rule.watch_path);
		if let Ok(rel) = path.strip_prefix(watch_root) {
			let rel_parent = rel.parent().unwrap_or(Path::new(""));
			let matched = rel_parent.components().any(|component| {
				if let std::path::Component::Normal(os_name) = component {
					if let Some(dir_name) = os_name.to_str() {
						if let Some(gs) = &rule.dir_glob_set {
							if gs.is_match(dir_name) { return true; }
						}
						if let Some(re) = &rule.dir_regex {
							if re.is_match(dir_name) { return true; }
						}
					}
				}
				false
			});
			if !matched { return false; }
		} else {
			return false;
		}
	}

	// ディレクトリ名除外: watch_path からの相対パスの各ディレクトリコンポーネントを評価
	if rule.exclude_dir_glob_set.is_some() || rule.exclude_dir_regex.is_some() {
		let watch_root = Path::new(&rule.watch_path);
		if let Ok(rel) = path.strip_prefix(watch_root) {
			let rel_parent = rel.parent().unwrap_or(Path::new(""));
			for component in rel_parent.components() {
				if let std::path::Component::Normal(os_name) = component {
					if let Some(dir_name) = os_name.to_str() {
						if let Some(gs) = &rule.exclude_dir_glob_set {
							if gs.is_match(dir_name) { return false; }
						}
						if let Some(re) = &rule.exclude_dir_regex {
							if re.is_match(dir_name) { return false; }
						}
					}
				}
			}
		}
	}

	true
}

/// events 積集合判定
fn matches_events(detected: &HashSet<Event>, rule_events: &[Event]) -> bool {
    rule_events.iter().any(|e| detected.contains(e))
}

fn to_config_event(kind: &EventKind) -> Option<Event> {
	match kind {
		EventKind::Create(_) => Some(Event::Create),
		// notify はリネームを EventKind::Modify(ModifyKind::Name(_)) として通知するため、
		// 通常の Modify と区別して Event::Rename にマッピングする (issue #30)
		EventKind::Modify(ModifyKind::Name(_)) => Some(Event::Rename),
		EventKind::Modify(_) => Some(Event::Modify),
		EventKind::Remove(_) => Some(Event::Delete),
		_ => None,
	}
}

pub async fn run_router(
    mut rx: mpsc::Receiver<notify::Result<notify::Event>>,
    compiled_rules: &[CompiledRule],
    retry: &RetryConfig,
    log: Arc<Logger>,
) -> Result<(), AppError> {
    // デバウンス用マップ: パス → (イベント集合, 最後の受信時刻, EntryKind)
    // EntryKind は notify サブタイプから推定したもの。確実な (Some) 値が来たら上書き。
    let mut pending: HashMap<PathBuf, (HashSet<Event>, Instant, Option<EntryKind>)> = HashMap::new();
    let mut interval = tokio::time::interval(Duration::from_millis(100));

    loop {
        tokio::select! {
            // (A) watcher からイベント受信 → pending に蓄積
            Some(res) = rx.recv() => {
                if let Ok(event) = res {
                    let notify_kind = kind_from_notify_event(&event);
                    if let Some(config_event) = to_config_event(&event.kind) {
                        for path in &event.paths {
                            let entry = pending
                                .entry(path.clone())
                                .or_insert_with(|| (HashSet::new(), Instant::now(), notify_kind));
                            entry.0.insert(config_event.clone());
                            entry.1 = Instant::now();
                            // notify から確実な種別が取れたら上書きする
                            // (Create/Remove は種別が来る、Modify/Rename は None)
                            if notify_kind.is_some() {
                                entry.2 = notify_kind;
                            }
                        }
                    }
                }
            }

            // (B) 100ms タイマー → 500ms 経過分を取り出して評価
            _ = interval.tick() => {
                let now = Instant::now();
                let ready: Vec<(PathBuf, HashSet<Event>, Option<EntryKind>)> = pending.iter()
                    .filter(|(_, (_, last, _))| now.duration_since(*last) >= Duration::from_millis(500))
                    .map(|(path, (events, _, kind))| (path.clone(), events.clone(), *kind))
                    .collect();

                for (path, detected_events, kind) in ready {
                    pending.remove(&path);
                    for rule in compiled_rules {
                        if !evaluate_rule(&path, &detected_events, kind, rule) {
                            continue;
                        }

                        // 検知: ターミナル（system）＋検知ログ（ルール別ファイル）へ
                        log.log_match(
                            &rule.name,
                            path.display().to_string(),
                            detected_events.clone(),
                        );
                        if let Some(dl) = &rule.detect_logger {
                            dl.log_match(
                                &rule.name,
                                path.display().to_string(),
                                detected_events.clone(),
                            );
                        }

                        // アクションログのブロック開始セパレータ
                        if let Some(al) = &rule.action_logger {
                            al.log_block_start(
                                path.display().to_string(),
                                detected_events.clone(),
                                rule.actions.len(),
                            );
                        }

                        let watch_path = PathBuf::from(&rule.watch_path);
                        // アクション失敗時のログ出力は execute_chain 内（ActionSink）で
                        // 完結する（ターミナル＋アクションログ）。ここでは再ログしない。
                        let _ = crate::actions::execute_chain(
                            &rule.actions,
                            &path,
                            &watch_path,
                            retry,
                            Arc::clone(&log),
                            rule.action_logger.clone(),
                        ).await;
                    }
                }
            }

            // (C) Ctrl+C → 終了
            _ = tokio::signal::ctrl_c() => {
                log.info("終了シグナル受信");
                break;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "router_tests.rs"]
mod tests;
