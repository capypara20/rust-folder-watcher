use std::path::Path;
use regex::Regex;
use serde::{Deserialize, Deserializer};
use globset::Glob;

use crate::error::AppError;
use crate::placeholder::validate_placeholders;

macro_rules! impl_case_insensitive_deserialize {
    ($type:ident, $($variant:ident => $s:literal),+ $(,)?) => {
        impl<'de> Deserialize<'de> for $type {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = String::deserialize(d)?;
                match s.to_lowercase().as_str() {
                    $($s => Ok($type::$variant),)+
                    _ => Err(serde::de::Error::custom(format!("unknown value: {}", s))),
                }
            }
        }
    };
}

#[derive(Debug, Clone)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl_case_insensitive_deserialize!(LogLevel,
    Trace => "trace",
    Debug => "debug",
    Info  => "info",
    Warn  => "warn",
    Error => "error",
);

#[derive(Debug, Clone)]
pub enum LogRotation {
	Daily,
	Never,
}

impl_case_insensitive_deserialize!(LogRotation,
    Daily => "daily",
    Never => "never",
);

#[derive(Debug, Clone)]
pub enum WatchTarget {
    File,
    Directory,
    Both,
}

impl_case_insensitive_deserialize!(WatchTarget,
    File      => "file",
    Directory => "directory",
    Both      => "both",
);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Event {
    Create,
    Modify,
    Delete,
    Rename,
}

impl_case_insensitive_deserialize!(Event,
    Create => "create",
    Modify => "modify",
    Delete => "delete",
    Rename => "rename",
);

#[derive(Debug, Clone)]
pub enum ActionType {
    Copy,
    Move,
    Command,
    Execute,
    Log,
}

impl_case_insensitive_deserialize!(ActionType,
    Copy    => "copy",
    Move    => "move",
    Command => "command",
    Execute => "execute",
    Log     => "log",
);

