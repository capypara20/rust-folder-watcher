use super::*;
use crate::actions::test_support::base_action;
use tempfile::tempdir;

fn sanitize_path(path: &std::path::Path) -> String {
	path.to_str().unwrap().replace('\\', "/")
}

/// collect_action_errors を単体で呼ぶテスト用ラッパー
fn validate_action(action: &ActionConfig, rule_name: &str) -> Result<(), AppError> {
	let mut errors = Vec::new();
	collect_action_errors(action, rule_name, &mut errors);
	finish_validation(errors)
}

// =========================================================
// ヘルパー: バリデーションで watch.path / destination に
// 実在するディレクトリが必要なので、一時ディレクトリのパスを
// テンプレートに埋め込む
// =========================================================

const CMD_ACTION: &str = r#"
	type = "command"
	shell = "cmd"
	command = "echo hi"
	working_dir = ""
"#;

fn rule_toml(name: &str, watch_path: &str, events: &str, action_block: &str) -> String {
	let watch_path = watch_path.replace('\\', "/");
	format!(r#"
		[[rules]]
		enabled = true
		name = "{name}"

		[rules.watch]
		path = "{watch_path}"
		recursive = true
		target = "file"
		include_hidden = false
		patterns = ["*.csv"]
		exclude_patterns = []
		events = [{events}]

		[[rules.actions]]
		{action_block}
	"#)
}

fn make_rules_toml(watch_path: &str, action_block: &str) -> String {
	rule_toml("test-rule", watch_path, r#""create", "modify""#, action_block)
}

fn validate_toml(toml_str: &str) -> Result<(), AppError> {
	let config: RulesConfig = toml::from_str(toml_str).unwrap();
	validate_rules_config(&config)
}

// =========================================================
// GlobalConfig パース
// =========================================================

#[test]
fn test_parse_global_config() {
	let dir = tempdir().unwrap();
	let dir_path = sanitize_path(dir.path());
	let toml_str = format!(r#"
		[retry]
		count = 3
		interval_ms = 1000

		[system_log]
		enabled = true
		dir = "{dir_path}"
		file_name = "system_{{Date}}.log"
		rotation = "daily"
		level = "info"
		console = true
	"#);
	let config: GlobalConfig = toml::from_str(&toml_str).unwrap();
	assert_eq!(config.retry.count, 3);
	assert_eq!(config.retry.interval_ms, 1000);
	assert_eq!(config.system_log.dir, dir_path);
	assert!(config.system_log.console);
}

#[test]
fn test_parse_global_config_log_levels() {
	let dir = tempdir().unwrap();
	let dir_path = sanitize_path(dir.path());
	let make = |level: &str| format!(r#"
		[retry]
		count = 1
		interval_ms = 500

		[system_log]
		dir = "{dir_path}"
		file_name = "system.log"
		rotation = "daily"
		level = "{level}"
	"#);
	for level in ["trace", "debug", "info", "warn", "error"] {
		let result: Result<GlobalConfig, _> = toml::from_str(&make(level));
		assert!(result.is_ok(), "level '{}' のパースに失敗", level);
	}
	let result: Result<GlobalConfig, _> = toml::from_str(&make("verbose"));
	assert!(result.is_err(), "未知の level はエラーになるべき");
}

#[test]
fn test_parse_global_config_rejects_invalid_structure() {
	// system_log セクション自体が無い場合はエラー
	let missing_section = r#"
		[retry]
		count = 1
		interval_ms = 500
	"#.to_string();
	// 旧 [global] 形式は deny_unknown_fields で明示エラーになる
	let legacy_global = r#"
		[global]
		log_level = "info"
		log_dir = "logs"
		log_file_name = "app.log"
		log_rotation = "daily"
		retry_count = 3
		retry_interval_ms = 1000
	"#.to_string();
	// system_log に未知キー（旧 log_to_file 等）があればエラー
	let dir = tempdir().unwrap();
	let unknown_key = format!(r#"
		[retry]
		count = 1
		interval_ms = 500

		[system_log]
		dir = "{}"
		file_name = "system.log"
		rotation = "daily"
		level = "info"
		log_to_file = true
	"#, sanitize_path(dir.path()));

	for (label, toml_str) in [
		("セクション欠落", missing_section),
		("旧 [global] 形式", legacy_global),
		("未知キー", unknown_key),
	] {
		let result: Result<GlobalConfig, _> = toml::from_str(&toml_str);
		assert!(result.is_err(), "{label} はパースエラーになるべき");
	}
}

// =========================================================
// GlobalConfig バリデーション
// =========================================================

fn make_global(dir: &str, file_name: &str) -> GlobalConfig {
	GlobalConfig {
		retry: RetryConfig { count: 3, interval_ms: 1000 },
		system_log: SystemLogConfig {
			enabled: true,
			dir: dir.to_string(),
			file_name: file_name.to_string(),
			rotation: LogRotation::Daily,
			level: LogLevel::Info,
			console: true,
		},
	}
}

#[test]
fn test_validate_global_config() {
	let dir = tempdir().unwrap();
	let dir_path = dir.path().to_str().unwrap();
	assert!(validate_global_config(&make_global(dir_path, "app_{Date}.log")).is_ok());

	let bad = [
		("空の dir", make_global("   ", "app.log")),
		("実在しない dir", make_global("nonexistent_dir_xyz_12345", "app.log")),
		("空の file_name", make_global(dir_path, "  ")),
		("file_name に不正プレースホルダー", make_global(dir_path, "app_{Name}.log")),
	];
	for (label, config) in &bad {
		assert!(validate_global_config(config).is_err(), "{label} はエラーになるべき");
	}
}

// =========================================================
// RulesConfig パース
// =========================================================

#[test]
fn test_parse_rules_config_action_types() {
	let dir = tempdir().unwrap();
	let dest = tempdir().unwrap();
	let watch_path = sanitize_path(dir.path());
	let dest_path = sanitize_path(dest.path());

	let copy_move = |t: &str| format!(r#"
		type = "{t}"
		destination = "{dest_path}"
		overwrite = true
		verify_integrity = true
		preserve_structure = false
	"#);
	let execute_block = r#"
		type = "execute"
		program = "notepad.exe"
		args = ["{FullName}"]
		working_dir = ""
	"#.to_string();

	for block in [copy_move("copy"), copy_move("move"), CMD_ACTION.to_string(), execute_block] {
		let config: RulesConfig = toml::from_str(&make_rules_toml(&watch_path, &block)).unwrap();
		assert_eq!(config.rules.len(), 1, "block: {block}");
		assert_eq!(config.rules[0].name, "test-rule");
	}

	// 未知の action type はパースエラー
	let result: Result<RulesConfig, _> =
		toml::from_str(&make_rules_toml(&watch_path, r#"type = "delete""#));
	assert!(result.is_err());
}

#[test]
fn test_parse_rules_config_regex_and_all_events() {
	let dir = tempdir().unwrap();
	let toml_str = format!(r#"
		[[rules]]
		enabled = false
		name = "regex-rule"

		[rules.watch]
		path = "{}"
		recursive = false
		target = "directory"
		include_hidden = true
		regex = "^report_\\d+\\.csv$"
		exclude_patterns = ["*.tmp"]
		events = ["create", "modify", "delete", "rename"]

		[[rules.actions]]
		type = "command"
		shell = "powershell"
		command = "echo test"
		working_dir = ""
	"#, sanitize_path(dir.path()));
	let config: RulesConfig = toml::from_str(&toml_str).unwrap();
	assert!(config.rules[0].watch.patterns.is_none());
	assert!(config.rules[0].watch.regex.is_some());
	assert_eq!(config.rules[0].watch.events.len(), 4);
}

// =========================================================
// RulesConfig バリデーション: 構造エラー
// =========================================================

#[test]
fn test_validate_rules_structural_errors() {
	assert!(validate_rules_config(&RulesConfig { rules: vec![] }).is_err(), "ルール 0 件はエラー");

	let dir = tempdir().unwrap();
	let path = sanitize_path(dir.path());
	let bad = [
		("空の name", rule_toml("  ", &path, r#""create""#, CMD_ACTION)),
		("空の events", rule_toml("no-events", &path, "", CMD_ACTION)),
		("実在しない watch.path", rule_toml("no-path", "C:/nonexistent_path_xyz_12345", r#""create""#, CMD_ACTION)),
	];
	for (label, toml_str) in &bad {
		assert!(validate_toml(toml_str).is_err(), "{label} はエラーになるべき");
	}

	// actions が空
	let no_actions = format!(r#"
		[[rules]]
		enabled = true
		name = "no-actions"
		actions = []

		[rules.watch]
		path = "{path}"
		recursive = true
		target = "file"
		include_hidden = false
		patterns = ["*.csv"]
		exclude_patterns = []
		events = ["create"]
	"#);
	assert!(validate_toml(&no_actions).is_err(), "actions 空はエラーになるべき");
}

// =========================================================
// RulesConfig バリデーション: watch のフィルタ指定
// (patterns / regex / exclude_* / dir_* の排他・構文チェック)
// =========================================================

#[test]
fn test_validate_watch_filters() {
	let dir = tempdir().unwrap();
	let path = sanitize_path(dir.path());

	// (フィルタ指定, バリデーションが通るか)
	let cases: &[(&str, bool)] = &[
		// patterns / regex はどちらか一方のみ必須
		("patterns = [\"*.csv\"]\nexclude_patterns = []", true),
		("patterns = [\"*.csv\"]\nregex = \"^test\"\nexclude_patterns = []", false),
		("exclude_patterns = []", false),
		// glob / regex 構文チェック
		("patterns = [\"[invalid\"]\nexclude_patterns = []", false),
		("regex = \"(unclosed\"\nexclude_patterns = []", false),
		("patterns = [\"*.csv\"]\nexclude_patterns = [\"[bad\"]", false),
		// exclude_patterns / exclude_regex は排他 (#28)
		("patterns = [\"*.csv\"]\nexclude_patterns = [\"*.tmp\"]\nexclude_regex = \"^debug\"", false),
		("patterns = [\"*.csv\"]\nexclude_patterns = []\nexclude_regex = \"^debug_\\\\d+\"", true),
		("patterns = [\"*.csv\"]\nexclude_patterns = []\nexclude_regex = \"(unclosed\"", false),
		// exclude_dir_patterns / exclude_dir_regex は排他 (#28)
		("patterns = [\"*.csv\"]\nexclude_patterns = []\nexclude_dir_patterns = [\"node_modules\"]\nexclude_dir_regex = \"^\\\\.\"", false),
		("patterns = [\"*.csv\"]\nexclude_patterns = []\nexclude_dir_patterns = [\"node_modules\", \".git\"]", true),
		("patterns = [\"*.csv\"]\nexclude_patterns = []\nexclude_dir_patterns = [\"[bad\"]", false),
		("patterns = [\"*.csv\"]\nexclude_patterns = []\nexclude_dir_regex = \"^\\\\.\"", true),
		("patterns = [\"*.csv\"]\nexclude_patterns = []\nexclude_dir_regex = \"(unclosed\"", false),
		// dir_patterns / dir_regex は排他 (#28)
		("patterns = [\"*.csv\"]\nexclude_patterns = []\ndir_patterns = [\"src\"]\ndir_regex = \"^reports\"", false),
		("patterns = [\"*.csv\"]\nexclude_patterns = []\ndir_patterns = [\"src\", \"reports\"]", true),
		("patterns = [\"*.csv\"]\nexclude_patterns = []\ndir_patterns = [\"[bad\"]", false),
		("patterns = [\"*.csv\"]\nexclude_patterns = []\ndir_regex = \"^reports_\\\\d+\"", true),
		("patterns = [\"*.csv\"]\nexclude_patterns = []\ndir_regex = \"(unclosed\"", false),
	];

	for (filters, expect_ok) in cases {
		let toml_str = format!(r#"
			[[rules]]
			enabled = true
			name = "filter-rule"

			[rules.watch]
			path = "{path}"
			recursive = true
			target = "file"
			include_hidden = false
			events = ["create"]
			{filters}

			[[rules.actions]]
			{CMD_ACTION}
		"#);
		assert_eq!(
			validate_toml(&toml_str).is_ok(),
			*expect_ok,
			"フィルタ指定:\n{filters}"
		);
	}
}

// =========================================================
// validate_action: 各 ActionType の必須フィールド
// =========================================================

fn copy_like_action(type_: ActionType, dest: &str) -> ActionConfig {
	let mut a = base_action(type_);
	a.destination = Some(dest.to_string());
	a.overwrite = Some(true);
	a.verify_integrity = Some(false);
	a.preserve_structure = Some(false);
	a
}

#[test]
fn test_validate_action_copy_move_required_fields() {
	let dest = tempdir().unwrap();
	let dest_path = sanitize_path(dest.path());
	assert!(validate_action(&copy_like_action(ActionType::Copy, &dest_path), "test").is_ok());
	assert!(validate_action(&copy_like_action(ActionType::Move, &dest_path), "test").is_ok());

	// 必須フィールドがひとつでも欠ければエラー
	let clears: &[(&str, fn(&mut ActionConfig))] = &[
		("destination", |a| a.destination = None),
		("overwrite", |a| a.overwrite = None),
		("verify_integrity", |a| a.verify_integrity = None),
		("preserve_structure", |a| a.preserve_structure = None),
	];
	for (label, clear) in clears {
		let mut a = copy_like_action(ActionType::Copy, &dest_path);
		clear(&mut a);
		assert!(validate_action(&a, "test").is_err(), "{label} 欠落はエラーになるべき");
	}
}

#[test]
fn test_validate_action_command_required_fields() {
	let mut valid = base_action(ActionType::Command);
	valid.shell = Some("cmd".to_string());
	valid.command = Some("echo hello".to_string());
	valid.working_dir = Some("".to_string());
	assert!(validate_action(&valid, "test").is_ok());

	let clears: &[(&str, fn(&mut ActionConfig))] = &[
		("shell", |a| a.shell = None),
		("command", |a| a.command = None),
		("working_dir", |a| a.working_dir = None),
	];
	for (label, clear) in clears {
		let mut a = valid.clone();
		clear(&mut a);
		assert!(validate_action(&a, "test").is_err(), "{label} 欠落はエラーになるべき");
	}
}

#[test]
fn test_validate_action_execute_required_fields() {
	let mut valid = base_action(ActionType::Execute);
	valid.program = Some("notepad.exe".to_string());
	valid.args = Some(vec![]);
	valid.working_dir = Some("".to_string());
	assert!(validate_action(&valid, "test").is_ok());

	let clears: &[(&str, fn(&mut ActionConfig))] = &[
		("program", |a| a.program = None),
		("args", |a| a.args = None),
		("working_dir", |a| a.working_dir = None),
	];
	for (label, clear) in clears {
		let mut a = valid.clone();
		clear(&mut a);
		assert!(validate_action(&a, "test").is_err(), "{label} 欠落はエラーになるべき");
	}
}

// =========================================================
// validate_action: destination の実在チェックとプレースホルダー
// =========================================================

#[test]
fn test_validate_action_destination_existence() {
	// 実在しない destination はエラー
	let a = copy_like_action(ActionType::Copy, "C:/nonexistent_dest_xyz_99999");
	assert!(validate_action(&a, "test").is_err());

	// destination 中の {Date} 以降はランタイム展開されるため、ルートだけ実在すれば OK
	let dest_root = tempdir().unwrap();
	let with_placeholder = format!("{}/{{Date}}/sub", sanitize_path(dest_root.path()));
	let a = copy_like_action(ActionType::Copy, &with_placeholder);
	assert!(validate_action(&a, "test").is_ok());

	// プレースホルダー前のルートが存在しなければエラー
	let a = copy_like_action(ActionType::Copy, "C:/nonexistent_root_zzz_99999/{Date}");
	assert!(validate_action(&a, "test").is_err());
}

#[test]
fn test_static_root_of_destination() {
	assert_eq!(static_root_of_destination("C:/data/backup"), "C:/data/backup");
	assert_eq!(static_root_of_destination("C:/data/backup/{Date}"), "C:/data/backup/");
	assert_eq!(static_root_of_destination("C:/data/backup/{Date}/sub"), "C:/data/backup/");
	assert_eq!(static_root_of_destination(r"C:\data\backup\{Date}"), r"C:\data\backup\");
	assert_eq!(static_root_of_destination("{Date}/sub"), "");
}

// =========================================================
// 正常系: 複数ルールを通したバリデーション
// =========================================================

#[test]
fn test_validate_rules_config_multiple_rules() {
	let watch_dir = tempdir().unwrap();
	let dest_dir = tempdir().unwrap();
	let wp = sanitize_path(watch_dir.path());
	let dp = sanitize_path(dest_dir.path());
	let toml_str = format!(r#"
		[[rules]]
		enabled = true
		name = "rule-1"

		[rules.watch]
		path = "{wp}"
		recursive = true
		target = "file"
		include_hidden = false
		patterns = ["*.csv"]
		exclude_patterns = []
		events = ["create"]

		[[rules.actions]]
		type = "copy"
		destination = "{dp}"
		overwrite = true
		verify_integrity = true
		preserve_structure = false

		[[rules]]
		enabled = true
		name = "rule-2"

		[rules.watch]
		path = "{wp}"
		recursive = false
		target = "directory"
		include_hidden = true
		regex = "^backup"
		exclude_patterns = []
		events = ["create", "modify"]

		[[rules.actions]]
		type = "command"
		shell = "cmd"
		command = "echo done"
		working_dir = ""
	"#);
	let config: RulesConfig = toml::from_str(&toml_str).unwrap();
	assert_eq!(config.rules.len(), 2);
	assert!(validate_rules_config(&config).is_ok());
}

// =========================================================
// テンプレート TOML パーステスト
// Windows バックスラッシュパスが TOML として正しくパースできること
// =========================================================

#[test]
fn test_global_template_is_valid_toml() {
	use crate::templates::GLOBAL_TOML;
	let val: toml::Value = toml::from_str(GLOBAL_TOML)
		.expect("GLOBAL_TOML テンプレートが TOML としてパースできません");
	let dir = val["system_log"]["dir"].as_str().unwrap();
	assert_eq!(dir, r"C:\logs", "system_log.dir のパスが正しく保持されていません");
}

#[test]
fn test_rules_template_is_valid_toml() {
	use crate::templates::RULES_TOML;
	let val: toml::Value = toml::from_str(RULES_TOML)
		.expect("RULES_TOML テンプレートが TOML としてパースできません");
	let path = val["rules"][0]["watch"]["path"].as_str().unwrap();
	assert_eq!(path, r"C:\監視フォルダ", "rules[0].watch.path のパスが正しく保持されていません");
	let detect_dir = val["rules"][0]["log"]["detect"]["dir"].as_str().unwrap();
	assert_eq!(detect_dir, r"C:\logs", "rules[0].log.detect.dir のパスが正しく保持されていません");
	let action_dir = val["rules"][0]["log"]["action"]["dir"].as_str().unwrap();
	assert_eq!(action_dir, r"C:\logs", "rules[0].log.action.dir のパスが正しく保持されていません");
}
