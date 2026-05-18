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
use crate::{config::{ActionConfig, Event, Global, Rule, WatchTarget}, error::AppError};
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
	pub rule_logger: Option<Arc<Logger>>,
}

pub fn compile_rules(rules: &[Rule], global: &Global) -> Result<(Vec<CompiledRule>, Vec<tokio::task::JoinHandle<()>>), AppError> {
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
		
		let rule_logger = if let Some(rule_log) = &rule.log {
			if rule_log.enabled {
				let log_dir = rule_log.log_dir.clone()
					.unwrap_or_else(|| global.log_dir.clone());
				let log_file_name = rule_log.log_file_name.clone()
					.unwrap_or_else(|| global.log_file_name.clone());
				let log_rotation = rule_log.log_rotation.clone()
					.unwrap_or_else(|| global.log_rotation.clone());
				let file_level = global.file_log_level.clone()
					.unwrap_or_else(|| global.log_level.clone());
				let (logger, handle) = Logger::for_rule(log_dir, log_file_name, log_rotation, file_level)?;
				log_handles.push(handle);
				Some(Arc::new(logger))
			} else {
				None
			}
		} else {
			None
		};

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
			rule_logger,
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
    global: &Global,
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

                        log.log_match(
                            &rule.name,
                            path.display().to_string(),
                            detected_events.clone(),
                        );
                        if let Some(rl) = &rule.rule_logger {
                            rl.log_match(
                                &rule.name,
                                path.display().to_string(),
                                detected_events.clone(),
                            );
                        }

                        let watch_path = PathBuf::from(&rule.watch_path);
                        if let Err(e) = crate::actions::execute_chain(
                            &rule.actions,
                            &path,
                            &watch_path,
                            global,
                            Arc::clone(&log),
                            rule.rule_logger.clone(),
                        ).await {
                            log.error(format!(
                                "アクションチェーン実行エラー: ルール={}, パス={}, エラー={}",
                                rule.name, path.display(), e
                            ));
                            if let Some(rl) = &rule.rule_logger {
                                rl.error(format!(
                                    "アクションチェーン実行エラー: パス={}, エラー={}",
                                    path.display(), e
                                ));
                            }
                        }
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
mod tests {
    use super::*;
    use std::collections::HashSet;
    use tempfile::TempDir;

    fn make_rule(watch_path: &str, recursive: bool, patterns: Option<Vec<&str>>) -> CompiledRule {
        let glob_set = patterns.map(|pats| {
            let mut builder = GlobSetBuilder::new();
            for p in pats {
                builder.add(Glob::new(p).unwrap());
            }
            builder.build().unwrap()
        });
        CompiledRule {
            name: format!("rule-{}", watch_path),
            enabled: true,
            watch_path: watch_path.to_string(),
            recursive,
            target: WatchTarget::Both,
            include_hidden: false,
            events: vec![Event::Create],
            glob_set,
            exclude_glob_set: None,
            exclude_regex: None,
            exclude_dir_glob_set: None,
            exclude_dir_regex: None,
            dir_glob_set: None,
            dir_regex: None,
            regexes: None,
            actions: vec![],
            rule_logger: None,
        }
    }

    fn create_events(e: Event) -> HashSet<Event> {
        let mut s = HashSet::new();
        s.insert(e);
        s
    }

    // 2つの監視ディレクトリを用意し、片方で検知したファイルが
    // もう片方のルールに誤マッチしないことを確認する（バグ再現テスト）
    #[test]
    fn test_no_cross_directory_match() {
        let dir_a = TempDir::new().unwrap();
        let dir_b = TempDir::new().unwrap();

        let rule_a = make_rule(dir_a.path().to_str().unwrap(), false, Some(vec!["*.csv"]));
        let rule_b = make_rule(dir_b.path().to_str().unwrap(), false, Some(vec!["*.csv"]));

        let file_in_a = dir_a.path().join("data.csv");
        std::fs::write(&file_in_a, "").unwrap();
        let events = create_events(Event::Create);

        assert!(evaluate_rule(&file_in_a, &events, None, &rule_a), "dir_a のルールはマッチすべき");
        assert!(!evaluate_rule(&file_in_a, &events, None, &rule_b), "dir_b のルールはマッチしてはいけない");
    }

    // recursive=false でサブディレクトリのファイルが除外されることを確認
    #[test]
    fn test_non_recursive_excludes_subdir() {
        let dir = TempDir::new().unwrap();
        let subdir = dir.path().join("sub");
        std::fs::create_dir(&subdir).unwrap();
        let file_in_sub = subdir.join("data.csv");
        std::fs::write(&file_in_sub, "").unwrap();

        let rule = make_rule(dir.path().to_str().unwrap(), false, Some(vec!["*.csv"]));
        let events = create_events(Event::Create);

        assert!(!evaluate_rule(&file_in_sub, &events, None, &rule), "サブディレクトリのファイルはマッチしてはいけない");
    }

    // recursive=true ではサブディレクトリのファイルもマッチすることを確認
    #[test]
    fn test_recursive_includes_subdir() {
        let dir = TempDir::new().unwrap();
        let subdir = dir.path().join("sub");
        std::fs::create_dir(&subdir).unwrap();
        let file_in_sub = subdir.join("data.csv");
        std::fs::write(&file_in_sub, "").unwrap();

        let rule = make_rule(dir.path().to_str().unwrap(), true, Some(vec!["*.csv"]));
        let events = create_events(Event::Create);

        assert!(evaluate_rule(&file_in_sub, &events, None, &rule), "recursive=true ならサブディレクトリもマッチすべき");
    }

    // 直下のファイルは recursive=false でもマッチすることを確認
    #[test]
    fn test_non_recursive_matches_direct_child() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("data.csv");
        std::fs::write(&file, "").unwrap();

        let rule = make_rule(dir.path().to_str().unwrap(), false, Some(vec!["*.csv"]));
        let events = create_events(Event::Create);

        assert!(evaluate_rule(&file, &events, None, &rule));
    }

    // パターンに合わないファイルは除外されることを確認
    #[test]
    fn test_pattern_mismatch_excluded() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("image.png");
        std::fs::write(&file, "").unwrap();

        let rule = make_rule(dir.path().to_str().unwrap(), false, Some(vec!["*.csv"]));
        let events = create_events(Event::Create);

        assert!(!evaluate_rule(&file, &events, None, &rule));
    }

    // -----------------------------------------------------------------
    // to_config_event: Rename / Modify サブタイプのマッピング (#30 回帰)
    // -----------------------------------------------------------------
    use notify::event::{DataChange, RenameMode};

    #[test]
    fn test_to_config_event_rename_from() {
        let kind = EventKind::Modify(ModifyKind::Name(RenameMode::From));
        assert_eq!(to_config_event(&kind), Some(Event::Rename));
    }

    #[test]
    fn test_to_config_event_rename_to() {
        let kind = EventKind::Modify(ModifyKind::Name(RenameMode::To));
        assert_eq!(to_config_event(&kind), Some(Event::Rename));
    }

    #[test]
    fn test_to_config_event_rename_any() {
        let kind = EventKind::Modify(ModifyKind::Name(RenameMode::Any));
        assert_eq!(to_config_event(&kind), Some(Event::Rename));
    }

    #[test]
    fn test_to_config_event_modify_data_still_modify() {
        let kind = EventKind::Modify(ModifyKind::Data(DataChange::Content));
        assert_eq!(to_config_event(&kind), Some(Event::Modify));
    }

    // -----------------------------------------------------------------
    // matches_target: kind ベース判定 (#23 / #24 回帰)
    // -----------------------------------------------------------------

    // target=file + Remove(File) → kind=Some(File) でマッチ (削除後でも判定できる)
    #[test]
    fn test_matches_target_file_with_kind_file() {
        let path = Path::new("/nonexistent/will_not_exist.txt");
        assert!(matches_target(path, &WatchTarget::File, Some(EntryKind::File)));
        assert!(!matches_target(path, &WatchTarget::File, Some(EntryKind::Dir)));
    }

    // target=directory + Remove(Folder) → kind=Some(Dir) でマッチ
    #[test]
    fn test_matches_target_directory_with_kind_dir() {
        let path = Path::new("/nonexistent/will_not_exist_dir");
        assert!(matches_target(path, &WatchTarget::Directory, Some(EntryKind::Dir)));
        assert!(!matches_target(path, &WatchTarget::Directory, Some(EntryKind::File)));
    }

    // target=both は kind に関係なく常に true
    #[test]
    fn test_matches_target_both_always_true() {
        let path = Path::new("/nonexistent/whatever");
        assert!(matches_target(path, &WatchTarget::Both, Some(EntryKind::File)));
        assert!(matches_target(path, &WatchTarget::Both, Some(EntryKind::Dir)));
        assert!(matches_target(path, &WatchTarget::Both, None));
    }

    // kind=None + 実在ファイル → File にマッチ (Modify/Rename パスの fallback)
    #[test]
    fn test_matches_target_file_kind_none_existing_file() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("real.txt");
        std::fs::write(&file, "").unwrap();
        assert!(matches_target(&file, &WatchTarget::File, None));
        assert!(!matches_target(&file, &WatchTarget::Directory, None));
    }

    // kind=None + パスなし (旧 Windows の Remove(Any) や Rename(From) の旧パス等)
    // → 判定不能なので target=file / target=directory どちらにも通さない (厳格)。
    //   旧 OS で Delete を確実に拾いたい場合は target=both を使う運用とする。
    //   target=both は kind に依存しないので常に通る。
    #[test]
    fn test_matches_target_kind_none_path_missing_does_not_match() {
        let path = Path::new("/definitely/does/not/exist_xyz_12345");
        assert!(!matches_target(path, &WatchTarget::File, None));
        assert!(!matches_target(path, &WatchTarget::Directory, None));
        // target=both は通る
        assert!(matches_target(path, &WatchTarget::Both, None));
    }

    // -----------------------------------------------------------------
    // exclude_regex: ファイル名正規表現除外 (#28)
    // -----------------------------------------------------------------

    #[test]
    fn test_exclude_regex_excludes_matching_file() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("debug_001.log");
        std::fs::write(&file, "").unwrap();

        let mut rule = make_rule(dir.path().to_str().unwrap(), false, None);
        rule.exclude_regex = Some(Regex::new(r"^debug_\d+").unwrap());
        let events = create_events(Event::Create);

        assert!(!evaluate_rule(&file, &events, None, &rule));
    }

    #[test]
    fn test_exclude_regex_passes_non_matching_file() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("report_001.log");
        std::fs::write(&file, "").unwrap();

        let mut rule = make_rule(dir.path().to_str().unwrap(), false, None);
        rule.exclude_regex = Some(Regex::new(r"^debug_\d+").unwrap());
        let events = create_events(Event::Create);

        assert!(evaluate_rule(&file, &events, None, &rule));
    }

    // -----------------------------------------------------------------
    // exclude_dir_patterns: フォルダ名 glob 除外 (#28)
    // -----------------------------------------------------------------

    #[test]
    fn test_exclude_dir_patterns_excludes_file_in_matching_dir() {
        let dir = TempDir::new().unwrap();
        let node_modules = dir.path().join("node_modules");
        std::fs::create_dir(&node_modules).unwrap();
        let file = node_modules.join("index.js");
        std::fs::write(&file, "").unwrap();

        let mut builder = GlobSetBuilder::new();
        builder.add(Glob::new("node_modules").unwrap());
        let gs = builder.build().unwrap();

        let mut rule = make_rule(dir.path().to_str().unwrap(), true, None);
        rule.exclude_dir_glob_set = Some(gs);
        let events = create_events(Event::Create);

        assert!(!evaluate_rule(&file, &events, None, &rule));
    }

    #[test]
    fn test_exclude_dir_patterns_passes_file_in_non_matching_dir() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir(&src).unwrap();
        let file = src.join("main.rs");
        std::fs::write(&file, "").unwrap();

        let mut builder = GlobSetBuilder::new();
        builder.add(Glob::new("node_modules").unwrap());
        let gs = builder.build().unwrap();

        let mut rule = make_rule(dir.path().to_str().unwrap(), true, None);
        rule.exclude_dir_glob_set = Some(gs);
        let events = create_events(Event::Create);

        assert!(evaluate_rule(&file, &events, None, &rule));
    }

    #[test]
    fn test_exclude_dir_patterns_nested_dir_excluded() {
        let dir = TempDir::new().unwrap();
        let parent = dir.path().join("packages");
        let node_modules = parent.join("node_modules");
        std::fs::create_dir_all(&node_modules).unwrap();
        let file = node_modules.join("dep.js");
        std::fs::write(&file, "").unwrap();

        let mut builder = GlobSetBuilder::new();
        builder.add(Glob::new("node_modules").unwrap());
        let gs = builder.build().unwrap();

        let mut rule = make_rule(dir.path().to_str().unwrap(), true, None);
        rule.exclude_dir_glob_set = Some(gs);
        let events = create_events(Event::Create);

        assert!(!evaluate_rule(&file, &events, None, &rule));
    }

    // -----------------------------------------------------------------
    // exclude_dir_regex: フォルダ名正規表現除外 (#28)
    // -----------------------------------------------------------------

    #[test]
    fn test_exclude_dir_regex_excludes_file_in_matching_dir() {
        let dir = TempDir::new().unwrap();
        let hidden = dir.path().join(".cache");
        std::fs::create_dir(&hidden).unwrap();
        let file = hidden.join("data.bin");
        std::fs::write(&file, "").unwrap();

        let mut rule = make_rule(dir.path().to_str().unwrap(), true, None);
        rule.exclude_dir_regex = Some(Regex::new(r"^\.").unwrap());
        let events = create_events(Event::Create);

        assert!(!evaluate_rule(&file, &events, None, &rule));
    }

    #[test]
    fn test_exclude_dir_regex_passes_file_in_non_matching_dir() {
        let dir = TempDir::new().unwrap();
        let visible = dir.path().join("data");
        std::fs::create_dir(&visible).unwrap();
        let file = visible.join("report.csv");
        std::fs::write(&file, "").unwrap();

        let mut rule = make_rule(dir.path().to_str().unwrap(), true, None);
        rule.exclude_dir_regex = Some(Regex::new(r"^\.").unwrap());
        let events = create_events(Event::Create);

        assert!(evaluate_rule(&file, &events, None, &rule));
    }

    // -----------------------------------------------------------------
    // dir_patterns: フォルダ名 glob 包含 (#28)
    // -----------------------------------------------------------------

    #[test]
    fn test_dir_patterns_includes_file_in_matching_dir() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir(&src).unwrap();
        let file = src.join("main.rs");
        std::fs::write(&file, "").unwrap();

        let mut builder = GlobSetBuilder::new();
        builder.add(Glob::new("src").unwrap());
        let gs = builder.build().unwrap();

        let mut rule = make_rule(dir.path().to_str().unwrap(), true, None);
        rule.dir_glob_set = Some(gs);
        let events = create_events(Event::Create);

        assert!(evaluate_rule(&file, &events, None, &rule));
    }

    #[test]
    fn test_dir_patterns_excludes_file_in_non_matching_dir() {
        let dir = TempDir::new().unwrap();
        let lib = dir.path().join("lib");
        std::fs::create_dir(&lib).unwrap();
        let file = lib.join("utils.rs");
        std::fs::write(&file, "").unwrap();

        let mut builder = GlobSetBuilder::new();
        builder.add(Glob::new("src").unwrap());
        let gs = builder.build().unwrap();

        let mut rule = make_rule(dir.path().to_str().unwrap(), true, None);
        rule.dir_glob_set = Some(gs);
        let events = create_events(Event::Create);

        assert!(!evaluate_rule(&file, &events, None, &rule));
    }

    #[test]
    fn test_dir_patterns_direct_child_not_matched() {
        // watch 直下のファイルはディレクトリコンポーネントが存在しないためマッチしない
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("root.csv");
        std::fs::write(&file, "").unwrap();

        let mut builder = GlobSetBuilder::new();
        builder.add(Glob::new("src").unwrap());
        let gs = builder.build().unwrap();

        let mut rule = make_rule(dir.path().to_str().unwrap(), false, None);
        rule.dir_glob_set = Some(gs);
        let events = create_events(Event::Create);

        assert!(!evaluate_rule(&file, &events, None, &rule));
    }

    #[test]
    fn test_dir_patterns_nested_dir_matched() {
        // 深いネストでも途中にマッチするフォルダがあれば通る
        let dir = TempDir::new().unwrap();
        let nested = dir.path().join("packages").join("src").join("components");
        std::fs::create_dir_all(&nested).unwrap();
        let file = nested.join("button.tsx");
        std::fs::write(&file, "").unwrap();

        let mut builder = GlobSetBuilder::new();
        builder.add(Glob::new("src").unwrap());
        let gs = builder.build().unwrap();

        let mut rule = make_rule(dir.path().to_str().unwrap(), true, None);
        rule.dir_glob_set = Some(gs);
        let events = create_events(Event::Create);

        assert!(evaluate_rule(&file, &events, None, &rule));
    }

    // -----------------------------------------------------------------
    // dir_regex: フォルダ名正規表現包含 (#28)
    // -----------------------------------------------------------------

    #[test]
    fn test_dir_regex_includes_file_in_matching_dir() {
        let dir = TempDir::new().unwrap();
        let reports = dir.path().join("reports_2024");
        std::fs::create_dir(&reports).unwrap();
        let file = reports.join("data.csv");
        std::fs::write(&file, "").unwrap();

        let mut rule = make_rule(dir.path().to_str().unwrap(), true, None);
        rule.dir_regex = Some(Regex::new(r"^reports_\d+$").unwrap());
        let events = create_events(Event::Create);

        assert!(evaluate_rule(&file, &events, None, &rule));
    }

    #[test]
    fn test_dir_regex_excludes_file_in_non_matching_dir() {
        let dir = TempDir::new().unwrap();
        let other = dir.path().join("archives");
        std::fs::create_dir(&other).unwrap();
        let file = other.join("data.csv");
        std::fs::write(&file, "").unwrap();

        let mut rule = make_rule(dir.path().to_str().unwrap(), true, None);
        rule.dir_regex = Some(Regex::new(r"^reports_\d+$").unwrap());
        let events = create_events(Event::Create);

        assert!(!evaluate_rule(&file, &events, None, &rule));
    }
}