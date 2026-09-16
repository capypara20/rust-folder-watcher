//! 設定（global.toml / rules.toml）の意味的バリデーション。
//!
//! 見つけた問題は [`Problem`]（場所・内容・対処）として積む。
//! 文の組み立てと字下げは `Problem` の表示側が 1 か所で行うので、
//! ここでは「どこの」「何が」「どうすればよいか」だけを書く。
//! 文言の約束は `config/problem.rs` の先頭を参照。

use std::path::Path;

use globset::Glob;
use regex::Regex;

use super::action::{missing_fields, rejected_fields};
use super::model::{ActionConfig, GlobalConfig, RulesConfig};
use super::problem::{Location, Problem, RuleRef};
use super::types::ActionType;
use crate::actions::command::VALID_SHELLS;
use crate::exe_path;
use crate::placeholder::{find_any_placeholder, find_unknown_placeholder, VALID_PLACEHOLDERS};

/// 検証の結果。問題が 1 件も無ければ `Ok`。
///
/// どのファイルの問題かはここでは分からない（設定の値だけを見ている）ので、
/// ファイルパスを付けて `AppError` にするのは読み込み側（`config::load`）の役目。
pub type Problems = Vec<Problem>;

pub(crate) fn finish_validation(errors: Problems) -> Result<(), Problems> {
	if errors.is_empty() {
		Ok(())
	} else {
		Err(errors)
	}
}

/* ---- global.toml ------------------------------------------ */

pub fn validate_global_config(config: &GlobalConfig) -> Result<(), Problems> {
	let mut errors = Vec::new();
	collect_log_target_errors(
		&config.system_log.dir,
		&config.system_log.file_name,
		|key| Location::Global(format!("system_log.{key}")),
		&mut errors,
	);
	if let Some(dashboard) = &config.dashboard {
		if dashboard.enabled && dashboard.bind.parse::<std::net::SocketAddr>().is_err() {
			errors.push(
				Problem::new(
					Location::Global("dashboard.bind".into()),
					format!("'{}' はアドレスとポートとして解釈できません", dashboard.bind),
				)
				.with_hint("「IP アドレス:ポート番号」の形で指定してください（例: 127.0.0.1:8080）"),
			);
		}
	}
	// 0 を渡すと tokio のタイマーが作れずパニックするため、ここで止める。
	if config.poll_interval_ms() == 0 {
		errors.push(
			Problem::new(Location::Global("detect.poll_interval_ms".into()), "0 は指定できません")
				.with_hint("1 以上の値を指定してください"),
		);
	}
	finish_validation(errors)
}

/// ログの出力先フォルダとファイル名を検証する。system_log と、ルールごとのログで共用する。
///
/// `at` は `"dir"` / `"file_name"` を受け取り、そのキーの場所を返す。
fn collect_log_target_errors(
	dir: &str,
	file_name: &str,
	at: impl Fn(&str) -> Location,
	errors: &mut Problems,
) {
	if dir.trim().is_empty() {
		errors.push(
			Problem::new(at("dir"), "空になっています")
				.with_hint("ログを書き出すフォルダを指定してください"),
		);
	} else {
		let dir_path = Path::new(dir);
		if !dir_path.exists() {
			errors.push(
				Problem::new(at("dir"), format!("フォルダ '{dir}' が存在しません"))
					.with_hint("先にフォルダを作成するか、存在するフォルダを指定してください"),
			);
		} else if !dir_path.is_dir() {
			errors.push(Problem::new(at("dir"), format!("'{dir}' はフォルダではありません")));
		}
	}

	if file_name.trim().is_empty() {
		errors.push(
			Problem::new(at("file_name"), "空になっています")
				.with_hint("ログのファイル名を指定してください"),
		);
		return;
	}
	let valid = ["Date", "DateTime"];
	let re = Regex::new(r"\{([A-Za-z]+)\}").unwrap();
	for caps in re.captures_iter(file_name) {
		let name = &caps[1];
		if !valid.contains(&name) {
			errors.push(
				Problem::new(at("file_name"), format!("{{{name}}} はログのファイル名には使えません"))
					.with_hint("使えるのは {Date} と {DateTime} です"),
			);
		}
	}
}

/* ---- rules.toml ------------------------------------------- */

