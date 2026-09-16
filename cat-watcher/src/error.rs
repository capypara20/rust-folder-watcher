//! アプリ全体のエラー型と終了コード。
//!
//! ## 種類の分け方
//!
//! **「どの段階で失敗したか」で分ける。** 以前は「設定」「監視」のような段階と、
//! 「I/O」「TOML」のような原因の種類が 1 つの列挙に混ざっていた。
//! そのため、設定ファイルが読めないだけの失敗が `Io` になり、
//! 終了コードで「ログの初期化に失敗」と誤って報告されていた。
//!
//! ```text
//! 起動前（使い方・設定）  Usage / ConfigRead / ConfigParse / ConfigInvalid / TemplateWrite
//! 実行中                  Runtime / Watch / Action / FileHash
//! ```
//!
//! 元になった原因（`std::io::Error` など）は捨てずに持つ。
//!
//! ## 文言の約束
//!
//! - `Display` はそれだけで意味が通る 1 文にする。表示する側は `[ERROR]` 以外の
//!   前置き（「実行エラー:」など）を足さない。以前は前置きが重なっていた
//! - どのファイルの話かを必ず書く
//! - 特定の道具や開発者の環境を前提にした言い回しをしない

use std::fmt;
use std::path::{Path, PathBuf};

use crate::path_fmt;

/// 終了コード。CLI とサービス（`sc query` の `SERVICE_EXIT_CODE`）で同じ値を使う。
pub mod exit_code {
    /// 起動前の準備（引数・設定ファイルの読み込み・書式・内容）に失敗した。
    pub const CONFIG: i32 = 10;
    /// 起動後の実行中に失敗した（実行基盤・監視・アクション）。
    pub const RUNTIME: i32 = 12;
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// 起動のしかたに問題がある（引数の不足、設定ファイルが見つからない等）。
    #[error("{0}")]
    Usage(String),

    /// 設定ファイルを読めない。
    #[error("設定ファイルを読み込めません: {}: {source}", path_fmt::for_log(.path))]
    ConfigRead {
        path: PathBuf,
        source: std::io::Error,
    },

    /// 設定ファイルが TOML として正しくない、または値の型が違う。
    #[error("設定ファイルの書き方に誤りがあります: {}\n{message}", path_fmt::for_log(.path))]
    ConfigParse { path: PathBuf, message: String },

    /// 設定の内容に問題がある。
    ///
    /// ファイルごとに、見つかった問題をまとめて持つ。1 件ずつ直して
    /// 再実行を繰り返さずに済むよう、global と rules の両方を一度に報告する。
    #[error("{}", render_invalid(.0))]
    ConfigInvalid(Vec<InvalidFile>),

    /// テンプレートを書き出せない（`--init`）。
    #[error("テンプレートを書き出せません: {}: {source}", path_fmt::for_log(.path))]
    TemplateWrite {
        path: PathBuf,
        source: std::io::Error,
    },

    /// 実行基盤（非同期ランタイム、Windows サービス制御）の失敗。
    #[error("{0}")]
    Runtime(String),

    /// ファイル監視の開始・継続に失敗した。
    #[error("{0}")]
    Watch(String),

    /// アクションの実行に失敗した。
    #[error("アクション実行エラー: {0}")]
    Action(String),

    /// コピー後の内容検証に失敗した。
    #[error("ファイルハッシュ値比較エラー: {0}")]
    FileHash(String),
}

impl AppError {
    /// この失敗で終了するときの終了コード。
    ///
    /// ワイルドカードを使わずに全種類を並べている。種類を足したときに、
    /// どちらの段階かを決め忘れるとコンパイルが通らないようにするため。
    pub fn exit_code(&self) -> i32 {
        match self {
            AppError::Usage(_)
            | AppError::ConfigRead { .. }
            | AppError::ConfigParse { .. }
            | AppError::ConfigInvalid(_)
            | AppError::TemplateWrite { .. } => exit_code::CONFIG,
            AppError::Runtime(_)
            | AppError::Watch(_)
            | AppError::Action(_)
            | AppError::FileHash(_) => exit_code::RUNTIME,
        }
    }
}

/// 1 つの設定ファイルで見つかった問題。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidFile {
    pub path: PathBuf,
    pub problems: Vec<String>,
}

impl InvalidFile {
    pub fn new(path: &Path, problems: Vec<String>) -> Self {
        Self {
            path: path.to_path_buf(),
            problems,
        }
    }
}

impl fmt::Display for InvalidFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "設定に {} 件の問題があります: {}",
            self.problems.len(),
            path_fmt::for_log(&self.path)
        )?;
        for (i, problem) in self.problems.iter().enumerate() {
            write!(f, "\n  [{}] {}", i + 1, problem)?;
        }
        Ok(())
    }
}

fn render_invalid(files: &[InvalidFile]) -> String {
    files
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
#[path = "tests/error.rs"]
mod tests;
