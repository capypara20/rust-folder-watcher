//! 設定ファイルの読み込みと、`~` のホームディレクトリ展開。

use std::path::Path;

use super::model::{GlobalConfig, RulesConfig};
use crate::error::AppError;

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