pub fn validate_rules_config(config: &RulesConfig) -> Result<(), Problems> {
	let mut errors = Vec::new();
	let rules = &config.rules;

	if rules.is_empty() {
		errors.push(
			Problem::new(Location::File, "ルールが 1 つもありません")
				.with_hint("[[rules]] を 1 つ以上定義してください"),
		);
		return finish_validation(errors);
	}

	for (i, rule) in rules.iter().enumerate() {
		let rule_ref = RuleRef::new(&rule.name, i + 1);
		let at = |key: &str| Location::Rule { rule: rule_ref.clone(), key: key.to_string() };

		if rule.name.trim().is_empty() {
			errors.push(
				Problem::new(at("name"), "空になっています")
					.with_hint("ルールを見分けられる名前を付けてください"),
			);
		}
		if rule.actions.is_empty() {
			errors.push(
				Problem::new(at("actions"), "アクションが 1 つもありません")
					.with_hint("[[rules.actions]] を 1 つ以上定義してください"),
			);
		}
		if !Path::new(&rule.watch.path).is_dir() {
			errors.push(
				Problem::new(at("watch.path"), format!("フォルダ '{}' が存在しません", rule.watch.path))
					.with_hint("監視するフォルダのパスが正しいか確認してください"),
			);
		}

		if rule.watch.events.is_empty() {
			errors.push(
				Problem::new(at("watch.events"), "空になっています")
					.with_hint("検知するイベント（create / modify / delete / rename）を 1 つ以上指定してください"),
			);
		}
		match (rule.watch.patterns.is_some(), rule.watch.regex.is_some()) {
			(true, true) => errors.push(
				Problem::new(at("watch.patterns"), "watch.patterns と watch.regex が両方指定されています")
					.with_hint("どちらか一方だけにしてください"),
			),
			(false, false) => errors.push(
				Problem::new(at("watch.patterns"), "watch.patterns と watch.regex のどちらも指定されていません")
					.with_hint("どちらか一方を指定してください（すべて対象にするなら patterns = [\"*\"]）"),
			),
			_ => {}
		}

		// glob / 正規表現の構文チェックと、glob 列と正規表現の排他チェック。
		// errors に積む順番がそのままエラー表示の順番になるので、並びは変えないこと。
		collect_glob_errors(rule.watch.patterns.as_deref().unwrap_or(&[]), &at("watch.patterns"), &mut errors);
		collect_regex_errors(rule.watch.regex.as_deref(), &at("watch.regex"), &mut errors);

		for (glob_key, regex_key, globs, regex) in [
			(
				"watch.exclude_patterns",
				"watch.exclude_regex",
				&rule.watch.exclude_patterns,
				rule.watch.exclude_regex.as_deref(),
			),
			(
				"watch.exclude_dir_patterns",
				"watch.exclude_dir_regex",
				&rule.watch.exclude_dir_patterns,
				rule.watch.exclude_dir_regex.as_deref(),
			),
			(
				"watch.dir_patterns",
				"watch.dir_regex",
				&rule.watch.dir_patterns,
				rule.watch.dir_regex.as_deref(),
			),
		] {
			if !globs.is_empty() && regex.is_some() {
				errors.push(
					Problem::new(at(glob_key), format!("{glob_key} と {regex_key} が両方指定されています"))
						.with_hint("どちらか一方だけにしてください"),
				);
			}
			collect_glob_errors(globs, &at(glob_key), &mut errors);
			collect_regex_errors(regex, &at(regex_key), &mut errors);
		}

		if let Some(rule_log) = &rule.log {
			for (section, target) in [("detect", &rule_log.detect), ("action", &rule_log.action)] {
				let Some(target) = target else { continue };
				if !target.enabled {
					continue;
				}
				collect_log_target_errors(
					&target.dir,
					&target.file_name,
					|key| at(&format!("log.{section}.{key}")),
					&mut errors,
				);
			}
		}

		// アクションはルールの項目の後に並べる（設定ファイルに書く順と同じ）。
		for (j, action) in rule.actions.iter().enumerate() {
			collect_action_errors(action, &rule_ref, j + 1, &mut errors);
			collect_action_placeholder_errors(action, &rule_ref, j + 1, &mut errors);
		}
	}

	finish_validation(errors)
}

/// glob パターン列の構文を検査する。
fn collect_glob_errors(patterns: &[String], at: &Location, errors: &mut Problems) {
	for pattern in patterns {
		if let Err(e) = Glob::new(pattern) {
			errors.push(Problem::new(at.clone(), format!("'{pattern}' は glob パターンとして正しくありません: {e}")));
		}
	}
}

/// 正規表現の構文を検査する。
fn collect_regex_errors(pattern: Option<&str>, at: &Location, errors: &mut Problems) {
	if let Some(pattern) = pattern {
		if let Err(e) = Regex::new(pattern) {
			errors.push(Problem::new(at.clone(), format!("'{pattern}' は正規表現として正しくありません: {e}")));
		}
	}
}

/* ---- アクション ------------------------------------------- */

