use std::collections::BTreeMap;
use std::path::Path;

use crate::error::AppError;

// CSV列インデックス
const COL_RULE_NAME: usize = 0;
const COL_ENABLED: usize = 1;
const COL_WATCH_PATH: usize = 2;
const COL_RECURSIVE: usize = 3;
const COL_TARGET: usize = 4;
const COL_INCLUDE_HIDDEN: usize = 5;
const COL_PATTERNS: usize = 6;
const COL_REGEX: usize = 7;
const COL_EXCLUDE_PATTERNS: usize = 8;
const COL_EVENTS: usize = 9;
const COL_ACTION_TYPE: usize = 10;
const COL_DESTINATION: usize = 11;
const COL_OVERWRITE: usize = 12;
const COL_PRESERVE_STRUCTURE: usize = 13;
const COL_VERIFY_INTEGRITY: usize = 14;
const COL_SHELL: usize = 15;
const COL_COMMAND: usize = 16;
const COL_PROGRAM: usize = 17;
const COL_ARGS: usize = 18;
const COL_WORKING_DIR: usize = 19;
const COL_MESSAGE: usize = 20;
// 列21以降は後から追加された列（既存CSVとの後方互換のため必ず末尾に追加する）
const COL_EXCLUDE_REGEX: usize = 21;
const COL_DIR_PATTERNS: usize = 22;
const COL_DIR_REGEX: usize = 23;
const COL_EXCLUDE_DIR_PATTERNS: usize = 24;
const COL_EXCLUDE_DIR_REGEX: usize = 25;
const COL_AUTO_CREATE: usize = 26;
const COL_DELAY_MS: usize = 27;

/// CSV の列順（先頭から）。テンプレートのヘッダー行・README・ヘルプはこれに合わせる。
/// 列を増やすときは**必ず末尾に追加**すること（既存 CSV を壊さないため）。
// 参照するのはテンプレートとの整合テストのみ。
#[cfg_attr(not(test), allow(dead_code))]
pub const COLUMNS: &[&str] = &[
    "rule_name",
    "enabled",
    "watch_path",
    "recursive",
    "target",
    "include_hidden",
    "patterns",
    "regex",
    "exclude_patterns",
    "events",
    "action_type",
    "destination",
    "overwrite",
    "preserve_structure",
    "verify_integrity",
    "shell",
    "command",
    "program",
    "args",
    "working_dir",
    "message",
    "exclude_regex",
    "dir_patterns",
    "dir_regex",
    "exclude_dir_patterns",
    "exclude_dir_regex",
    "auto_create",
    "delay_ms",
];

pub fn run(csv_path: &Path, output: Option<&Path>) -> Result<(), AppError> {
    let content = std::fs::read_to_string(csv_path)
        .map_err(|e| AppError::Config(format!("CSV ファイルの読み込みに失敗: {e}")))?;

    let toml = convert(&content)?;

    if let Some(out_path) = output {
        std::fs::write(out_path, &toml)
            .map_err(|e| AppError::Config(format!("出力ファイルの書き込みに失敗: {e}")))?;
        let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        println!("[{ts}] [INFO]    変換完了: {}", out_path.display());
    } else {
        print!("{toml}");
    }

    Ok(())
}

