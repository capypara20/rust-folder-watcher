//! 設定（global.toml / rules.toml）の型定義・読み込み・バリデーション。
//!
//! - [`types`]    列挙型（LogLevel / Event / ActionType など）と Deserialize。
//! - [`model`]    設定ファイルをマッピングするデータ構造。
//! - [`action`]   種類ごとに必須項目を分けた、検証済みのアクション。
//! - [`loader`]   ファイル読み込みと `~` 展開。
//! - [`validate`] 読み込んだ設定の意味的バリデーション。

mod action;
mod loader;
mod model;
mod types;
mod validate;

pub use action::Action;
pub use loader::{find_config_file, load, resolve_config_path};
pub use model::*;
pub use types::*;

// テスト（`tests` は `super::*` でこのモジュールのスコープを参照する）から
// 呼ぶ内部ヘルパとエラー型をスコープへ持ち込む。本体では使わないため cfg(test)。
#[cfg(test)]
use validate::{
	collect_action_errors, finish_validation, static_root_of_destination, validate_global_config,
	validate_rules_config, Problems,
};

#[cfg(test)]
#[path = "../tests/config.rs"]
mod tests;
