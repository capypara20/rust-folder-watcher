//! 設定ファイルの読み込みと、`~` のホームディレクトリ展開。

use std::path::{Path, PathBuf};

use super::model::{GlobalConfig, RulesConfig};
use crate::error::AppError;

/// 設定ファイルを既定の場所から探す。
///
/// 探す順番は「カレントディレクトリ → 実行ファイルと同じフォルダ」。
/// Windows サービスとして起動するとカレントディレクトリが `C:\Windows\System32`
/// になるため、実行ファイル横も見ることで「exe と設定を同じフォルダに置いて
/// サービス登録する」運用がオプション指定なしで動く。
pub fn find_config_file(file_name: &str) -> Option<PathBuf> {
	let mut candidates = Vec::new();
	if let Ok(cwd) = std::env::current_dir() {
		candidates.push(cwd.join(file_name));
	}
	if let Ok(exe) = std::env::current_exe() {
		if let Some(dir) = exe.parent() {
			candidates.push(dir.join(file_name));
		}
	}
	candidates.into_iter().find(|p| p.is_file())
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
		AppError::Config(format!(
			"{flag} が未指定で、{file_name} も見つかりませんでした（カレントディレクトリと実行ファイルと同じフォルダを探しました）"
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

pub fn load_global_config(path: &Path) -> Result<GlobalConfig, AppError> {
	let content = std::fs::read_to_string(path)?;
	let mut config: GlobalConfig = toml::from_str(&content)
							.map_err(|e| AppError::TomlParse(e.to_string()))?;
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
	for rule in &mut rules.rules {
		for action in &mut rule.actions {
			if action.auto_create.is_none() {
				action.auto_create = Some(default_auto_create);
			}
		}
	}
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
