use super::*;
use crate::test_support::base_action;
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

/// テストで使うシェル名。`shell` はロード時に「この OS で起動できる値か」を
/// 検証するようになったため、Windows と Linux で使い分ける必要がある。
#[cfg(windows)]
const TEST_SHELL: &str = "cmd";
#[cfg(not(windows))]
const TEST_SHELL: &str = "bash";

fn cmd_action() -> String {
	format!(
		r#"
	type = "command"
	shell = "{TEST_SHELL}"
	command = "echo hi"
	working_dir = ""
"#
	)
}

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
	// [service] セクション省略時はログオンユーザー権限実行が既定 ON。
	assert!(config.service.is_none());
	assert!(config.run_as_logged_in_user());
}

#[test]
fn test_parse_service_section() {
	let dir = tempdir().unwrap();
	let dir_path = sanitize_path(dir.path());
	let make = |body: &str| format!(r#"
		[retry]
		count = 1
		interval_ms = 500

		[system_log]
		dir = "{dir_path}"
		file_name = "system.log"
		rotation = "daily"
		level = "info"

		{body}
	"#);

	// run_as_logged_in_user を明示的に false にできる。
	let off: GlobalConfig = toml::from_str(&make("[service]\nrun_as_logged_in_user = false")).unwrap();
	assert!(!off.run_as_logged_in_user());

	// [service] はあるが値を省略すると既定 true。
	let default_on: GlobalConfig = toml::from_str(&make("[service]")).unwrap();
	assert!(default_on.run_as_logged_in_user());

	// 未知キーは deny_unknown_fields で弾く。
	let unknown: Result<GlobalConfig, _> =
		toml::from_str(&make("[service]\nrun_as_admin = true"));
	assert!(unknown.is_err());
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
		dashboard: None,
		startup_scan: None,
		destination: None,
		detect: None,
		service: None,
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
// DashboardConfig パース / バリデーション
// =========================================================

#[test]
fn test_parse_dashboard_config() {
	let dir = tempdir().unwrap();
	let dir_path = sanitize_path(dir.path());
	let toml_str = format!(r#"
		[retry]
		count = 1
		interval_ms = 500

		[system_log]
		dir = "{dir_path}"
		file_name = "system.log"
		rotation = "daily"
		level = "info"

		[dashboard]
		enabled = true
		bind = "127.0.0.1:9000"
		history = 50
	"#);
	let config: GlobalConfig = toml::from_str(&toml_str).unwrap();
	let dash = config.dashboard.expect("dashboard セクションがパースされていない");
	assert!(dash.enabled);
	assert_eq!(dash.bind, "127.0.0.1:9000");
	assert_eq!(dash.history, 50);
}

#[test]
fn test_dashboard_absent_is_none_and_defaults_apply() {
	let dir = tempdir().unwrap();
	let dir_path = sanitize_path(dir.path());
	let base = format!(r#"
		[retry]
		count = 1
		interval_ms = 500

		[system_log]
		dir = "{dir_path}"
		file_name = "system.log"
		rotation = "daily"
		level = "info"
	"#);

	// セクションが無ければ None。
	let config: GlobalConfig = toml::from_str(&base).unwrap();
	assert!(config.dashboard.is_none());

	// bind / history 省略時は既定値（127.0.0.1:8080 / 200）。
	let with_defaults = format!("{base}\n[dashboard]\nenabled = true\n");
	let config: GlobalConfig = toml::from_str(&with_defaults).unwrap();
	let dash = config.dashboard.unwrap();
	assert!(dash.enabled);
	assert_eq!(dash.bind, "127.0.0.1:8080");
	assert_eq!(dash.history, 200);
}

#[test]
fn test_validate_dashboard_bind() {
	let dir = tempdir().unwrap();
	let dir_path = dir.path().to_str().unwrap();
	let mut cfg = make_global(dir_path, "app.log");

	// enabled かつ bind が不正ならエラー。
	cfg.dashboard = Some(DashboardConfig {
		enabled: true,
		bind: "not-an-address".to_string(),
		history: 10,
	});
	assert!(validate_global_config(&cfg).is_err(), "不正な bind はエラーになるべき");

	// 正しい bind は通る。
	cfg.dashboard = Some(DashboardConfig {
		enabled: true,
		bind: "127.0.0.1:8080".to_string(),
		history: 10,
	});
	assert!(validate_global_config(&cfg).is_ok(), "正しい bind は通るべき");

	// enabled=false なら bind が不正でも検証しない。
	cfg.dashboard = Some(DashboardConfig {
		enabled: false,
		bind: "garbage".to_string(),
		history: 10,
	});
	assert!(validate_global_config(&cfg).is_ok(), "無効時は bind を検証しない");
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

	for block in [copy_move("copy"), copy_move("move"), cmd_action(), execute_block] {
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
		("空の name", rule_toml("  ", &path, r#""create""#, &cmd_action())),
		("空の events", rule_toml("no-events", &path, "", &cmd_action())),
		("実在しない watch.path", rule_toml("no-path", "/nonexistent_path_xyz_12345/deeper", r#""create""#, &cmd_action())),
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
			{action}
		"#, action = cmd_action());
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
	valid.shell = Some(TEST_SHELL.to_string());
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

/// テスト用に auto_create を明示した copy アクションを作る。
fn copy_action_with_auto_create(dest: &str, auto_create: bool) -> ActionConfig {
	let mut a = copy_like_action(ActionType::Copy, dest);
	a.auto_create = Some(auto_create);
	a
}

// auto_create = true（既定）: 宛先フォルダが無いだけならロードを通す (#68)。
// 実行時に create_dir_all で掘られるため、事前に手で作らせる必要はない。
#[test]
fn test_destination_missing_dir_is_allowed_when_auto_create() {
	let existing_root = tempdir().unwrap();
	let root = sanitize_path(existing_root.path());

	// 固定パス（プレースホルダーなし）でも、まだ存在しないだけなら OK
	let a = copy_action_with_auto_create(&format!("{root}/not_yet/deeper"), true);
	assert!(validate_action(&a, "test").is_ok());

	// プレースホルダーありも同様
	let a = copy_action_with_auto_create(&format!("{root}/{{Date}}/sub"), true);
	assert!(validate_action(&a, "test").is_ok());

	// 先頭からプレースホルダーで始まる場合は静的部分が無いので判定しない
	let a = copy_action_with_auto_create("{WatchPath}/archive", true);
	assert!(validate_action(&a, "test").is_ok());
}

// auto_create = true でも、先祖をたどって 1 つも実在しない宛先はエラー。
// 未接続のドライブレターや綴り間違いの共有名を拾うための最低限のチェック。
#[cfg(windows)]
#[test]
fn test_destination_without_existing_drive_is_error() {
	let a = copy_action_with_auto_create(r"Q:\backup\{Date}", true);
	assert!(validate_action(&a, "test").is_err());
}

// Unix では絶対パスの根（`/`）が必ず実在するため、このチェックは発火しない。
// 実行時に create_dir_all で作られるので、ロードは通してよい。
// （このチェックが意味を持つのは、根そのものが存在しないことがある
//   Windows のドライブレターや UNC 共有）
#[cfg(not(windows))]
#[test]
fn test_destination_absolute_path_passes_on_unix() {
	let a = copy_action_with_auto_create("/nonexistent_root_zzz_99999/backup/{Date}", true);
	assert!(validate_action(&a, "test").is_ok());
}

// auto_create = false: 従来どおり静的部分の実在を要求する。
#[test]
fn test_destination_existence_required_when_auto_create_disabled() {
	let dest_root = tempdir().unwrap();
	let root = sanitize_path(dest_root.path());

	// 実在するフォルダなら OK
	assert!(validate_action(&copy_action_with_auto_create(&root, false), "test").is_ok());
	// {Date} の親が実在すれば OK
	let a = copy_action_with_auto_create(&format!("{root}/{{Date}}/sub"), false);
	assert!(validate_action(&a, "test").is_ok());

	// まだ存在しない固定パスはエラー
	let a = copy_action_with_auto_create(&format!("{root}/not_yet"), false);
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
		shell = "{TEST_SHELL}"
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
