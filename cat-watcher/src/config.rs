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
#[serde(deny_unknown_fields)]
pub struct GlobalConfig {
    pub retry: RetryConfig,
    pub system_log: SystemLogConfig,
    /// ダッシュボード設定（省略可）。未指定なら無効。
    #[serde(default)]
    pub dashboard: Option<DashboardConfig>,
    /// 起動時スキャン設定（省略可）。未指定なら有効（既定 ON）。
    #[serde(default)]
    pub startup_scan: Option<StartupScanConfig>,
    /// Windows サービス設定（省略可）。未指定なら既定値を使う。
    // service フィールドは Windows のサービス起動経路でのみ参照するため、
    // 非 Windows ビルドでは未使用になる。
    #[cfg_attr(not(windows), allow(dead_code))]
    #[serde(default)]
    pub service: Option<ServiceConfig>,
}

impl GlobalConfig {
    /// 起動時スキャンを行うか。セクション未指定なら有効（既定 ON）。
    /// イベント監視だけでは取りこぼす「起動前から存在するファイル」や
    /// 「ダウンタイム中に届いたファイル」を回収するための安全網。
    pub fn scan_on_start(&self) -> bool {
        self.startup_scan
            .as_ref()
            .map(|s| s.enabled)
            .unwrap_or(true)
    }

    /// Windows サービス起動時に、外部プロセス（command / execute）を
    /// アクティブなログオンユーザーの権限で実行するか。セクション未指定なら
    /// 有効（既定 ON）。ログオンユーザーがいない場合はサービスアカウント
    /// 権限へフォールバックする。CLI 起動時はこの設定に関係なく、元々
    /// ログオンユーザー権限で動作する。
    // 参照するのは Windows のサービス起動経路のみ。
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn run_as_logged_in_user(&self) -> bool {
        self.service
            .as_ref()
            .map(|s| s.run_as_logged_in_user)
            .unwrap_or(true)
    }
}

/// Windows サービス設定。サービスとして常駐する場合の挙動を制御する。
/// CLI 起動時には参照されない。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceConfig {
    /// サービス起動時に外部プロセスをアクティブなログオンユーザー権限で実行するか。
    /// サービスは既定で SYSTEM 権限で動くため、PowerShell / 7-Zip などの外部
    /// プロセスがログオンユーザーの環境で動かず不便なことへの対策。
    /// ログオンユーザーがいない（誰もログオンせずサービス起動した）場合は
    /// サービスアカウント権限で起動する。
    // 値を読むのは Windows のサービス起動経路のみ。
    #[cfg_attr(not(windows), allow(dead_code))]
    #[serde(default = "default_true")]
    pub run_as_logged_in_user: bool,
}

/// 起動時スキャン設定。起動直後に監視フォルダを 1 度だけ走査し、
/// 既に存在するエントリを Create イベントとして検知パイプラインへ流す。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartupScanConfig {
    /// 起動時スキャンを行うか。
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// ダッシュボード（ブラウザでログをリアルタイム表示する localhost HTTP サーバ）設定。
/// 実際に機能するのは `dashboard` feature を有効にしてビルドした場合のみ。
/// feature 無しビルドでも設定の読み込み・検証は行う（セクションを書いてもエラーにしない）。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DashboardConfig {
    /// ダッシュボードを起動するか。
    #[serde(default)]
    pub enabled: bool,
    /// 待ち受けアドレス。ログにパスが出るためローカル限定を推奨。
    #[serde(default = "default_dashboard_bind")]
    pub bind: String,
    /// 接続時にブラウザへ再生する直近イベント件数（メモリ保持）。
    // history はサーバ起動時（dashboard feature 有効時）のみ参照するため、
    // feature 無しビルドでは未使用になる。
    #[cfg_attr(not(feature = "dashboard"), allow(dead_code))]
    #[serde(default = "default_dashboard_history")]
    pub history: usize,
}

fn default_dashboard_bind() -> String {
    "127.0.0.1:8080".to_string()
}

fn default_dashboard_history() -> usize {
    200
}