/// 1 つのアクションを検証する。`index` は 1 始まりの番号。
pub(crate) fn collect_action_errors(action: &ActionConfig, rule: &RuleRef, index: usize, errors: &mut Problems) {
	let at = |key: &str| Location::Action { rule: rule.clone(), index, key: key.to_string() };

	// 必須項目と「その type では効かない項目」の判定は config/action.rs の表が持つ。
	// ここに条件を書き写すと、型を足したときに片方だけ直し忘れる。
	for missing in missing_fields(action) {
		errors.push(missing.problem(rule.clone(), index, action.type_));
	}
	for rejected in rejected_fields(action) {
		errors.push(rejected.problem(rule.clone(), index, action.type_));
	}

	// ここから下は「書かれている値が使えるか」の検査。
	// 表では表せないので、型ごとに個別に見る。
	match action.type_ {
		ActionType::Copy | ActionType::Move => {
			if let Some(dest) = &action.destination {
				// auto_create は設定読み込み時に global の既定値が焼き込まれている。
				// 未解決（None）のまま来た場合は自動作成側を既定とする。
				if let Some(problem) = check_destination(dest, action.auto_create.unwrap_or(true), at("destination")) {
					errors.push(problem);
				}
			}
		}

		ActionType::Command => {
			// 起動時に弾かないと、実行時に検知のたび失敗し続けることになる。
			if let Some(shell) = &action.shell {
				if !VALID_SHELLS.contains(&shell.to_lowercase().as_str()) {
					errors.push(
						Problem::new(at("shell"), format!("'{shell}' はこの OS では使えません"))
							.with_hint(format!("{} のいずれかを指定してください", VALID_SHELLS.join(" / "))),
					);
				} else if let Some(program) = crate::actions::command::shell_program(shell) {
					// 名前が有効でも、その実行ファイルが見つからなければ起動できない。
					let subject = format!("シェル '{shell}' の実行ファイル '{program}'");
					if let Some(problem) = check_executable(program, &subject, at("shell")) {
						errors.push(problem);
					}
				}
			}
			collect_working_dir_errors(action, at("working_dir"), errors);
		}

		ActionType::Execute => {
			collect_working_dir_errors(action, at("working_dir"), errors);
			// プレースホルダーを書いた場合は「置き換えられない」の方で知らせる（二重に出さない）。
			if let Some(program) = action.program.as_ref().filter(|p| find_any_placeholder(p).is_none()) {
				// 名前だけの指定（"pwsh" など）も、実行時に初めて失敗しないよう PATH 解決まで確かめる。
				if let Some(problem) = check_executable(program, &format!("'{program}'"), at("program")) {
					errors.push(problem);
				}
			}
		}
	}
}

/// working_dir は command / execute で共通。空文字は「変更しない」の意味。
fn collect_working_dir_errors(action: &ActionConfig, at: Location, errors: &mut Problems) {
	let Some(dir) = &action.working_dir else { return };
	// プレースホルダーを書いた場合は「置き換えられない」の方で知らせる。
	// ここでも「存在しません」を出すと、同じ原因で 2 件になって紛らわしい。
	if find_any_placeholder(dir).is_some() {
		return;
	}
	if !dir.is_empty() && !Path::new(dir).is_dir() {
		errors.push(
			Problem::new(at, format!("フォルダ '{dir}' が存在しません"))
				.with_hint("存在するフォルダを指定するか、変更しない場合は空文字 \"\" にしてください"),
		);
	}
}

