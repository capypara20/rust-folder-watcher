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
// 列21以降は後から追加されたフィルター列（既存CSVとの後方互換のため末尾に追加）
const COL_EXCLUDE_REGEX: usize = 21;
const COL_DIR_PATTERNS: usize = 22;
const COL_DIR_REGEX: usize = 23;
const COL_EXCLUDE_DIR_PATTERNS: usize = 24;
const COL_EXCLUDE_DIR_REGEX: usize = 25;

pub fn run(csv_path: &Path, output: Option<&Path>) -> Result<(), AppError> {
    let content = std::fs::read_to_string(csv_path)
        .map_err(|e| AppError::Config(format!("CSV ファイルの読み込みに失敗: {e}")))?;

    let rows = parse_csv(&content);
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

        let enabled = get(first, COL_ENABLED);
        let enabled = if enabled.is_empty() { "true" } else { &enabled };
        let watch_path = get(first, COL_WATCH_PATH);
        let recursive = get(first, COL_RECURSIVE);
        let recursive = if recursive.is_empty() { "false" } else { &recursive };
        let target = get(first, COL_TARGET);
        let target = if target.is_empty() { "file" } else { &target };
        let include_hidden = get(first, COL_INCLUDE_HIDDEN);
        let include_hidden = if include_hidden.is_empty() { "false" } else { &include_hidden };
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
        toml.push_str(&format!("name    = \"{rule_name}\"\n"));
        toml.push('\n');
        toml.push_str("[rules.watch]\n");
        toml.push_str(&format!("path             = \"{}\"\n", escape_toml_str(&watch_path)));
        toml.push_str(&format!("recursive        = {recursive}\n"));
        toml.push_str(&format!("target           = \"{target}\"\n"));
        toml.push_str(&format!("include_hidden   = {include_hidden}\n"));

        if !patterns.is_empty() {
            let pats: Vec<String> = patterns.split('|').map(|s| format!("\"{}\"", s.trim())).collect();
            toml.push_str(&format!("patterns         = [{}]\n", pats.join(", ")));
        }
        if !regex.is_empty() {
            toml.push_str(&format!("regex            = \"{}\"\n", escape_toml_str(&regex)));
        }

        let excl: Vec<String> = if exclude_patterns.is_empty() {
            vec![]
        } else {
            exclude_patterns.split('|').map(|s| format!("\"{}\"", s.trim())).collect()
        };
        toml.push_str(&format!("exclude_patterns = [{}]\n", excl.join(", ")));

        if !exclude_regex.is_empty() {
            toml.push_str(&format!("exclude_regex    = \"{}\"\n", escape_toml_str(&exclude_regex)));
        }
        if !dir_patterns.is_empty() {
            let pats: Vec<String> = dir_patterns.split('|').map(|s| format!("\"{}\"", escape_toml_str(s.trim()))).collect();
            toml.push_str(&format!("dir_patterns     = [{}]\n", pats.join(", ")));
        }
        if !dir_regex.is_empty() {
            toml.push_str(&format!("dir_regex        = \"{}\"\n", escape_toml_str(&dir_regex)));
        }
        if !exclude_dir_patterns.is_empty() {
            let pats: Vec<String> = exclude_dir_patterns.split('|').map(|s| format!("\"{}\"", escape_toml_str(s.trim()))).collect();
            toml.push_str(&format!("exclude_dir_patterns = [{}]\n", pats.join(", ")));
        }
        if !exclude_dir_regex.is_empty() {
            toml.push_str(&format!("exclude_dir_regex = \"{}\"\n", escape_toml_str(&exclude_dir_regex)));
        }

        let evts: Vec<String> = events.split('|').map(|s| format!("\"{}\"", s.trim())).collect();
        toml.push_str(&format!("events           = [{}]\n", evts.join(", ")));
        toml.push('\n');

        for row in rows {
            let action_type = get(row, COL_ACTION_TYPE);
            if action_type.is_empty() {
                return Err(AppError::Config(format!(
                    "ルール '{rule_name}' の action_type が空です"
                )));
            }

            toml.push_str("[[rules.actions]]\n");
            toml.push_str(&format!("type = \"{action_type}\"\n"));

            match action_type.as_str() {
                "copy" | "move" => {
                    let dest = get(row, COL_DESTINATION);
                    let overwrite = get(row, COL_OVERWRITE);
                    let overwrite = if overwrite.is_empty() { "false" } else { &overwrite };
                    let preserve = get(row, COL_PRESERVE_STRUCTURE);
                    let preserve = if preserve.is_empty() { "false" } else { &preserve };
                    let verify = get(row, COL_VERIFY_INTEGRITY);
                    let verify = if verify.is_empty() { "false" } else { &verify };
                    toml.push_str(&format!("destination        = \"{}\"\n", escape_toml_str(&dest)));
                    toml.push_str(&format!("overwrite          = {overwrite}\n"));
                    toml.push_str(&format!("preserve_structure = {preserve}\n"));
                    toml.push_str(&format!("verify_integrity   = {verify}\n"));
                }
                "command" => {
                    let shell = get(row, COL_SHELL);
                    let command = get(row, COL_COMMAND);
                    let working_dir = get(row, COL_WORKING_DIR);
                    toml.push_str(&format!("shell       = \"{shell}\"\n"));
                    toml.push_str(&format!("command     = \"{}\"\n", escape_toml_str(&command)));
                    toml.push_str(&format!("working_dir = \"{}\"\n", escape_toml_str(&working_dir)));
                }
                "execute" => {
                    let program = get(row, COL_PROGRAM);
                    let args = get(row, COL_ARGS);
                    let working_dir = get(row, COL_WORKING_DIR);
                    toml.push_str(&format!("program     = \"{}\"\n", escape_toml_str(&program)));
                    if args.is_empty() {
                        toml.push_str("args        = []\n");
                    } else {
                        let arg_list: Vec<String> = args.split('|').map(|s| format!("\"{}\"", s.trim())).collect();
                        toml.push_str(&format!("args        = [{}]\n", arg_list.join(", ")));
                    }
                    toml.push_str(&format!("working_dir = \"{}\"\n", escape_toml_str(&working_dir)));
                }
                "log" => {
                    let message = get(row, COL_MESSAGE);
                    toml.push_str(&format!("message = \"{}\"\n", escape_toml_str(&message)));
                }
                other => {
                    return Err(AppError::Config(format!(
                        "ルール '{rule_name}' の action_type '{other}' は不明です (copy / move / command / execute / log)"
                    )));
                }
            }
            toml.push('\n');
        }
    }

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

fn get(row: &[String], col: usize) -> String {
    row.get(col).cloned().unwrap_or_default()
}

fn escape_toml_str(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
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
