use std::path::Path;
use std::sync::LazyLock;

use crate::error::AppError;
use chrono::Local;
use regex::Regex;

pub struct PlaceholderContext {
    pub full_name: String,      //{FullName}		= 絶対パス
    pub directory_name: String, //{DirectoryName}	= 絶対パスからファイル名を除いた部分
    pub name: String,           //{Name}			= ファイル名
    pub base_name: String,      //{BaseName}		= 拡張子を除いたファイル名
    pub extension: String,      //{Extension}		= ファイルの拡張子
    pub relative_path: String,  //{RelativePath}	= 相対パス
    pub watch_path: String,     //{WatchPath}		= 監視対象のパス
    pub destination: String,    //{Destination}		= コピー先/移動先のパス
    pub date: String,           //{Date}			= 日付
    pub time: String,           //{Time}			= 時刻
    pub datetime: String,       //{DateTime}		= 日付と時刻
}

impl PlaceholderContext {
    pub fn new(full_path: &Path, watch_path: &Path, destination: &str) -> Self {
        let now = Local::now();
        Self {
            full_name: full_path.to_string_lossy().replace('\\', "/"),
            directory_name: full_path
                .parent()
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default(),
            name: full_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            base_name: full_path
                .file_stem()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            extension: full_path
                .extension()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            relative_path: full_path
                .strip_prefix(watch_path)
                .map(|n| n.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default(),
            watch_path: watch_path.to_string_lossy().replace('\\', "/"),
            destination: destination.replace('\\', "/"),
            date: now.format("%Y%m%d").to_string(),
            time: now.format("%H%M%S").to_string(),
            datetime: now.format("%Y%m%d_%H%M%S").to_string(),
        }
    }
}

static PLACEHOLDER_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{\{|\}\}|\{([A-Za-z]+)\}").unwrap());

pub fn expand_placeholders(template: &str, ctx: &PlaceholderContext) -> Result<String, AppError> {
    let result = PLACEHOLDER_REGEX.replace_all(template, |caps: &regex::Captures| {
        if let Some(name) = caps.get(1) {
            match name.as_str() {
                "FullName" => ctx.full_name.clone(),
                "DirectoryName" => ctx.directory_name.clone(),
                "Name" => ctx.name.clone(),
                "BaseName" => ctx.base_name.clone(),
                "Extension" => ctx.extension.clone(),
                "RelativePath" => ctx.relative_path.clone(),
                "WatchPath" => ctx.watch_path.clone(),
                "Destination" => ctx.destination.clone(),
                "Date" => ctx.date.clone(),
                "Time" => ctx.time.clone(),
                "DateTime" => ctx.datetime.clone(),
                _ => caps.get(0).unwrap().as_str().to_string(), // 不明なプレースホルダーはそのまま
            }
        } else {
            match caps.get(0).unwrap().as_str() {
                "{{" => "{".to_string(),
                "}}" => "}".to_string(),
                other => other.to_string(), // それ以外はそのまま
            }
        }
    });
    Ok(result.to_string())
}

pub fn validate_placeholders(
    text: &str,
    rule_name: &str,
    field_name: &str,
) -> Result<(), AppError> {
    // 有効なブレースホルダー
    let valid = [
        "FullName",
        "DirectoryName",
        "Name",
        "BaseName",
        "Extension",
        "RelativePath",
        "WatchPath",
        "Destination",
        "Date",
        "Time",
        "DateTime",
    ];

    for caps in PLACEHOLDER_REGEX.captures_iter(text) {
        if let Some(name) = caps.get(1) {
            let placeholder = name.as_str();
            if !valid.contains(&placeholder) {
                return Err(AppError::Validation(format!(
                    "監視ルール名 {} の {} に未知のブレースホルダーがあります {{{}}}",
                    rule_name, field_name, placeholder
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/placeholder.rs"]
mod tests;