#[derive(Debug, Clone, Deserialize)]
pub struct RulesConfig {
    pub rules: Vec<Rule>,
}

fn default_true() -> bool {
    true
}

/// リトライ設定（copy / move アクション用）。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetryConfig {
    pub count: u32,
    pub interval_ms: u64,
}

/// システムログ設定（プログラム全体の起動日誌）。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemLogConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub dir: String,
    pub file_name: String,
    pub rotation: LogRotation,
    pub level: LogLevel,
    /// コンソール出力 ON/OFF（ターミナル再設計までの暫定置き場）。
    #[serde(default = "default_true")]
    pub console: bool,
}

/// ルール別ログ設定。検知ログ・アクションログをそれぞれ個別指定する。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleLog {
    pub detect: Option<RuleLogTarget>,
    pub action: Option<RuleLogTarget>,
}

/// 検知ログ / アクションログ共通の出力先設定。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleLogTarget {
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub dir: String,
    pub file_name: String,
    pub rotation: LogRotation,
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
	config.system_log.dir = expand_tilde(&config.system_log.dir);
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
			if let Some(detect) = &mut log.detect {
				detect.dir = expand_tilde(&detect.dir);
			}
			if let Some(action) = &mut log.action {
				action.dir = expand_tilde(&action.dir);
			}
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

/// ログの出力先ディレクトリとファイル名を検証する共通ヘルパ。
/// `label` はエラーメッセージ内のフィールド名（例: "system_log.dir"）。
fn validate_log_target(
	dir: &str,
	file_name: &str,
	dir_label: &str,
	file_label: &str,
	errors: &mut Vec<String>,
) {
	if dir.trim().is_empty() {
		errors.push(format!("{dir_label} が空文字列です。ログ出力先ディレクトリを定義してください"));
	} else {
		let dir_path = Path::new(dir);
		if !dir_path.exists() {
			errors.push(format!("{dir_label} が存在しません: {}", dir_path.display()));
		} else if !dir_path.is_dir() {
			errors.push(format!("{dir_label} にディレクトリ以外のパスが指定されています: {}", dir_path.display()));
		}
	}

	if file_name.trim().is_empty() {
		errors.push(format!("{file_label} が空文字列です。ファイル名を定義してください"));
	} else {
		let valid_placeholders = ["Date", "DateTime"];
		let re = regex::Regex::new(r"\{([A-Za-z]+)\}").unwrap();
		for caps in re.captures_iter(file_name) {
			let name = &caps[1];
			if !valid_placeholders.contains(&name) {
				errors.push(format!(
					"{file_label} に使用できないプレースホルダーがあります: {{{name}}}。使用可能なのは {{Date}} と {{DateTime}} のみです"
				));
			}
		}
	}
}

pub fn validate_global_config(config: &GlobalConfig) -> Result<(), AppError> {
	let mut errors = Vec::new();
	validate_log_target(
		&config.system_log.dir,
		&config.system_log.file_name,
		"system_log.dir",
		"system_log.file_name",
		&mut errors,
	);
	if let Some(dashboard) = &config.dashboard {
		if dashboard.enabled && dashboard.bind.parse::<std::net::SocketAddr>().is_err() {
			errors.push(format!(
				"dashboard.bind がソケットアドレスとして不正です（例: 127.0.0.1:8080）: {}",
				dashboard.bind
			));
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
			if let Some(detect) = &rule_log.detect {
				if detect.enabled {
					validate_log_target(
						&detect.dir,
						&detect.file_name,
						&format!("監視ルール名 {} の log.detect.dir", rule_id),
						&format!("監視ルール名 {} の log.detect.file_name", rule_id),
						&mut errors,
					);
				}
			}
			if let Some(action) = &rule_log.action {
				if action.enabled {
					validate_log_target(
						&action.dir,
						&action.file_name,
						&format!("監視ルール名 {} の log.action.dir", rule_id),
						&format!("監視ルール名 {} の log.action.file_name", rule_id),
						&mut errors,
					);
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
#[path = "config_tests.rs"]
mod tests;