/// CSV の中身を rules.toml の文字列に変換する（ファイル入出力を含まない本体）。
pub fn convert(content: &str) -> Result<String, AppError> {
    let rows = parse_csv(content);
    if rows.is_empty() {
        return Err(AppError::Config("CSV が空です".to_string()));
    }

    // ヘッダー行スキップ（1行目がヘッダーの場合）
    let data_start = if rows[0].first().map(|s| s.as_str()) == Some("rule_name") {
        1
    } else {
        0
    };

    // rule_name をキーにしてルールをグループ化（順序保持のため BTreeMap）
    let mut rule_order: Vec<String> = Vec::new();
    let mut rule_map: BTreeMap<String, Vec<Vec<String>>> = BTreeMap::new();

    for row in &rows[data_start..] {
        if row.is_empty() || row.iter().all(|s| s.is_empty()) {
            continue;
        }
        let name = get(row, COL_RULE_NAME);
        if name.is_empty() {
            continue;
        }
        if !rule_map.contains_key(&name) {
            rule_order.push(name.clone());
        }
        rule_map.entry(name).or_default().push(row.clone());
    }

    if rule_order.is_empty() {
        return Err(AppError::Config("有効なルールが1件もありません".to_string()));
    }

    let mut toml = String::new();

    for rule_name in &rule_order {
        let rows = &rule_map[rule_name];
        let first = &rows[0];

        let enabled = bool_field(&get(first, COL_ENABLED), true, rule_name, "enabled")?;
        let watch_path = get(first, COL_WATCH_PATH);
        let recursive = bool_field(&get(first, COL_RECURSIVE), false, rule_name, "recursive")?;
        let target = get(first, COL_TARGET);
        let target = if target.is_empty() { "file".to_string() } else { target };
        let include_hidden =
            bool_field(&get(first, COL_INCLUDE_HIDDEN), false, rule_name, "include_hidden")?;
        let patterns = get(first, COL_PATTERNS);
        let regex = get(first, COL_REGEX);
        let exclude_patterns = get(first, COL_EXCLUDE_PATTERNS);
        let exclude_regex = get(first, COL_EXCLUDE_REGEX);
        let dir_patterns = get(first, COL_DIR_PATTERNS);
        let dir_regex = get(first, COL_DIR_REGEX);
        let exclude_dir_patterns = get(first, COL_EXCLUDE_DIR_PATTERNS);
        let exclude_dir_regex = get(first, COL_EXCLUDE_DIR_REGEX);
        let events = get(first, COL_EVENTS);

        if watch_path.is_empty() {
            return Err(AppError::Config(format!(
                "ルール '{rule_name}' の watch_path が空です"
            )));
        }
        if events.is_empty() {
            return Err(AppError::Config(format!(
                "ルール '{rule_name}' の events が空です"
            )));
        }
        if patterns.is_empty() && regex.is_empty() {
            return Err(AppError::Config(format!(
                "ルール '{rule_name}' の patterns または regex のどちらかを指定してください"
            )));
        }
        if !patterns.is_empty() && !regex.is_empty() {
            return Err(AppError::Config(format!(
                "ルール '{rule_name}' の patterns と regex は同時指定できません"
            )));
        }
        if !exclude_patterns.is_empty() && !exclude_regex.is_empty() {
            return Err(AppError::Config(format!(
                "ルール '{rule_name}' の exclude_patterns と exclude_regex は同時指定できません"
            )));
        }
        if !dir_patterns.is_empty() && !dir_regex.is_empty() {
            return Err(AppError::Config(format!(
                "ルール '{rule_name}' の dir_patterns と dir_regex は同時指定できません"
            )));
        }
        if !exclude_dir_patterns.is_empty() && !exclude_dir_regex.is_empty() {
            return Err(AppError::Config(format!(
                "ルール '{rule_name}' の exclude_dir_patterns と exclude_dir_regex は同時指定できません"
            )));
        }

        toml.push_str("[[rules]]\n");
        toml.push_str(&format!("enabled = {enabled}\n"));
        toml.push_str(&format!("name    = {}\n", toml_str(rule_name)));
        toml.push('\n');
        toml.push_str("[rules.watch]\n");
        toml.push_str(&format!("path             = {}\n", toml_str(&watch_path)));
        toml.push_str(&format!("recursive        = {recursive}\n"));
        toml.push_str(&format!("target           = {}\n", toml_str(&target)));
        toml.push_str(&format!("include_hidden   = {include_hidden}\n"));

        if !patterns.is_empty() {
            toml.push_str(&format!("patterns         = {}\n", toml_str_array(&patterns)));
        }
        if !regex.is_empty() {
            toml.push_str(&format!("regex            = {}\n", toml_str(&regex)));
        }

        let excl = if exclude_patterns.is_empty() {
            "[]".to_string()
        } else {
            toml_str_array(&exclude_patterns)
        };
        toml.push_str(&format!("exclude_patterns = {excl}\n"));

        if !exclude_regex.is_empty() {
            toml.push_str(&format!("exclude_regex    = {}\n", toml_str(&exclude_regex)));
        }
        if !dir_patterns.is_empty() {
            toml.push_str(&format!("dir_patterns     = {}\n", toml_str_array(&dir_patterns)));
        }
        if !dir_regex.is_empty() {
            toml.push_str(&format!("dir_regex        = {}\n", toml_str(&dir_regex)));
        }
        if !exclude_dir_patterns.is_empty() {
            toml.push_str(&format!(
                "exclude_dir_patterns = {}\n",
                toml_str_array(&exclude_dir_patterns)
            ));
        }
        if !exclude_dir_regex.is_empty() {
            toml.push_str(&format!("exclude_dir_regex = {}\n", toml_str(&exclude_dir_regex)));
        }

        toml.push_str(&format!("events           = {}\n", toml_str_array(&events)));
        toml.push('\n');

        for row in rows {
            let action_type = get(row, COL_ACTION_TYPE);
            if action_type.is_empty() {
                return Err(AppError::Config(format!(
                    "ルール '{rule_name}' の action_type が空です"
                )));
            }

            toml.push_str("[[rules.actions]]\n");
            toml.push_str(&format!("type = {}\n", toml_str(&action_type)));

            match action_type.to_lowercase().as_str() {
                "copy" | "move" => {
                    let dest = get(row, COL_DESTINATION);
                    let overwrite =
                        bool_field(&get(row, COL_OVERWRITE), false, rule_name, "overwrite")?;
                    let preserve = bool_field(
                        &get(row, COL_PRESERVE_STRUCTURE),
                        false,
                        rule_name,
                        "preserve_structure",
                    )?;
                    let verify = bool_field(
                        &get(row, COL_VERIFY_INTEGRITY),
                        false,
                        rule_name,
                        "verify_integrity",
                    )?;
                    toml.push_str(&format!("destination        = {}\n", toml_str(&dest)));
                    toml.push_str(&format!("overwrite          = {overwrite}\n"));
                    toml.push_str(&format!("preserve_structure = {preserve}\n"));
                    toml.push_str(&format!("verify_integrity   = {verify}\n"));
                    // 空欄なら global.toml の [destination] auto_create に従う
                    let auto_create = get(row, COL_AUTO_CREATE);
                    if !auto_create.trim().is_empty() {
                        let v = bool_field(&auto_create, true, rule_name, "auto_create")?;
                        toml.push_str(&format!("auto_create        = {v}\n"));
                    }
                }
                "command" => {
                    let shell = get(row, COL_SHELL);
                    let command = get(row, COL_COMMAND);
                    let working_dir = get(row, COL_WORKING_DIR);
                    toml.push_str(&format!("shell       = {}\n", toml_str(&shell)));
                    toml.push_str(&format!("command     = {}\n", toml_str(&command)));
                    toml.push_str(&format!("working_dir = {}\n", toml_str(&working_dir)));
                }
                "execute" => {
                    let program = get(row, COL_PROGRAM);
                    let args = get(row, COL_ARGS);
                    let working_dir = get(row, COL_WORKING_DIR);
                    toml.push_str(&format!("program     = {}\n", toml_str(&program)));
                    let args_toml = if args.is_empty() {
                        "[]".to_string()
                    } else {
                        toml_str_array(&args)
                    };
                    toml.push_str(&format!("args        = {args_toml}\n"));
                    toml.push_str(&format!("working_dir = {}\n", toml_str(&working_dir)));
                }
                "log" => {
                    let message = get(row, COL_MESSAGE);
                    toml.push_str(&format!("message = {}\n", toml_str(&message)));
                }
                other => {
                    return Err(AppError::Config(format!(
                        "ルール '{rule_name}' の action_type '{other}' は不明です (copy / move / command / execute / log)"
                    )));
                }
            }

            // アクション共通: 実行前の待ち時間（空欄なら出力しない = 0）
            if let Some(delay) = u64_field(&get(row, COL_DELAY_MS), rule_name, "delay_ms")? {
                toml.push_str(&format!("delay_ms = {delay}\n"));
            }
            toml.push('\n');
        }
    }

    Ok(toml)
}

