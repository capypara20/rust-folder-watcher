//! 設定ファイルの読み込みと、`~` のホームディレクトリ展開。

use std::path::{Path, PathBuf};

use super::model::{GlobalConfig, RulesConfig};
use super::types::ActionType;
use super::validate::{validate_global_config, validate_rules_config};
use crate::error::{AppError, InvalidFile};
use crate::path_fmt;

/// 設定ファイルを既定の場所から探す。
///
/// 探す順番は「カレントディレクトリ → 実行ファイルと同じフォルダ」。
/// Windows サービスとして起動するとカレントディレクトリが `C:\Windows\System32`
/// になるため、実行ファイル横も見ることで「exe と設定を同じフォルダに置いて
/// サービス登録する」運用がオプション指定なしで動く。
pub fn find_config_file(file_name: &str) -> Option<PathBuf> {
	config_file_candidates(file_name).into_iter().find(|p| p.is_file())
}

/// 既定の設定ファイルを探す場所。見つからなかったときのエラーにも出す。
fn config_file_candidates(file_name: &str) -> Vec<PathBuf> {
	let mut candidates = Vec::new();
	if let Ok(cwd) = std::env::current_dir() {
		candidates.push(cwd.join(file_name));
	}
	if let Ok(exe) = std::env::current_exe() {
		if let Some(dir) = exe.parent() {
			candidates.push(dir.join(file_name));
		}
	}
	candidates
}

/// コマンドラインで明示されたパスがあればそれを、無ければ既定の場所から探す。
/// `flag` はエラーメッセージに載せるオプション名（例: `--global`）。
pub fn resolve_config_path(
	explicit: Option<PathBuf>,
	file_name: &str,
	flag: &str,
) -> Result<PathBuf, AppError> {
	if let Some(path) = explicit {
		return Ok(path);
	}
	find_config_file(file_name).ok_or_else(|| {
		// 「どこを探したか」を実際のパスで出す。サービスではカレントディレクトリが
		// 想定と違うことが多く、場所の説明だけでは切り分けられないため。
		let searched = config_file_candidates(file_name)
			.iter()
			.map(|p| format!("\n    {}", path_fmt::for_log(p)))
			.collect::<String>();
		AppError::Usage(format!(
			"{file_name} が見つかりません。{flag} で設定ファイルのパスを指定してください\n  探した場所:{searched}"
		))
	})
}

pub(crate) fn expand_tilde(s: &str) -> String {
	if s == "~" || s.starts_with("~/") || s.starts_with("~\\") {
		let home = std::env::var_os("HOME")
			.or_else(|| std::env::var_os("USERPROFILE"))
			.map(|h| h.to_string_lossy().into_owned())
			.unwrap_or_default();
		return format!("{}{}", home, &s[1..]);
	}
	s.to_string()
}

/// 設定を読み込み、global の既定値をルールへ反映し、検証まで済ませる。
///
/// CLI とサービスの両方がここを通る。以前は同じ手順を 2 か所に書いていた。
///
/// 検証は global と rules の両方を行ってからまとめて返す。
/// 片方で止めると、直して再実行したあとにもう片方の問題が出てくるため。
pub fn load(global_path: &Path, rules_path: &Path) -> Result<(GlobalConfig, RulesConfig), AppError> {
	let global = load_global_config(global_path)?;
	let mut rules = load_rules_config(rules_path)?;
	// 検証と実行時が同じ値を見るよう、検証の前に反映する。
	apply_global_defaults(&global, &mut rules);

	let mut invalid = Vec::new();
	if let Err(problems) = validate_global_config(&global) {
		invalid.push(InvalidFile::new(global_path, problems));
	}
	if let Err(problems) = validate_rules_config(&rules) {
		invalid.push(InvalidFile::new(rules_path, problems));
	}
	if !invalid.is_empty() {
		return Err(AppError::ConfigInvalid(invalid));
	}
	Ok((global, rules))
}

/// 設定ファイルを読んで TOML として解釈する。失敗したらファイルパスを添える。
fn read_toml<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, AppError> {
	let content = std::fs::read_to_string(path).map_err(|source| AppError::ConfigRead {
		path: path.to_path_buf(),
		source,
	})?;
	toml::from_str(&content).map_err(|e| AppError::ConfigParse {
		path: path.to_path_buf(),
		message: e.to_string().trim_end().to_string(),
	})
}

pub fn load_global_config(path: &Path) -> Result<GlobalConfig, AppError> {
	let mut config: GlobalConfig = read_toml(path)?;
	config.system_log.dir = expand_tilde(&config.system_log.dir);
	Ok(config)
}

/// global.toml 側の既定値を、各ルールのアクションへ焼き込む。
///
/// バリデーションと実行時の両方が「アクションに書かれた値」だけを見れば済むよう、
/// 読み込み直後にここで解決してしまう。個別指定（`auto_create`）があればそれを
/// 優先し、無ければ global の値を入れる。
pub fn apply_global_defaults(global: &GlobalConfig, rules: &mut RulesConfig) {
	let default_auto_create = global.auto_create_destination();
	let default_wait = global.wait_for_process();
	let default_timeout_ms = global.process_timeout_ms();
	for rule in &mut rules.rules {
		for action in &mut rule.actions {
			if action.auto_create.is_none() {
				action.auto_create = Some(default_auto_create);
			}
			// wait / timeout_ms は外部プロセスを起動するアクション専用の設定。
			// copy / move には焼き込まないでおく。そうしておけば「copy に wait を
			// 書いてしまった」ケースをバリデーションで検出できる。
			if matches!(action.type_, ActionType::Command | ActionType::Execute) {
				if action.wait.is_none() {
					action.wait = Some(default_wait);
				}
				if action.timeout_ms.is_none() {
					action.timeout_ms = default_timeout_ms;
				}
			}
		}
	}
}

pub fn load_rules_config(path: &Path) -> Result<RulesConfig, AppError> {
	let mut config: RulesConfig = read_toml(path)?;
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
