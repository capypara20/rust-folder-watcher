//! 設定（global.toml / rules.toml）の意味的バリデーション。

use std::path::Path;

use globset::Glob;
use regex::Regex;
use crate::exe_path;

use super::action::{missing_fields, rejected_fields};
use super::model::{ActionConfig, GlobalConfig, RulesConfig};
use super::types::ActionType;
use crate::actions::command::VALID_SHELLS;
use crate::placeholder::validate_placeholders;

/// 検証の結果。問題が 1 件も無ければ `Ok`。
///
/// どのファイルの問題かはここでは分からない（設定の値だけを見ている）ので、
/// ファイルパスを付けて `AppError` にするのは読み込み側（`config::load`）の役目。
pub type Problems = Vec<String>;

pub(crate) fn finish_validation(errors: Problems) -> Result<(), Problems> {
	if errors.is_empty() {
		Ok(())
	} else {
		Err(errors)
	}
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

pub fn validate_global_config(config: &GlobalConfig) -> Result<(), Problems> {
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
	// 0 を渡すと tokio のタイマーが作れずパニックするため、ここで止める。
	if config.poll_interval_ms() == 0 {
		errors.push("detect.poll_interval_ms は 1 以上にしてください（0 では検知の確認処理が回りません）".to_string());
	}
	finish_validation(errors)
}

/// glob パターン列の構文を検査する。
///
/// patterns / exclude_patterns / dir_patterns / exclude_dir_patterns の 4 種で共用する。
/// `field` はエラーメッセージに出す設定キー名。
fn collect_glob_errors(patterns: &[String], rule_id: &str, field: &str, errors: &mut Vec<String>) {
	for pt in patterns {
		if let Err(e) = Glob::new(pt) {
			errors.push(format!("監視ルール名 {} の {} に無効な glob があります '{}': {}", rule_id, field, pt, e));
		}
	}
}

/// 正規表現の構文を検査する。
///
/// regex / exclude_regex / dir_regex / exclude_dir_regex の 4 種で共用する。
fn collect_regex_errors(pattern: Option<&str>, rule_id: &str, field: &str, errors: &mut Vec<String>) {
	if let Some(re_str) = pattern {
		if let Err(e) = Regex::new(re_str) {
			errors.push(format!("監視ルール名 {} の {} に無効な正規表現があります '{}': {}", rule_id, field, re_str, e));
		}
	}
}

/// glob 列と正規表現が両方指定されていないかを検査する。
///
/// watch.patterns と watch.regex だけは「どちらか一方が必須」で意味が違うため、
/// ここではなく呼び出し側で個別に判定している。
fn collect_exclusive_error(
	patterns: &[String],
	regex: Option<&str>,
	rule_id: &str,
	glob_field: &str,
	regex_field: &str,
	errors: &mut Vec<String>,
) {
	if !patterns.is_empty() && regex.is_some() {
		errors.push(format!("監視ルール名 {} の {} と {} は片方のみ定義できます", rule_id, glob_field, regex_field));
	}
}

pub fn validate_rules_config(config: &RulesConfig) -> Result<(), Problems> {
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

		// glob / 正規表現の構文チェックと、glob 列と正規表現の排他チェック。
		// errors に積む順番がそのままエラー表示の順番になるので、並びは変えないこと。
		collect_glob_errors(rule.watch.patterns.as_deref().unwrap_or(&[]), &rule_id, "patterns", &mut errors);
		collect_regex_errors(rule.watch.regex.as_deref(), &rule_id, "regex", &mut errors);

		collect_glob_errors(&rule.watch.exclude_patterns, &rule_id, "exclude_patterns", &mut errors);
		collect_exclusive_error(
			&rule.watch.exclude_patterns,
			rule.watch.exclude_regex.as_deref(),
			&rule_id,
			"exclude_patterns",
			"exclude_regex",
			&mut errors,
		);
		collect_regex_errors(rule.watch.exclude_regex.as_deref(), &rule_id, "exclude_regex", &mut errors);

		collect_exclusive_error(
			&rule.watch.exclude_dir_patterns,
			rule.watch.exclude_dir_regex.as_deref(),
			&rule_id,
			"exclude_dir_patterns",
			"exclude_dir_regex",
			&mut errors,
		);
		collect_glob_errors(&rule.watch.exclude_dir_patterns, &rule_id, "exclude_dir_patterns", &mut errors);
		collect_regex_errors(rule.watch.exclude_dir_regex.as_deref(), &rule_id, "exclude_dir_regex", &mut errors);

		collect_exclusive_error(
			&rule.watch.dir_patterns,
			rule.watch.dir_regex.as_deref(),
			&rule_id,
			"dir_patterns",
			"dir_regex",
			&mut errors,
		);
		collect_glob_errors(&rule.watch.dir_patterns, &rule_id, "dir_patterns", &mut errors);
		collect_regex_errors(rule.watch.dir_regex.as_deref(), &rule_id, "dir_regex", &mut errors);

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
///   "{WatchPath}/out"           → "" (静的部分なし = 実行時まで判定不能)
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

/// copy / move の destination をロード時に検査する。
///
/// 実行時は宛先フォルダを自動作成する（`auto_create = true`）ため、
/// 「フォルダがまだ無い」だけでは起動を止めない。ただしドライブや共有そのものが
/// 無い（例: 未接続の `Z:\backup`）と実行時に毎回失敗し続けるので、
/// **先祖をたどっても実在するフォルダが 1 つも無い**場合はエラーにする。
///
/// `auto_create = false` のときは従来どおり、静的部分が実在するディレクトリで
/// あることを要求する（typo による予期しない書き込みをロード時に検出したい用途）。
pub(crate) fn collect_destination_errors(
	dest: &str,
	auto_create: bool,
	rule_name: &str,
	errors: &mut Vec<String>,
) {
	let static_root = static_root_of_destination(dest);
	// "{WatchPath}/out" のように先頭からプレースホルダーで始まる場合は、
	// 展開してみないと分からないのでロード時には判定しない。
	if static_root.is_empty() {
		return;
	}

	let path = Path::new(static_root);
	if !auto_create {
		if !path.is_dir() {
			errors.push(format!(
				"監視ルール名 {} のアクションの destination(コピー先/移動先) のルート '{}' が存在しません（auto_create = true にすると実行時に自動作成できます）",
				rule_name, static_root
			));
		}
		return;
	}

	// 相対パスはプロセスの作業ディレクトリ基準になり、ここでは判定できない。
	if !path.is_absolute() {
		return;
	}
	if !has_existing_ancestor(path) {
		errors.push(format!(
			"監視ルール名 {} のアクションの destination(コピー先/移動先) '{}' は、親をたどっても実在するフォルダが見つかりません（ドライブレターやネットワーク共有名を確認してください）",
			rule_name, static_root
		));
	}
}

/// そのパス自身か、先祖のいずれかが実在するディレクトリなら true。
fn has_existing_ancestor(path: &Path) -> bool {
	path.ancestors()
		.any(|p| !p.as_os_str().is_empty() && p.is_dir())
}

/// 実行ファイルが実際に起動できる場所にあるかを検査する。
///
/// 起動時に弾かないと、検知が起きるたびに同じ失敗を繰り返すことになる。
///
/// **サービスとして動かす場合、exe の探索に使われるのはサービスの PATH
/// （システム PATH）であって、ログオンユーザーの PATH ではない。**
/// そのため「CLI では動くのにサービスでは動かない」という事故が起きる。
/// ここで検査しておけば、その食い違いを起動時点で検出できる。
fn collect_executable_errors(program: &str, rule_name: &str, label: &str, errors: &mut Vec<String>) {
	match exe_path::resolve(program) {
		exe_path::Resolved::Found(_) => {}
		exe_path::Resolved::MissingAtPath => errors.push(format!(
			"監視ルール名 {} のアクションの {} が存在しません: {}",
			rule_name, label, program
		)),
		exe_path::Resolved::NotOnPath => errors.push(format!(
			"監視ルール名 {} のアクションの {} '{}' が PATH 上で見つかりません\n    対処: フルパスで指定するか、システム PATH に追加してください\n          サービスは SYSTEM のシステム PATH を使うため、ユーザー領域に入れたもの（scoop 等）は見つかりません\n    検索した PATH: {}",
			rule_name,
			label,
			program,
			exe_path::search_path_summary()
		)),
	}
}

pub(crate) fn collect_action_errors(action: &ActionConfig, rule_name: &str, errors: &mut Vec<String>) {
	// 必須項目と「その type では効かない項目」の判定は config/action.rs の表が持つ。
	// ここに条件を書き写すと、型を足したときに片方だけ直し忘れる。
	for missing in missing_fields(action) {
		errors.push(missing.message(rule_name, action.type_));
	}
	for rejected in rejected_fields(action) {
		errors.push(rejected.message(rule_name, action.type_));
	}

	// ここから下は「書かれている値が使えるか」の検査。
	// 表では表せないので、型ごとに個別に見る。
	match action.type_ {
		ActionType::Copy | ActionType::Move => {
			if let Some(dest) = &action.destination {
				// auto_create は設定読み込み時に global の既定値が焼き込まれている。
				// 未解決（None）のまま来た場合は自動作成側を既定とする。
				collect_destination_errors(
					dest,
					action.auto_create.unwrap_or(true),
					rule_name,
					errors,
				);
			}
		}

		ActionType::Command => {
			// 起動時に弾かないと、実行時に検知のたび失敗し続けることになる。
			if let Some(shell) = &action.shell {
				if !VALID_SHELLS.contains(&shell.to_lowercase().as_str()) {
					errors.push(format!(
						"監視ルール名 {} のアクションの shell '{}' はこの OS では使用できません。{} のいずれかを指定してください",
						rule_name,
						shell,
						VALID_SHELLS.join(" / ")
					));
				} else if let Some(program) = crate::actions::command::shell_program(shell) {
					// 名前が有効でも、その実行ファイルが見つからなければ起動できない。
					collect_executable_errors(program, rule_name, &format!("shell '{shell}' の実行ファイル"), errors);
				}
			}
			collect_working_dir_errors(action, rule_name, errors);
		}

		ActionType::Execute => {
			collect_working_dir_errors(action, rule_name, errors);
			if let Some(program) = &action.program {
				// 従来は絶対パスのときだけ存在を見ていた。名前だけの指定（"pwsh" など）は
				// 素通りして実行時に初めて失敗していたので、PATH 解決まで確かめる。
				collect_executable_errors(program, rule_name, "program", errors);
			}
		}
	}
}

/// working_dir は command / execute で共通。空文字は「変更しない」の意味。
fn collect_working_dir_errors(action: &ActionConfig, rule_name: &str, errors: &mut Vec<String>) {
	let Some(dir) = &action.working_dir else { return };
	if !dir.is_empty() && !Path::new(dir).is_dir() {
		errors.push(format!("監視ルール名 {} のアクションの working_dir が存在しません: {}", rule_name, dir));
	}
}

fn collect_action_placeholder_errors(action: &ActionConfig, rule_name: &str, errors: &mut Vec<String>) {
	let fields = [
		("action.destination", &action.destination),
		("action.command", &action.command),
		("action.working_dir", &action.working_dir),
		("action.program", &action.program),
	];
	for (field_name, field_value) in fields {
		if let Some(value) = field_value {
			if let Err(e) = validate_placeholders(value, rule_name, field_name) {
				errors.push(e);
			}
		}
	}
	if let Some(args) = &action.args {
		for (index, arg) in args.iter().enumerate() {
			if let Err(e) = validate_placeholders(arg, rule_name, &format!("action.args[{}]", index)) {
				errors.push(e);
			}
		}
	}
}