fn get(row: &[String], col: usize) -> String {
    row.get(col).cloned().unwrap_or_default()
}

/// TOML の基本文字列（`"..."`）として安全な形に整える。
///
/// Windows パス（`C:\tool\app.exe`）をそのまま埋め込むと `\t` が
/// タブのエスケープと解釈されて壊れた TOML になるため、必ずここを通す。
fn toml_str(s: &str) -> String {
    let escaped = s
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\t', "\\t")
        .replace('\r', "\\r")
        .replace('\n', "\\n");
    format!("\"{escaped}\"")
}

/// `|` 区切りのセルを TOML の文字列配列にする。
fn toml_str_array(field: &str) -> String {
    let items: Vec<String> = field.split('|').map(|s| toml_str(s.trim())).collect();
    format!("[{}]", items.join(", "))
}

/// bool 列を TOML の `true` / `false` に正規化する。
///
/// Excel は論理値を `TRUE` / `FALSE`（大文字）で書き出すため、そのまま
/// 埋め込むと TOML パースに失敗する。`1` / `0` / `yes` / `no` も受ける。
fn bool_field(value: &str, default: bool, rule_name: &str, column: &str) -> Result<bool, AppError> {
    let v = value.trim();
    if v.is_empty() {
        return Ok(default);
    }
    match v.to_lowercase().as_str() {
        "true" | "1" | "yes" | "y" | "on" => Ok(true),
        "false" | "0" | "no" | "n" | "off" => Ok(false),
        other => Err(AppError::Config(format!(
            "ルール '{rule_name}' の {column} は true / false で指定してください（'{other}' は解釈できません）"
        ))),
    }
}

/// 数値列を読む。空なら None。
fn u64_field(value: &str, rule_name: &str, column: &str) -> Result<Option<u64>, AppError> {
    let v = value.trim();
    if v.is_empty() {
        return Ok(None);
    }
    v.parse::<u64>().map(Some).map_err(|_| {
        AppError::Config(format!(
            "ルール '{rule_name}' の {column} は 0 以上の整数で指定してください（'{v}' は解釈できません）"
        ))
    })
}

fn parse_csv(content: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    for line in content.lines() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        let mut fields = Vec::new();
        let mut field = String::new();
        let mut in_quotes = false;
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            match chars[i] {
                '"' if in_quotes && chars.get(i + 1) == Some(&'"') => {
                    field.push('"');
                    i += 2;
                }
                '"' => {
                    in_quotes = !in_quotes;
                    i += 1;
                }
                ',' if !in_quotes => {
                    fields.push(field.trim().to_string());
                    field = String::new();
                    i += 1;
                }
                c => {
                    field.push(c);
                    i += 1;
                }
            }
        }
        fields.push(field.trim().to_string());
        rows.push(fields);
    }
    rows
}

#[cfg(test)]
#[path = "tests/csv_import.rs"]
mod tests;
