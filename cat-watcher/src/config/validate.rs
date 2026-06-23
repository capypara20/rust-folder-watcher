//! 設定（global.toml / rules.toml）の意味的バリデーション。

use std::path::Path;

use globset::Glob;
use regex::Regex;

use super::model::{ActionConfig, GlobalConfig, RulesConfig};
use super::types::ActionType;
use crate::error::AppError;
use crate::placeholder::validate_placeholders;

pub(crate) fn finish_validation(errors: Vec<String>) -> Result<(), AppError> {
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
pub(crate) fn static_root_of_destination(dest: &str) -> &str {
	match dest.find('{') {
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
	}
}

pub(crate) fn collect_action_errors(action: &ActionConfig, rule_name: &str, errors: &mut Vec<String>) {
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
