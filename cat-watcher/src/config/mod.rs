//! 設定（global.toml / rules.toml）の型定義・読み込み・バリデーション。
//!
//! - [`types`]    列挙型（LogLevel / Event / ActionType など）と Deserialize。
//! - [`model`]    設定ファイルをマッピングするデータ構造。
//! - [`loader`]   ファイル読み込みと `~` 展開。
//! - [`validate`] 読み込んだ設定の意味的バリデーション。

mod loader;
mod model;
mod types;
mod validate;

pub use loader::{load_global_config, load_rules_config};
pub use model::*;
pub use types::*;
pub use validate::{validate_global_config, validate_rules_config};

// テスト（`tests` は `super::*` でこのモジュールのスコープを参照する）から
// 呼ぶ内部ヘルパとエラー型をスコープへ持ち込む。本体では使わないため cfg(test)。
#[cfg(test)]
use crate::error::AppError;
#[cfg(test)]
use validate::{collect_action_errors, finish_validation, static_root_of_destination};

#[cfg(test)]
#[path = "../tests/config.rs"]
mod tests;
