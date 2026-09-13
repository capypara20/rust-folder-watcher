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