/// プレースホルダーの書き方を検査する。
///
/// 実行時にプレースホルダーを展開するのは `destination` / `command` / `args` だけ。
/// `program` と `working_dir` は書いた文字列のまま使うので、`{FullName}` などを
/// 書いても置き換わらない。以前はここも「使える名前か」だけを見ていたため、
/// 置き換わらないのに検査を通ってしまっていた。
pub(crate) fn collect_action_placeholder_errors(action: &ActionConfig, rule: &RuleRef, index: usize, errors: &mut Problems) {
	let at = |key: &str| Location::Action { rule: rule.clone(), index, key: key.to_string() };

	// 展開する項目: 使える名前だけか
	let hint = format!(
		"使えるのは {} です",
		VALID_PLACEHOLDERS.iter().map(|n| format!("{{{n}}}")).collect::<Vec<_>>().join(" ")
	);
	let mut expanded: Vec<(String, &str)> = [("destination", &action.destination), ("command", &action.command)]
		.into_iter()
		.filter_map(|(key, value)| value.as_deref().map(|v| (key.to_string(), v)))
		.collect();
	if let Some(args) = &action.args {
		// args は 1 始まりで数える（actions の番号と揃える）
		expanded.extend(args.iter().enumerate().map(|(i, arg)| (format!("args[{}]", i + 1), arg.as_str())));
	}
	for (key, value) in expanded {
		if let Some(name) = find_unknown_placeholder(value) {
			errors.push(
				Problem::new(at(&key), format!("{{{name}}} は使えないプレースホルダーです")).with_hint(hint.clone()),
			);
		}
	}

	// 展開しない項目: そもそも書けない
	for (key, value) in [("program", &action.program), ("working_dir", &action.working_dir)] {
		let Some(value) = value else { continue };
		if let Some(name) = find_any_placeholder(value) {
			errors.push(
				Problem::new(at(key), format!("{{{name}}} は {key} では置き換えられません"))
					.with_hint("プレースホルダーが使えるのは destination / command / args です。この項目には値をそのまま書いてください"),
			);
		}
	}
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
/// 無いと実行時に毎回失敗し続けるので、
/// **先祖をたどっても実在するフォルダが 1 つも無い**場合はエラーにする。
///
/// `auto_create = false` のときは、静的部分が実在するディレクトリで
/// あることを要求する（typo による予期しない書き込みをロード時に検出したい用途）。
fn check_destination(dest: &str, auto_create: bool, at: Location) -> Option<Problem> {
	let static_root = static_root_of_destination(dest);
	// "{WatchPath}/out" のように先頭からプレースホルダーで始まる場合は、
	// 展開してみないと分からないのでロード時には判定しない。
	if static_root.is_empty() {
		return None;
	}

	let path = Path::new(static_root);
	if !auto_create {
		return (!path.is_dir()).then(|| {
			Problem::new(at, format!("フォルダ '{static_root}' が存在しません")).with_hint(
				"先にフォルダを作成してください。実行時に自動で作らせるなら auto_create = true にしてください",
			)
		});
	}

	// 相対パスはプロセスの作業ディレクトリ基準になり、ここでは判定できない。
	if !path.is_absolute() || has_existing_ancestor(path) {
		return None;
	}
	Some(
		Problem::new(at, format!("'{static_root}' は、親をたどっても存在するフォルダがありません"))
			.with_hint(DESTINATION_ROOT_HINT),
	)
}

/// 親をたどっても何も無い、というのは先頭部分の書き間違いがほとんど。
/// Windows にはドライブと共有名があるので、それを具体的に挙げる。
#[cfg(windows)]
const DESTINATION_ROOT_HINT: &str = "ドライブ名やネットワーク共有名が正しいか確認してください";
#[cfg(not(windows))]
const DESTINATION_ROOT_HINT: &str = "パスの先頭部分が正しいか確認してください";

/// そのパス自身か、先祖のいずれかが実在するディレクトリなら true。
fn has_existing_ancestor(path: &Path) -> bool {
	path.ancestors().any(|p| !p.as_os_str().is_empty() && p.is_dir())
}

/// 実行ファイルが実際に起動できる場所にあるかを検査する。
///
/// `subject` は内容の文頭に置く「何が」（例: `'tool'`、`シェル 'pwsh' の実行ファイル 'pwsh.exe'`）。
fn check_executable(program: &str, subject: &str, at: Location) -> Option<Problem> {
	match exe_path::resolve(program) {
		exe_path::Resolved::Found(_) => None,
		exe_path::Resolved::MissingAtPath => Some(
			Problem::new(at, format!("{subject} が存在しません")).with_hint("パスが正しいか確認してください"),
		),
		// ファイルは在るので「存在しない」とは言わない。対処がまったく違う。
		exe_path::Resolved::NotExecutable(found) => {
			// 名前だけの指定（PATH で見つけた）ときは、どのファイルかを添える。
			// パスで指定したときは subject と同じになるので書かない。
			let message = if found == Path::new(program) {
				format!("{subject} に実行権限がありません")
			} else {
				format!("{subject} に実行権限がありません: {}", crate::path_fmt::for_log(&found))
			};
			Some(
				Problem::new(at, message)
					.with_hint("ファイルに実行権限を付けてください（例: chmod +x <ファイル>）"),
			)
		}
		exe_path::Resolved::NotOnPath => Some(
			Problem::new(
				at,
				format!("{subject} が PATH 上に見つかりません\n{}", exe_path::search_path_summary()),
			)
			.with_hint(NOT_ON_PATH_HINT),
		),
	}
}

/// PATH 上に見つからないときの対処。
///
/// Windows のサービスは、実行ファイルをサービスの実行アカウントの PATH から探す。
/// ログオンしているユーザーの PATH ではないので、「CLI では動くのにサービスでは
/// 見つからない」が起きる。これは Windows のサービス一般の事実なので書き分ける。
#[cfg(windows)]
const NOT_ON_PATH_HINT: &str = "フルパスで指定するか、PATH に含まれるフォルダに置いてください\n\
	サービスとして動かす場合は、サービスの実行アカウントの PATH から探されます";
#[cfg(not(windows))]
const NOT_ON_PATH_HINT: &str = "フルパスで指定するか、PATH に含まれるフォルダに置いてください";