#[derive(Debug, Clone, Deserialize)]
pub struct GlobalConfig {
    pub global: Global,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RulesConfig {
    pub rules: Vec<Rule>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
pub struct Global {
    pub log_level: LogLevel,
    pub log_dir: String,
    pub log_file_name: String,
    pub log_rotation: LogRotation,
    pub retry_count: u32,
    pub retry_interval_ms: u64,
    #[serde(default = "default_true")]
    pub log_to_console: bool,
    #[serde(default = "default_true")]
    pub log_to_file: bool,
    /// ターミナル出力専用ログレベル。省略時は log_level を使用。
    pub terminal_log_level: Option<LogLevel>,
    /// ファイル出力専用ログレベル。省略時は log_level を使用。
    pub file_log_level: Option<LogLevel>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RuleLog {
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub log_dir: Option<String>,
    pub log_file_name: Option<String>,
    pub log_rotation: Option<LogRotation>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Rule {
    pub enabled: bool,
    pub name: String,
    pub watch: Watch,
    pub actions: Vec<ActionConfig>,
    pub log: Option<RuleLog>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Watch {
    pub path: String,
    pub recursive: bool,
    pub target: WatchTarget,
    pub include_hidden: bool,
    pub patterns: Option<Vec<String>>,
    pub regex: Option<String>,
    pub exclude_patterns: Vec<String>,
    #[serde(default)]
    pub exclude_regex: Option<String>,
    #[serde(default)]
    pub exclude_dir_patterns: Vec<String>,
    #[serde(default)]
    pub exclude_dir_regex: Option<String>,
    #[serde(default)]
    pub dir_patterns: Vec<String>,
    #[serde(default)]
    pub dir_regex: Option<String>,
    pub events: Vec<Event>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ActionConfig {
    #[serde(rename = "type")]
    pub type_: ActionType,

    // typeがCopy / Move のとき
    pub destination: Option<String>,
    pub overwrite: Option<bool>,
    pub verify_integrity: Option<bool>,
    pub preserve_structure: Option<bool>,

    // typeがCommand / Executeのとき
    pub working_dir: Option<String>,

    // typeがCommandのとき
    pub shell: Option<String>,
    pub command: Option<String>,

    // typeがExecuteのとき
    pub program: Option<String>,
    pub args: Option<Vec<String>>,

    // typeがLogのとき
    pub message: Option<String>,
}

fn expand_tilde(s: &str) -> String {
	if s == "~" || s.starts_with("~/") || s.starts_with("~\\") {
		let home = std::env::var_os("HOME")
			.or_else(|| std::env::var_os("USERPROFILE"))
			.map(|h| h.to_string_lossy().into_owned())
			.unwrap_or_default();
		return format!("{}{}", home, &s[1..]);
	}
	s.to_string()
}

pub fn load_global_config(path: &Path) -> Result<GlobalConfig, AppError> {
	let content = std::fs::read_to_string(path)?;
	let mut config: GlobalConfig = toml::from_str(&content)
							.map_err(|e| AppError::TomlParse(e.to_string()))?;
	config.global.log_dir = expand_tilde(&config.global.log_dir);
	Ok(config)
}

pub fn load_rules_config(path: &Path) -> Result<RulesConfig, AppError> {
	let content = std::fs::read_to_string(path)?;
	let mut config: RulesConfig = toml::from_str(&content)
							.map_err(|e| AppError::TomlParse(e.to_string()))?;
	for rule in &mut config.rules {
		rule.watch.path = expand_tilde(&rule.watch.path);
		for action in &mut rule.actions {
			action.destination = action.destination.as_deref().map(expand_tilde);
			action.working_dir = action.working_dir.as_deref().map(expand_tilde);
		}
		if let Some(log) = &mut rule.log {
			log.log_dir = log.log_dir.as_deref().map(expand_tilde);
		}
	}
	Ok(config)
}

fn finish_validation(errors: Vec<String>) -> Result<(), AppError> {
	if errors.is_empty() {
		return Ok(());
	}
	if errors.len() == 1 {
		return Err(AppError::Validation(errors.into_iter().next().unwrap()));
	}
	let mut msg = format!("バリデーションエラーが {} 件見つかりました:\n", errors.len());
	for (i, e) in errors.iter().enumerate() {
		msg.push_str(&format!("  [{}] {}\n", i + 1, e));
	}
	Err(AppError::Validation(msg.trim_end().to_string()))
}

pub fn validate_global_config(config: &GlobalConfig) -> Result<(), AppError> {
	let mut errors = Vec::new();

	let log_dir = &config.global.log_dir;
	if log_dir.trim().is_empty() {
		errors.push("log_dir が空文字列です。ログ出力先ディレクトリを定義してください".to_string());
	} else {
		let dir_path = Path::new(log_dir);
		if !dir_path.exists() {
			errors.push(format!("log_dir が存在しません: {}", dir_path.display()));
		} else if !dir_path.is_dir() {
			errors.push(format!("log_dir にディレクトリ以外のパスが指定されています: {}", dir_path.display()));
		}
	}

	let log_file_name = &config.global.log_file_name;
	if log_file_name.trim().is_empty() {
		errors.push("log_file_name が空文字列です。ファイル名を定義してください".to_string());
	} else {
		let valid_placeholders = ["Date", "DateTime"];
		let re = regex::Regex::new(r"\{([A-Za-z]+)\}").unwrap();
		for caps in re.captures_iter(log_file_name) {
			let name = &caps[1];
			if !valid_placeholders.contains(&name) {
				errors.push(format!(
					"log_file_name に使用できないプレースホルダーがあります: {{{name}}}。使用可能なのは {{Date}} と {{DateTime}} のみです"
				));
			}
		}
	}

	finish_validation(errors)
}

pub fn validate_rules_config(config: &RulesConfig) -> Result<(), AppError> {
	let mut errors = Vec::new();
	let rules = &config.rules;

	if rules.is_empty() {
		errors.push("ルールが1つも定義されていません。少なくとも1つのルールを定義してください".to_string());
		return finish_validation(errors);
	}

	for (index, rule) in rules.iter().enumerate() {
		let rule_id = if rule.name.trim().is_empty() {
			format!("{}番目のルール(name未設定)", index + 1)
		} else {
			rule.name.clone()
		};

		if rule.name.trim().is_empty() {
			errors.push(format!("{} 番目の name が空文字列です。ルールにわかりやすい名前を定義してください", index + 1));
		}
		if rule.actions.is_empty() {
			errors.push(format!("監視ルール名 {} の actions(処理) が1つも定義されていません。少なくとも1つのアクションを定義してください", rule_id));
		}
		if rule.watch.events.is_empty() {
			errors.push(format!("監視ルール名 {} の watch.events(検知イベント) が1つも定義されていません。少なくとも1つのイベントを定義してください", rule_id));
		}
		if (rule.watch.patterns.is_some() && rule.watch.regex.is_some()) || (rule.watch.patterns.is_none() && rule.watch.regex.is_none()) {
			errors.push(format!("監視ルール名 {} の watch.patterns と watch.regex は片方のみ定義できます。どちらか一方を定義してください", rule_id));
		}

		for action in &rule.actions {
			collect_action_errors(action, &rule_id, &mut errors);
			collect_action_placeholder_errors(action, &rule_id, &mut errors);
		}

		let watch_path = Path::new(&rule.watch.path);
		if !watch_path.is_dir() {
			errors.push(format!("監視ルール名 {} の watch.path が存在しません: {}", rule_id, watch_path.display()));
		}

		if let Some(patterns) = &rule.watch.patterns {
			for pt in patterns {
				if let Err(e) = Glob::new(pt) {
					errors.push(format!("監視ルール名 {} の patterns に無効な glob があります '{}': {}", rule_id, pt, e));
				}
			}
		}

		if let Some(regex_str) = &rule.watch.regex {
			if let Err(e) = Regex::new(regex_str) {
				errors.push(format!("監視ルール名 {} の regex に無効な正規表現があります '{}': {}", rule_id, regex_str, e));
			}
		}

		for glob in &rule.watch.exclude_patterns {
			if let Err(e) = Glob::new(glob) {
				errors.push(format!("監視ルール名 {} の exclude_patterns に無効な glob があります '{}': {}", rule_id, glob, e));
			}
		}

		if !rule.watch.exclude_patterns.is_empty() && rule.watch.exclude_regex.is_some() {
			errors.push(format!("監視ルール名 {} の exclude_patterns と exclude_regex は片方のみ定義できます", rule_id));
		}

		if let Some(re_str) = &rule.watch.exclude_regex {
			if let Err(e) = Regex::new(re_str) {
				errors.push(format!("監視ルール名 {} の exclude_regex に無効な正規表現があります '{}': {}", rule_id, re_str, e));
			}
		}

		if !rule.watch.exclude_dir_patterns.is_empty() && rule.watch.exclude_dir_regex.is_some() {
			errors.push(format!("監視ルール名 {} の exclude_dir_patterns と exclude_dir_regex は片方のみ定義できます", rule_id));
		}

		for glob in &rule.watch.exclude_dir_patterns {
			if let Err(e) = Glob::new(glob) {
				errors.push(format!("監視ルール名 {} の exclude_dir_patterns に無効な glob があります '{}': {}", rule_id, glob, e));
			}
		}

		if let Some(re_str) = &rule.watch.exclude_dir_regex {
			if let Err(e) = Regex::new(re_str) {
				errors.push(format!("監視ルール名 {} の exclude_dir_regex に無効な正規表現があります '{}': {}", rule_id, re_str, e));
			}
		}

		if !rule.watch.dir_patterns.is_empty() && rule.watch.dir_regex.is_some() {
			errors.push(format!("監視ルール名 {} の dir_patterns と dir_regex は片方のみ定義できます", rule_id));
		}

		for glob in &rule.watch.dir_patterns {
			if let Err(e) = Glob::new(glob) {
				errors.push(format!("監視ルール名 {} の dir_patterns に無効な glob があります '{}': {}", rule_id, glob, e));
			}
		}

		if let Some(re_str) = &rule.watch.dir_regex {
			if let Err(e) = Regex::new(re_str) {
				errors.push(format!("監視ルール名 {} の dir_regex に無効な正規表現があります '{}': {}", rule_id, re_str, e));
			}
		}

		if let Some(rule_log) = &rule.log {
			if rule_log.enabled {
				if let Some(dir) = &rule_log.log_dir {
					let p = Path::new(dir);
					if !p.exists() {
						errors.push(format!("監視ルール名 {} の log.log_dir が存在しません: {}", rule_id, dir));
					} else if !p.is_dir() {
						errors.push(format!("監視ルール名 {} の log.log_dir にディレクトリ以外のパスが指定されています: {}", rule_id, dir));
					}
				}
				if let Some(file_name) = &rule_log.log_file_name {
					if file_name.trim().is_empty() {
						errors.push(format!("監視ルール名 {} の log.log_file_name が空文字列です", rule_id));
					} else {
						let valid_placeholders = ["Date", "DateTime"];
						let re = regex::Regex::new(r"\{([A-Za-z]+)\}").unwrap();
						for caps in re.captures_iter(file_name) {
							let name = &caps[1];
							if !valid_placeholders.contains(&name) {
								errors.push(format!(
									"監視ルール名 {} の log.log_file_name に使用できないプレースホルダーがあります: {{{name}}}",
									rule_id
								));
							}
						}
					}
				}
			}
		}
	}

	finish_validation(errors)
}

/// destination 文字列から、最初のプレースホルダー（`{`）より前の静的部分を取り出し、
/// さらに最後の `/` または `\` までの部分（=ディレクトリのルート）を返す。
/// プレースホルダーが含まれない場合は文字列全体をそのまま返す。
///
/// 例:
///   "C:/data/backup/{Date}/sub" → "C:/data/backup/"
///   "C:/data/backup/{Date}"     → "C:/data/backup/"
///   "C:/data/backup"            → "C:/data/backup"
///   "{Date}"                    → "" (空文字列 = 不正扱い)
fn static_root_of_destination(dest: &str) -> &str {
	let static_part = match dest.find('{') {
		Some(idx) => {
			// "{{" は次段で expand_placeholders がリテラル '{' に変換するので静的扱いできるが、
			// シンプルにするため最初の '{' で切る。
			let last_sep = dest[..idx].rfind(['/', '\\']);
			match last_sep {
				Some(sep_idx) => &dest[..=sep_idx],
				None => "",
			}
		}
		None => dest,
	};
	static_part
}

fn collect_action_errors(action: &ActionConfig, rule_name: &str, errors: &mut Vec<String>) {
	match action.type_ {
		ActionType::Copy | ActionType::Move => {
			if action.destination.is_none() {
				errors.push(format!("監視ルール名 {} のアクションの type が Copy / Move のとき、destination(コピー先/移動先) を定義してください", rule_name));
			}
			if action.overwrite.is_none() {
				errors.push(format!("監視ルール名 {} のアクションの type が Copy / Move のとき、overwrite(上書きの有無) を定義してください", rule_name));
			}
			if action.preserve_structure.is_none() {
				errors.push(format!("監視ルール名 {} のアクションの type が Copy / Move のとき、preserve_structure(ディレクトリ構造を保持するか) を定義してください", rule_name));
			}
			if action.verify_integrity.is_none() {
				errors.push(format!("監視ルール名 {} のアクションの type が Copy / Move のとき、verify_integrity(コピー後にファイルの完全性を検証するか) を定義してください", rule_name));
			}
			if let Some(dest) = &action.destination {
				let static_root = static_root_of_destination(dest);
				if !Path::new(static_root).is_dir() {
					errors.push(format!(
						"監視ルール名 {} のアクションの destination(コピー先/移動先) のルート '{}' が存在しません",
						rule_name, static_root
					));
				}
			}
		}

		ActionType::Command => {
			if action.shell.is_none() {
				errors.push(format!("監視ルール名 {} のアクションの type が Command のとき、shell(コマンドを実行するシェル) を定義してください", rule_name));
			}
			if action.command.is_none() {
				errors.push(format!("監視ルール名 {} のアクションの type が Command のとき、command(実行するコマンド) を定義してください", rule_name));
			}
			if action.working_dir.is_none() {
				errors.push(format!("監視ルール名 {} のアクションの type が Command のとき、working_dir(コマンド/プログラムを実行するディレクトリ) を定義してください", rule_name));
			}
			if let Some(dir) = &action.working_dir {
				if !dir.is_empty() && !Path::new(dir).is_dir() {
					errors.push(format!("監視ルール名 {} のアクションの working_dir が存在しません: {}", rule_name, dir));
				}
			}
		}

		ActionType::Execute => {
			if action.program.is_none() {
				errors.push(format!("監視ルール名 {} のアクションの type が Execute のとき、program(実行するプログラム) を定義してください", rule_name));
			}
			if action.args.is_none() {
				errors.push(format!("監視ルール名 {} のアクションの type が Execute のとき、args(プログラムに渡す引数) を定義してください。引数がない場合は空の配列を指定してください", rule_name));
			}
			if action.working_dir.is_none() {
				errors.push(format!("監視ルール名 {} のアクションの type が Execute のとき、working_dir(コマンド/プログラムを実行するディレクトリ) を定義してください", rule_name));
			}
			if let Some(dir) = &action.working_dir {
				if !dir.is_empty() && !Path::new(dir).is_dir() {
					errors.push(format!("監視ルール名 {} のアクションの working_dir が存在しません: {}", rule_name, dir));
				}
			}
			if let Some(program) = &action.program {
				let p = Path::new(program);
				if p.is_absolute() && !p.exists() {
					errors.push(format!("監視ルール名 {} のアクションの program が存在しません: {}", rule_name, program));
				}
			}
		}

		ActionType::Log => {
			if action.message.is_none() {
				errors.push(format!("監視ルール名 {} のアクションの type が Log のとき、message(出力するメッセージ) を定義してください", rule_name));
			}
		}
	}
}

fn collect_action_placeholder_errors(action: &ActionConfig, rule_name: &str, errors: &mut Vec<String>) {
	let fields = [
		("action.destination", &action.destination),
		("action.command", &action.command),
		("action.working_dir", &action.working_dir),
		("action.program", &action.program),
		("action.message", &action.message),
	];
	for (field_name, field_value) in fields {
		if let Some(value) = field_value {
			if let Err(e) = validate_placeholders(value, rule_name, field_name) {
				errors.push(e.to_string());
			}
		}
	}
	if let Some(args) = &action.args {
		for (index, arg) in args.iter().enumerate() {
			if let Err(e) = validate_placeholders(arg, rule_name, &format!("action.args[{}]", index)) {
				errors.push(e.to_string());
			}
		}
	}
}

#[cfg(test)]
fn validate_action(action: &ActionConfig, rule_name: &str) -> Result<(), AppError> {
	let mut errors = Vec::new();
	collect_action_errors(action, rule_name, &mut errors);
	finish_validation(errors)
}

#[cfg(test)]
fn validate_action_placeholders(action: &ActionConfig, rule_name: &str) -> Result<(), AppError> {
	let mut errors = Vec::new();
	collect_action_placeholder_errors(action, rule_name, &mut errors);
	finish_validation(errors)
}

#[cfg(test)]
mod tests {
	use super::*;
	use tempfile::tempdir;

	fn sanitize_path(path: &std::path::Path) -> String {
		path.to_str().unwrap().replace('\\', "/")
	}

	// =========================================================
	// ヘルパー: バリデーションで watch.path / destination に
	// 実在するディレクトリが必要なので、一時ディレクトリのパスを
	// テンプレートに埋め込む
	// =========================================================

	fn make_rules_toml(watch_path: &str, action_block: &str) -> String {
		let watch_path = watch_path.replace('\\', "/");
		format!(r#"
			[[rules]]
			enabled = true
			name = "test-rule"

			[rules.watch]
			path = "{watch_path}"
			recursive = true
			target = "file"
			include_hidden = false
			patterns = ["*.csv"]
			exclude_patterns = []
			events = ["create", "modify"]

			[[rules.actions]]
			{action_block}
		"#)
	}

	fn make_rules_toml_with_watch(watch_block: &str, watch_path: &str) -> String {
		let watch_path = watch_path.replace('\\', "/");
		format!(r#"
			[[rules]]
			enabled = true
			name = "test-rule"

			[rules.watch]
			path = "{watch_path}"
			{watch_block}

			[[rules.actions]]
			type = "command"
			shell = "cmd"
			command = "echo hello"
			working_dir = ""
		"#)
	}

	// =========================================================
	// GlobalConfig パーステスト
	// =========================================================

	#[test]
	fn test_parse_global_config() {
		let dir = tempdir().unwrap();
		let dir_path = sanitize_path(dir.path());
		let toml_str = format!(r#"
			[global]
			log_level = "info"
			log_dir = "{dir_path}"
			log_file_name = "app_{{Date}}.log"
			log_rotation = "daily"
			retry_count = 3
			retry_interval_ms = 1000
			dry_run = false
		"#);
		let config: GlobalConfig = toml::from_str(&toml_str).unwrap();
		assert_eq!(config.global.retry_count, 3);
		assert_eq!(config.global.retry_interval_ms, 1000);
		assert_eq!(config.global.log_dir, dir_path);
	}

	#[test]
	fn test_parse_global_config_all_log_levels() {
		let dir = tempdir().unwrap();
		let dir_path = sanitize_path(dir.path());
		for level in &["trace", "debug", "info", "warn", "error"] {
			let toml_str = format!(r#"
				[global]
				log_level = "{level}"
				log_dir = "{dir_path}"
				log_file_name = "app.log"
				log_rotation = "daily"
				retry_count = 1
				retry_interval_ms = 500
				dry_run = false
			"#);
			let result: Result<GlobalConfig, _> = toml::from_str(&toml_str);
			assert!(result.is_ok(), "log_level '{}' のパースに失敗", level);
		}
	}

	#[test]
	fn test_parse_global_config_invalid_log_level() {
		let dir = tempdir().unwrap();
		let dir_path = sanitize_path(dir.path());
		let toml_str = format!(r#"
			[global]
			log_level = "verbose"
			log_dir = "{dir_path}"
			log_file_name = "app.log"
			log_rotation = "daily"
			retry_count = 1
			retry_interval_ms = 500
			dry_run = false
		"#);
		let result: Result<GlobalConfig, _> = toml::from_str(&toml_str);
		assert!(result.is_err());
	}

	#[test]
	fn test_parse_global_config_missing_field() {
		let toml_str = r#"
			[global]
			log_level = "info"
			log_dir = "logs"
		"#;
		let result: Result<GlobalConfig, _> = toml::from_str(toml_str);
		assert!(result.is_err());
	}

	// =========================================================
	// GlobalConfig バリデーションテスト
	// =========================================================

	#[test]
	fn test_validate_global_config_empty_log_dir() {
		let config = GlobalConfig {
			global: Global {
				log_level: LogLevel::Info,
				log_dir: "   ".to_string(),
				log_file_name: "app.log".to_string(),
				log_rotation: LogRotation::Daily,
				retry_count: 3,
				retry_interval_ms: 1000,
				log_to_console: true,
				log_to_file: true,
				terminal_log_level: None,
				file_log_level: None,
			},
		};
		let result = validate_global_config(&config);
		assert!(result.is_err());
	}

	#[test]
	fn test_validate_global_config_dir_not_exist() {
		let config = GlobalConfig {
			global: Global {
				log_level: LogLevel::Info,
				log_dir: "nonexistent_dir_xyz_12345".to_string(),
				log_file_name: "app.log".to_string(),
				log_rotation: LogRotation::Daily,
				retry_count: 3,
				retry_interval_ms: 1000,
				log_to_console: true,
				log_to_file: true,
				terminal_log_level: None,
				file_log_level: None,
			},
		};
		let result = validate_global_config(&config);
		assert!(result.is_err());
	}

	#[test]
	fn test_validate_global_config_empty_file_name() {
		let dir = tempdir().unwrap();
		let config = GlobalConfig {
			global: Global {
				log_level: LogLevel::Info,
				log_dir: dir.path().to_str().unwrap().to_string(),
				log_file_name: "  ".to_string(),
				log_rotation: LogRotation::Daily,
				retry_count: 3,
				retry_interval_ms: 1000,
				log_to_console: true,
				log_to_file: true,
				terminal_log_level: None,
				file_log_level: None,
			},
		};
		let result = validate_global_config(&config);
		assert!(result.is_err());
	}

	#[test]
	fn test_validate_global_config_invalid_placeholder_in_file_name() {
		let dir = tempdir().unwrap();
		let config = GlobalConfig {
			global: Global {
				log_level: LogLevel::Info,
				log_dir: dir.path().to_str().unwrap().to_string(),
				log_file_name: "app_{Name}.log".to_string(),
				log_rotation: LogRotation::Daily,
				retry_count: 3,
				retry_interval_ms: 1000,
				log_to_console: true,
				log_to_file: true,
				terminal_log_level: None,
				file_log_level: None,
			},
		};
		let result = validate_global_config(&config);
		assert!(result.is_err());
	}

	#[test]
	fn test_validate_global_config_valid() {
		let dir = tempdir().unwrap();
		let config = GlobalConfig {
			global: Global {
				log_level: LogLevel::Info,
				log_dir: dir.path().to_str().unwrap().to_string(),
				log_file_name: "app_{Date}.log".to_string(),
				log_rotation: LogRotation::Daily,
				retry_count: 3,
				retry_interval_ms: 1000,
				log_to_console: true,
				log_to_file: true,
				terminal_log_level: None,
				file_log_level: None,
			},
		};
		let result = validate_global_config(&config);
		assert!(result.is_ok());
	}

	// =========================================================
	// RulesConfig パーステスト
	// =========================================================

	#[test]
	fn test_parse_rules_config_copy_action() {
		let dir = tempdir().unwrap();
		let dest = tempdir().unwrap();
		let toml_str = make_rules_toml(
			&sanitize_path(dir.path()),
			&format!(r#"
				type = "copy"
				destination = "{}"
				overwrite = true
				verify_integrity = true
				preserve_structure = false
			"#, sanitize_path(dest.path())),
		);
		let config: RulesConfig = toml::from_str(&toml_str).unwrap();
		assert_eq!(config.rules.len(), 1);
		assert_eq!(config.rules[0].name, "test-rule");
	}

	#[test]
	fn test_parse_rules_config_move_action() {
		let dir = tempdir().unwrap();
		let dest = tempdir().unwrap();
		let toml_str = make_rules_toml(
			&sanitize_path(dir.path()),
			&format!(r#"
				type = "move"
				destination = "{}"
				overwrite = false
				verify_integrity = false
				preserve_structure = true
			"#, sanitize_path(dest.path())),
		);
		let config: RulesConfig = toml::from_str(&toml_str).unwrap();
		assert_eq!(config.rules[0].actions[0].overwrite, Some(false));
	}

	#[test]
	fn test_parse_rules_config_command_action() {
		let dir = tempdir().unwrap();
		let toml_str = make_rules_toml(
			&sanitize_path(dir.path()),
			r#"
				type = "command"
				shell = "cmd"
				command = "echo hello"
				working_dir = ""
			"#,
		);
		let config: RulesConfig = toml::from_str(&toml_str).unwrap();
		assert_eq!(config.rules[0].actions[0].command, Some("echo hello".to_string()));
	}

	#[test]
	fn test_parse_rules_config_execute_action() {
		let dir = tempdir().unwrap();
		let toml_str = make_rules_toml(
			&sanitize_path(dir.path()),
			r#"
				type = "execute"
				program = "notepad.exe"
				args = ["{FullName}"]
				working_dir = ""
			"#,
		);
		let config: RulesConfig = toml::from_str(&toml_str).unwrap();
		assert_eq!(config.rules[0].actions[0].program, Some("notepad.exe".to_string()));
	}

	#[test]
	fn test_parse_rules_config_with_regex_instead_of_patterns() {
		let dir = tempdir().unwrap();
		let toml_str = format!(r#"
			[[rules]]
			enabled = true
			name = "regex-rule"

			[rules.watch]
			path = "{}"
			recursive = false
			target = "directory"
			include_hidden = true
			regex = "^report_\\d+\\.csv$"
			exclude_patterns = ["*.tmp"]
			events = ["create"]

			[[rules.actions]]
			type = "command"
			shell = "powershell"
			command = "echo test"
			working_dir = ""
		"#, sanitize_path(dir.path()));
		let config: RulesConfig = toml::from_str(&toml_str).unwrap();
		assert!(config.rules[0].watch.patterns.is_none());
		assert!(config.rules[0].watch.regex.is_some());
	}

	#[test]
	fn test_parse_rules_config_all_events() {
		let dir = tempdir().unwrap();
		let toml_str = format!(r#"
			[[rules]]
			enabled = false
			name = "all-events"

			[rules.watch]
			path = "{}"
			recursive = true
			target = "both"
			include_hidden = false
			patterns = ["*"]
			exclude_patterns = []
			events = ["create", "modify", "delete", "rename"]

			[[rules.actions]]
			type = "command"
			shell = "cmd"
			command = "echo done"
			working_dir = ""
		"#, sanitize_path(dir.path()));
		let config: RulesConfig = toml::from_str(&toml_str).unwrap();
		assert_eq!(config.rules[0].watch.events.len(), 4);
	}

	#[test]
	fn test_parse_rules_config_invalid_action_type() {
		let dir = tempdir().unwrap();
		let toml_str = make_rules_toml(
			&sanitize_path(dir.path()),
			r#"type = "delete""#,
		);
		let result: Result<RulesConfig, _> = toml::from_str(&toml_str);
		assert!(result.is_err());
	}

	// =========================================================
	// RulesConfig バリデーションテスト
	// =========================================================

	#[test]
	fn test_validate_rules_empty_rules() {
		let config = RulesConfig { rules: vec![] };
		let result = validate_rules_config(&config);
		assert!(result.is_err());
	}

	#[test]
	fn test_validate_rules_empty_name() {
		let dir = tempdir().unwrap();
		let toml_str = format!(r#"
			[[rules]]
			enabled = true
			name = "  "

			[rules.watch]
			path = "{}"
			recursive = true
			target = "file"
			include_hidden = false
			patterns = ["*.csv"]
			exclude_patterns = []
			events = ["create"]

			[[rules.actions]]
			type = "command"
			shell = "cmd"
			command = "echo hi"
			working_dir = ""
		"#, sanitize_path(dir.path()));
		let config: RulesConfig = toml::from_str(&toml_str).unwrap();
		let result = validate_rules_config(&config);
		assert!(result.is_err());
	}

	#[test]
	fn test_validate_rules_empty_actions() {
		let dir = tempdir().unwrap();
		let toml_str = format!(r#"
			[[rules]]
			enabled = true
			name = "no-actions"
			actions = []

			[rules.watch]
			path = "{}"
			recursive = true
			target = "file"
			include_hidden = false
			patterns = ["*.csv"]
			exclude_patterns = []
			events = ["create"]
		"#, sanitize_path(dir.path()));
		let config: RulesConfig = toml::from_str(&toml_str).unwrap();
		let result = validate_rules_config(&config);
		assert!(result.is_err());
	}

	#[test]
	fn test_validate_rules_empty_events() {
		let dir = tempdir().unwrap();
		let toml_str = format!(r#"
			[[rules]]
			enabled = true
			name = "no-events"

			[rules.watch]
			path = "{}"
			recursive = true
			target = "file"
			include_hidden = false
			patterns = ["*.csv"]
			exclude_patterns = []
			events = []

			[[rules.actions]]
			type = "command"
			shell = "cmd"
			command = "echo hi"
			working_dir = ""
		"#, sanitize_path(dir.path()));
		let config: RulesConfig = toml::from_str(&toml_str).unwrap();
		let result = validate_rules_config(&config);
		assert!(result.is_err());
	}

	// =========================================================
	// patterns / regex 排他チェック
	// =========================================================

	#[test]
	fn test_validate_rules_patterns_and_regex_both_present() {
		let dir = tempdir().unwrap();
		let toml_str = make_rules_toml_with_watch(
			r#"
			recursive = true
			target = "file"
			include_hidden = false
			patterns = ["*.csv"]
			regex = "^test"
			exclude_patterns = []
			events = ["create"]
			"#,
			dir.path().to_str().unwrap(),
		);
		let config: RulesConfig = toml::from_str(&toml_str).unwrap();
		let result = validate_rules_config(&config);
		assert!(result.is_err());
	}

	#[test]
	fn test_validate_rules_patterns_and_regex_both_absent() {
		let dir = tempdir().unwrap();
		let toml_str = make_rules_toml_with_watch(
			r#"
			recursive = true
			target = "file"
			include_hidden = false
			exclude_patterns = []
			events = ["create"]
			"#,
			dir.path().to_str().unwrap(),
		);
		let config: RulesConfig = toml::from_str(&toml_str).unwrap();
		let result = validate_rules_config(&config);
		assert!(result.is_err());
	}

	// =========================================================
	// watch.path 存在チェック
	// =========================================================

	#[test]
	fn test_validate_rules_watch_path_not_exist() {
		let toml_str = make_rules_toml(
			"C:/nonexistent_path_xyz_12345",
			r#"
				type = "command"
				shell = "cmd"
				command = "echo hi"
				working_dir = ""
			"#,
		);
		let config: RulesConfig = toml::from_str(&toml_str).unwrap();
		let result = validate_rules_config(&config);
		assert!(result.is_err());
	}

	// =========================================================
	// glob / regex 構文チェック
	// =========================================================

	#[test]
	fn test_validate_rules_invalid_glob_pattern() {
		let dir = tempdir().unwrap();
		let toml_str = format!(r#"
			[[rules]]
			enabled = true
			name = "bad-glob"

			[rules.watch]
			path = "{}"
			recursive = true
			target = "file"
			include_hidden = false
			patterns = ["[invalid"]
			exclude_patterns = []
			events = ["create"]

			[[rules.actions]]
			type = "command"
			shell = "cmd"
			command = "echo hi"
			working_dir = ""
		"#, sanitize_path(dir.path()));
		let config: RulesConfig = toml::from_str(&toml_str).unwrap();
		let result = validate_rules_config(&config);
		assert!(result.is_err());
	}

	#[test]
	fn test_validate_rules_invalid_regex() {
		let dir = tempdir().unwrap();
		let toml_str = format!(r#"
			[[rules]]
			enabled = true
			name = "bad-regex"

			[rules.watch]
			path = "{}"
			recursive = true
			target = "file"
			include_hidden = false
			regex = "(unclosed"
			exclude_patterns = []
			events = ["create"]

			[[rules.actions]]
			type = "command"
			shell = "cmd"
			command = "echo hi"
			working_dir = ""
		"#, sanitize_path(dir.path()));
		let config: RulesConfig = toml::from_str(&toml_str).unwrap();
		let result = validate_rules_config(&config);
		assert!(result.is_err());
	}

	#[test]
	fn test_validate_rules_invalid_exclude_glob() {
		let dir = tempdir().unwrap();
		let toml_str = format!(r#"
			[[rules]]
			enabled = true
			name = "bad-exclude"

			[rules.watch]
			path = "{}"
			recursive = true
			target = "file"
			include_hidden = false
			patterns = ["*.csv"]
			exclude_patterns = ["[bad"]
			events = ["create"]

			[[rules.actions]]
			type = "command"
			shell = "cmd"
			command = "echo hi"
			working_dir = ""
		"#, sanitize_path(dir.path()));
		let config: RulesConfig = toml::from_str(&toml_str).unwrap();
		let result = validate_rules_config(&config);
		assert!(result.is_err());
	}

	// =========================================================
	// exclude_regex / exclude_dir_patterns / exclude_dir_regex (#28)
	// =========================================================

	#[test]
	fn test_validate_exclude_patterns_and_exclude_regex_both_set() {
		let dir = tempdir().unwrap();
		let toml_str = format!(r#"
			[[rules]]
			enabled = true
			name = "both-exclude"

			[rules.watch]
			path = "{}"
			recursive = true
			target = "file"
			include_hidden = false
			patterns = ["*.csv"]
			exclude_patterns = ["*.tmp"]
			exclude_regex = "^debug"
			events = ["create"]

			[[rules.actions]]
			type = "command"
			shell = "cmd"
			command = "echo hi"
			working_dir = ""
		"#, sanitize_path(dir.path()));
		let config: RulesConfig = toml::from_str(&toml_str).unwrap();
		let result = validate_rules_config(&config);
		assert!(result.is_err());
	}

	#[test]
	fn test_validate_exclude_regex_only_valid() {
		let dir = tempdir().unwrap();
		let toml_str = format!(r#"
			[[rules]]
			enabled = true
			name = "exclude-regex-only"

			[rules.watch]
			path = "{}"
			recursive = true
			target = "file"
			include_hidden = false
			patterns = ["*.csv"]
			exclude_patterns = []
			exclude_regex = "^debug_\\d+"
			events = ["create"]

			[[rules.actions]]
			type = "command"
			shell = "cmd"
			command = "echo hi"
			working_dir = ""
		"#, sanitize_path(dir.path()));
		let config: RulesConfig = toml::from_str(&toml_str).unwrap();
		let result = validate_rules_config(&config);
		assert!(result.is_ok());
	}

	#[test]
	fn test_validate_invalid_exclude_regex() {
		let dir = tempdir().unwrap();
		let toml_str = format!(r#"
			[[rules]]
			enabled = true
			name = "bad-exclude-regex"

			[rules.watch]
			path = "{}"
			recursive = true
			target = "file"
			include_hidden = false
			patterns = ["*.csv"]
			exclude_patterns = []
			exclude_regex = "(unclosed"
			events = ["create"]

			[[rules.actions]]
			type = "command"
			shell = "cmd"
			command = "echo hi"
			working_dir = ""
		"#, sanitize_path(dir.path()));
		let config: RulesConfig = toml::from_str(&toml_str).unwrap();
		let result = validate_rules_config(&config);
		assert!(result.is_err());
	}

	#[test]
	fn test_validate_exclude_dir_patterns_and_exclude_dir_regex_both_set() {
		let dir = tempdir().unwrap();
		let toml_str = format!(r#"
			[[rules]]
			enabled = true
			name = "both-dir-exclude"

			[rules.watch]
			path = "{}"
			recursive = true
			target = "file"
			include_hidden = false
			patterns = ["*.csv"]
			exclude_patterns = []
			exclude_dir_patterns = ["node_modules"]
			exclude_dir_regex = "^\\."
			events = ["create"]

			[[rules.actions]]
			type = "command"
			shell = "cmd"
			command = "echo hi"
			working_dir = ""
		"#, sanitize_path(dir.path()));
		let config: RulesConfig = toml::from_str(&toml_str).unwrap();
		let result = validate_rules_config(&config);
		assert!(result.is_err());
	}

	#[test]
	fn test_validate_exclude_dir_patterns_valid() {
		let dir = tempdir().unwrap();
		let toml_str = format!(r#"
			[[rules]]
			enabled = true
			name = "dir-patterns-ok"

			[rules.watch]
			path = "{}"
			recursive = true
			target = "file"
			include_hidden = false
			patterns = ["*.csv"]
			exclude_patterns = []
			exclude_dir_patterns = ["node_modules", ".git"]
			events = ["create"]

			[[rules.actions]]
			type = "command"
			shell = "cmd"
			command = "echo hi"
			working_dir = ""
		"#, sanitize_path(dir.path()));
		let config: RulesConfig = toml::from_str(&toml_str).unwrap();
		let result = validate_rules_config(&config);
		assert!(result.is_ok());
	}

	#[test]
	fn test_validate_invalid_exclude_dir_glob() {
		let dir = tempdir().unwrap();
		let toml_str = format!(r#"
			[[rules]]
			enabled = true
			name = "bad-dir-glob"

			[rules.watch]
			path = "{}"
			recursive = true
			target = "file"
			include_hidden = false
			patterns = ["*.csv"]
			exclude_patterns = []
			exclude_dir_patterns = ["[bad"]
			events = ["create"]

			[[rules.actions]]
			type = "command"
			shell = "cmd"
			command = "echo hi"
			working_dir = ""
		"#, sanitize_path(dir.path()));
		let config: RulesConfig = toml::from_str(&toml_str).unwrap();
		let result = validate_rules_config(&config);
		assert!(result.is_err());
	}

	#[test]
	fn test_validate_invalid_exclude_dir_regex() {
		let dir = tempdir().unwrap();
		let toml_str = format!(r#"
			[[rules]]
			enabled = true
			name = "bad-dir-regex"

			[rules.watch]
			path = "{}"
			recursive = true
			target = "file"
			include_hidden = false
			patterns = ["*.csv"]
			exclude_patterns = []
			exclude_dir_regex = "(unclosed"
			events = ["create"]

			[[rules.actions]]
			type = "command"
			shell = "cmd"
			command = "echo hi"
			working_dir = ""
		"#, sanitize_path(dir.path()));
		let config: RulesConfig = toml::from_str(&toml_str).unwrap();
		let result = validate_rules_config(&config);
		assert!(result.is_err());
	}

	#[test]
	fn test_validate_exclude_dir_regex_valid() {
		let dir = tempdir().unwrap();
		let toml_str = format!(r#"
			[[rules]]
			enabled = true
			name = "dir-regex-ok"

			[rules.watch]
			path = "{}"
			recursive = true
			target = "file"
			include_hidden = false
			patterns = ["*.csv"]
			exclude_patterns = []
			exclude_dir_regex = "^\\."
			events = ["create"]

			[[rules.actions]]
			type = "command"
			shell = "cmd"
			command = "echo hi"
			working_dir = ""
		"#, sanitize_path(dir.path()));
		let config: RulesConfig = toml::from_str(&toml_str).unwrap();
		let result = validate_rules_config(&config);
		assert!(result.is_ok());
	}

	// =========================================================
	// dir_patterns / dir_regex (#28)
	// =========================================================

	#[test]
	fn test_validate_dir_patterns_and_dir_regex_both_set() {
		let dir = tempdir().unwrap();
		let toml_str = format!(r#"
			[[rules]]
			enabled = true
			name = "both-dir-include"

			[rules.watch]
			path = "{}"
			recursive = true
			target = "file"
			include_hidden = false
			patterns = ["*.csv"]
			exclude_patterns = []
			dir_patterns = ["src"]
			dir_regex = "^reports"
			events = ["create"]

			[[rules.actions]]
			type = "command"
			shell = "cmd"
			command = "echo hi"
			working_dir = ""
		"#, sanitize_path(dir.path()));
		let config: RulesConfig = toml::from_str(&toml_str).unwrap();
		let result = validate_rules_config(&config);
		assert!(result.is_err());
	}

	#[test]
	fn test_validate_dir_patterns_valid() {
		let dir = tempdir().unwrap();
		let toml_str = format!(r#"
			[[rules]]
			enabled = true
			name = "dir-patterns-include-ok"

			[rules.watch]
			path = "{}"
			recursive = true
			target = "file"
			include_hidden = false
			patterns = ["*.csv"]
			exclude_patterns = []
			dir_patterns = ["src", "reports"]
			events = ["create"]

			[[rules.actions]]
			type = "command"
			shell = "cmd"
			command = "echo hi"
			working_dir = ""
		"#, sanitize_path(dir.path()));
		let config: RulesConfig = toml::from_str(&toml_str).unwrap();
		let result = validate_rules_config(&config);
		assert!(result.is_ok());
	}

	#[test]
	fn test_validate_invalid_dir_glob() {
		let dir = tempdir().unwrap();
		let toml_str = format!(r#"
			[[rules]]
			enabled = true
			name = "bad-dir-include-glob"

			[rules.watch]
			path = "{}"
			recursive = true
			target = "file"
			include_hidden = false
			patterns = ["*.csv"]
			exclude_patterns = []
			dir_patterns = ["[bad"]
			events = ["create"]

			[[rules.actions]]
			type = "command"
			shell = "cmd"
			command = "echo hi"
			working_dir = ""
		"#, sanitize_path(dir.path()));
		let config: RulesConfig = toml::from_str(&toml_str).unwrap();
		let result = validate_rules_config(&config);
		assert!(result.is_err());
	}

	#[test]
	fn test_validate_dir_regex_valid() {
		let dir = tempdir().unwrap();
		let toml_str = format!(r#"
			[[rules]]
			enabled = true
			name = "dir-regex-include-ok"

			[rules.watch]
			path = "{}"
			recursive = true
			target = "file"
			include_hidden = false
			patterns = ["*.csv"]
			exclude_patterns = []
			dir_regex = "^reports_\\d+"
			events = ["create"]

			[[rules.actions]]
			type = "command"
			shell = "cmd"
			command = "echo hi"
			working_dir = ""
		"#, sanitize_path(dir.path()));
		let config: RulesConfig = toml::from_str(&toml_str).unwrap();
		let result = validate_rules_config(&config);
		assert!(result.is_ok());
	}

	#[test]
	fn test_validate_invalid_dir_regex() {
		let dir = tempdir().unwrap();
		let toml_str = format!(r#"
			[[rules]]
			enabled = true
			name = "bad-dir-regex-include"

			[rules.watch]
			path = "{}"
			recursive = true
			target = "file"
			include_hidden = false
			patterns = ["*.csv"]
			exclude_patterns = []
			dir_regex = "(unclosed"
			events = ["create"]

			[[rules.actions]]
			type = "command"
			shell = "cmd"
			command = "echo hi"
			working_dir = ""
		"#, sanitize_path(dir.path()));
		let config: RulesConfig = toml::from_str(&toml_str).unwrap();
		let result = validate_rules_config(&config);
		assert!(result.is_err());
	}

	// =========================================================
	// validate_action: Copy / Move
	// =========================================================

	#[test]
	fn test_validate_action_copy_valid() {
		let dest = tempdir().unwrap();
		let action = ActionConfig {
			type_: ActionType::Copy,
			destination: Some(dest.path().to_str().unwrap().to_string()),
			overwrite: Some(true),
			verify_integrity: Some(true),
			preserve_structure: Some(false),
			working_dir: None,
			shell: None,
			command: None,
			program: None,
			args: None,
			message: None,
		};
		assert!(validate_action(&action, "test").is_ok());
	}

	#[test]
	fn test_validate_action_copy_missing_destination() {
		let action = ActionConfig {
			type_: ActionType::Copy,
			destination: None,
			overwrite: Some(true),
			verify_integrity: Some(true),
			preserve_structure: Some(false),
			working_dir: None,
			shell: None,
			command: None,
			program: None,
			args: None,
			message: None,
		};
		assert!(validate_action(&action, "test").is_err());
	}

	#[test]
	fn test_validate_action_copy_missing_overwrite() {
		let dest = tempdir().unwrap();
		let action = ActionConfig {
			type_: ActionType::Copy,
			destination: Some(dest.path().to_str().unwrap().to_string()),
			overwrite: None,
			verify_integrity: Some(true),
			preserve_structure: Some(false),
			working_dir: None,
			shell: None,
			command: None,
			program: None,
			args: None,
			message: None,
		};
		assert!(validate_action(&action, "test").is_err());
	}

	#[test]
	fn test_validate_action_copy_missing_verify_integrity() {
		let dest = tempdir().unwrap();
		let action = ActionConfig {
			type_: ActionType::Copy,
			destination: Some(dest.path().to_str().unwrap().to_string()),
			overwrite: Some(true),
			verify_integrity: None,
			preserve_structure: Some(false),
			working_dir: None,
			shell: None,
			command: None,
			program: None,
			args: None,
			message: None,
		};
		assert!(validate_action(&action, "test").is_err());
	}

	#[test]
	fn test_validate_action_copy_missing_preserve_structure() {
		let dest = tempdir().unwrap();
		let action = ActionConfig {
			type_: ActionType::Copy,
			destination: Some(dest.path().to_str().unwrap().to_string()),
			overwrite: Some(true),
			verify_integrity: Some(true),
			preserve_structure: None,
			working_dir: None,
			shell: None,
			command: None,
			program: None,
			args: None,
			message: None,
		};
		assert!(validate_action(&action, "test").is_err());
	}

	#[test]
	fn test_validate_action_copy_destination_not_exist() {
		let action = ActionConfig {
			type_: ActionType::Copy,
			destination: Some("C:/nonexistent_dest_xyz_99999".to_string()),
			overwrite: Some(true),
			verify_integrity: Some(true),
			preserve_structure: Some(false),
			working_dir: None,
			shell: None,
			command: None,
			program: None,
			args: None,
			message: None,
		};
		assert!(validate_action(&action, "test").is_err());
	}

	#[test]
	fn test_validate_action_copy_destination_with_placeholder_ok() {
		// destination 中の {Date} 以降はランタイム展開されるため、ルートだけ実在すれば OK
		let dest_root = tempdir().unwrap();
		let dest_template = format!("{}/{{Date}}/sub", sanitize_path(dest_root.path()));
		let action = ActionConfig {
			type_: ActionType::Copy,
			destination: Some(dest_template),
			overwrite: Some(true),
			verify_integrity: Some(false),
			preserve_structure: Some(false),
			working_dir: None,
			shell: None,
			command: None,
			program: None,
			args: None,
			message: None,
		};
		assert!(validate_action(&action, "test").is_ok());
	}

	#[test]
	fn test_validate_action_copy_destination_with_placeholder_root_missing() {
		// プレースホルダー前のルートが存在しなければエラー
		let action = ActionConfig {
			type_: ActionType::Copy,
			destination: Some("C:/nonexistent_root_zzz_99999/{Date}".to_string()),
			overwrite: Some(true),
			verify_integrity: Some(false),
			preserve_structure: Some(false),
			working_dir: None,
			shell: None,
			command: None,
			program: None,
			args: None,
			message: None,
		};
		assert!(validate_action(&action, "test").is_err());
	}

	#[test]
	fn test_static_root_of_destination_no_placeholder() {
		assert_eq!(static_root_of_destination("C:/data/backup"), "C:/data/backup");
	}

	#[test]
	fn test_static_root_of_destination_with_placeholder() {
		assert_eq!(static_root_of_destination("C:/data/backup/{Date}"), "C:/data/backup/");
		assert_eq!(static_root_of_destination("C:/data/backup/{Date}/sub"), "C:/data/backup/");
	}

	#[test]
	fn test_static_root_of_destination_backslash_separator() {
		assert_eq!(static_root_of_destination(r"C:\data\backup\{Date}"), r"C:\data\backup\");
	}

	#[test]
	fn test_static_root_of_destination_placeholder_at_start() {
		assert_eq!(static_root_of_destination("{Date}/sub"), "");
	}

	#[test]
	fn test_validate_action_move_valid() {
		let dest = tempdir().unwrap();
		let action = ActionConfig {
			type_: ActionType::Move,
			destination: Some(dest.path().to_str().unwrap().to_string()),
			overwrite: Some(false),
			verify_integrity: Some(false),
			preserve_structure: Some(true),
			working_dir: None,
			shell: None,
			command: None,
			program: None,
			args: None,
			message: None,
		};
		assert!(validate_action(&action, "test").is_ok());
	}

	// =========================================================
	// validate_action: Command
	// =========================================================

	#[test]
	fn test_validate_action_command_valid() {
		let action = ActionConfig {
			type_: ActionType::Command,
			destination: None,
			overwrite: None,
			verify_integrity: None,
			preserve_structure: None,
			working_dir: Some("".to_string()),
			shell: Some("cmd".to_string()),
			command: Some("echo hello".to_string()),
			program: None,
			args: None,
			message: None,
		};
		assert!(validate_action(&action, "test").is_ok());
	}

	#[test]
	fn test_validate_action_command_missing_shell() {
		let action = ActionConfig {
			type_: ActionType::Command,
			destination: None,
			overwrite: None,
			verify_integrity: None,
			preserve_structure: None,
			working_dir: Some("".to_string()),
			shell: None,
			command: Some("echo hello".to_string()),
			program: None,
			args: None,
			message: None,
		};
		assert!(validate_action(&action, "test").is_err());
	}

	#[test]
	fn test_validate_action_command_missing_command() {
		let action = ActionConfig {
			type_: ActionType::Command,
			destination: None,
			overwrite: None,
			verify_integrity: None,
			preserve_structure: None,
			working_dir: Some("".to_string()),
			shell: Some("cmd".to_string()),
			command: None,
			program: None,
			args: None,
			message: None,
		};
		assert!(validate_action(&action, "test").is_err());
	}

	#[test]
	fn test_validate_action_command_missing_working_dir() {
		let action = ActionConfig {
			type_: ActionType::Command,
			destination: None,
			overwrite: None,
			verify_integrity: None,
			preserve_structure: None,
			working_dir: None,
			shell: Some("cmd".to_string()),
			command: Some("echo hello".to_string()),
			program: None,
			args: None,
			message: None,
		};
		assert!(validate_action(&action, "test").is_err());
	}

	// =========================================================
	// validate_action: Execute
	// =========================================================

	#[test]
	fn test_validate_action_execute_valid() {
		let action = ActionConfig {
			type_: ActionType::Execute,
			destination: None,
			overwrite: None,
			verify_integrity: None,
			preserve_structure: None,
			working_dir: Some("".to_string()),
			shell: None,
			command: None,
			program: Some("notepad.exe".to_string()),
			args: Some(vec![]),
			message: None,
		};
		assert!(validate_action(&action, "test").is_ok());
	}

	#[test]
	fn test_validate_action_execute_missing_program() {
		let action = ActionConfig {
			type_: ActionType::Execute,
			destination: None,
			overwrite: None,
			verify_integrity: None,
			preserve_structure: None,
			working_dir: Some("".to_string()),
			shell: None,
			command: None,
			program: None,
			args: Some(vec![]),
			message: None,
		};
		assert!(validate_action(&action, "test").is_err());
	}

	#[test]
	fn test_validate_action_execute_missing_args() {
		let action = ActionConfig {
			type_: ActionType::Execute,
			destination: None,
			overwrite: None,
			verify_integrity: None,
			preserve_structure: None,
			working_dir: Some("".to_string()),
			shell: None,
			command: None,
			program: Some("notepad.exe".to_string()),
			args: None,
			message: None,
		};
		assert!(validate_action(&action, "test").is_err());
	}

	#[test]
	fn test_validate_action_execute_missing_working_dir() {
		let action = ActionConfig {
			type_: ActionType::Execute,
			destination: None,
			overwrite: None,
			verify_integrity: None,
			preserve_structure: None,
			working_dir: None,
			shell: None,
			command: None,
			program: Some("notepad.exe".to_string()),
			args: Some(vec!["file.txt".to_string()]),
			message: None,
		};
		assert!(validate_action(&action, "test").is_err());
	}

	// =========================================================
	// 正常系: 全体を通したバリデーション
	// =========================================================

	#[test]
	fn test_validate_rules_config_valid_copy_rule() {
		let watch_dir = tempdir().unwrap();
		let dest_dir = tempdir().unwrap();
		let toml_str = make_rules_toml(
			&sanitize_path(watch_dir.path()),
			&format!(r#"
				type = "copy"
				destination = "{}"
				overwrite = true
				verify_integrity = false
				preserve_structure = true
			"#, sanitize_path(dest_dir.path())),
		);
		let config: RulesConfig = toml::from_str(&toml_str).unwrap();
		let result = validate_rules_config(&config);
		assert!(result.is_ok());
	}

	#[test]
	fn test_validate_rules_config_valid_command_rule() {
		let watch_dir = tempdir().unwrap();
		let toml_str = make_rules_toml(
			&sanitize_path(watch_dir.path()),
			r#"
				type = "command"
				shell = "powershell"
				command = "Get-Date"
				working_dir = ""
			"#,
		);
		let config: RulesConfig = toml::from_str(&toml_str).unwrap();
		let result = validate_rules_config(&config);
		assert!(result.is_ok());
	}

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
		let result = validate_rules_config(&config);
		assert!(result.is_ok());
	}
}