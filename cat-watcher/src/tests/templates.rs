//! `--init` で出力するテンプレートが、そのまま読み込める形になっているかを守るテスト。
//! テンプレートはハードコードした文字列なので、設定項目を足したときに
//! 追従し忘れるとユーザーの手元でいきなりパースエラーになる。

use super::*;
use crate::config::{GlobalConfig, RulesConfig};

#[test]
fn global_template_parses() {
    toml::from_str::<GlobalConfig>(GLOBAL_TOML)
        .unwrap_or_else(|e| panic!("global.toml テンプレートがパースできない: {e}"));
}

#[test]
fn rules_template_parses() {
    toml::from_str::<RulesConfig>(RULES_TOML)
        .unwrap_or_else(|e| panic!("rules.toml テンプレートがパースできない: {e}"));
}

#[test]
fn csv_template_converts_and_parses() {
    let toml_text = crate::csv_import::convert(RULES_CSV)
        .unwrap_or_else(|e| panic!("CSV テンプレートの変換に失敗: {e}"));
    toml::from_str::<RulesConfig>(&toml_text)
        .unwrap_or_else(|e| panic!("CSV から生成した TOML がパースできない: {e}\n---\n{toml_text}"));
}

/// CSV テンプレートのヘッダーが、実装の列順（`csv_import::COLUMNS`）と一致すること。
/// ここがズレると、利用者は正しく書いたつもりの CSV から別物のルールを作ってしまう。
#[test]
fn csv_template_header_matches_column_order() {
    let header = RULES_CSV
        .lines()
        .next()
        .expect("CSV テンプレートが空")
        .trim_end_matches('\r');
    let actual: Vec<&str> = header.split(',').collect();

    assert_eq!(
        actual,
        crate::csv_import::COLUMNS,
        "CSV テンプレートのヘッダーが実装の列順と一致していない"
    );
}

/// テンプレートの行が、ヘッダーと同じ列数になっていること（末尾カンマの数え間違い防止）。
#[test]
fn csv_template_sample_row_has_same_column_count() {
    let mut lines = RULES_CSV.lines();
    let header = lines.next().unwrap().trim_end_matches('\r');
    let sample = lines.next().expect("サンプル行がない").trim_end_matches('\r');

    assert_eq!(
        sample.split(',').count(),
        header.split(',').count(),
        "サンプル行の列数がヘッダーと合っていない"
    );
}
