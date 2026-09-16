//! 設定の問題 1 件を表す型と、その表示。
//!
//! ## なぜ構造にするか
//!
//! 以前は検証のたびに `format!("監視ルール名 {} のアクションの…", …)` と
//! 文を丸ごと手書きしていた。前置きが 12 か所に写され、「{} 番目の」と
//! 「{}番目の」のように書き方も揃っていなかった。
//!
//! ここでは 1 件を「**どこの**設定か」「**何が**問題か」「**どうすればよいか**」に
//! 分けて持ち、表示の組み立ては [`Problem`] の `Display` 1 か所で行う。
//! 検証する側は場所と内容だけを書けばよい。
//!
//! ## 文言の約束
//!
//! - 内容は「対象の値 + どうなっているか」の 1 文。値は `'...'` で囲む
//! - 対処は、どの環境でも通用する一般的な手順だけを書く。特定の道具名や、
//!   開発者自身の環境を前提にした言い回しはしない
//! - OS によって事実が違う場合（Windows のサービスの PATH、Unix の実行権限など）だけ
//!   `cfg` で書き分ける
//!
//! ```text
//!   [1] rules "backup" > actions[2] > program
//!       'tool' が PATH 上に見つかりません
//!       対処: フルパスで指定するか、PATH に含まれるフォルダに置いてください
//! ```

use std::fmt;

/// 設定の中の場所。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    /// ファイル全体に関わる問題（ルールが 1 つも無い等）。
    File,
    /// global.toml の項目。`system_log.dir` のようにキーをそのまま持つ。
    Global(String),
    /// あるルールの項目。`watch.path` のように `[[rules]]` からの相対キーを持つ。
    Rule { rule: RuleRef, key: String },
    /// あるルールの、あるアクションの項目。
    Action {
        rule: RuleRef,
        /// 1 始まりの番号。設定ファイルに書いた順。
        index: usize,
        key: String,
    },
}

/// ルールの指し方。名前があれば名前で、空なら何番目かで指す。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleRef {
    Named(String),
    /// 1 始まりの番号。
    Nth(usize),
}

impl RuleRef {
    /// ルールの名前と、1 始まりの番号から作る。名前が空白だけなら番号で指す。
    pub fn new(name: &str, index: usize) -> Self {
        if name.trim().is_empty() {
            RuleRef::Nth(index)
        } else {
            RuleRef::Named(name.to_string())
        }
    }
}

impl fmt::Display for RuleRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RuleRef::Named(name) => write!(f, "rules \"{name}\""),
            RuleRef::Nth(n) => write!(f, "rules[{n}]"),
        }
    }
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Location::File => write!(f, "（ファイル全体）"),
            Location::Global(key) => write!(f, "{key}"),
            Location::Rule { rule, key } => write!(f, "{rule} > {key}"),
            Location::Action { rule, index, key } => write!(f, "{rule} > actions[{index}] > {key}"),
        }
    }
}

/// 設定の問題 1 件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub location: Location,
    /// 何が問題か。複数行でもよい（続きの行は表示時に字下げされる）。
    pub message: String,
    /// どうすればよいか。複数行でもよい。
    pub hint: Option<String>,
}

impl Problem {
    pub fn new(location: Location, message: impl Into<String>) -> Self {
        Self {
            location,
            message: message.into(),
            hint: None,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// 1 件を、見出し（場所）→ 内容 → 対処の順に、字下げを揃えて書く。
    ///
    /// 番号（`[1]` など）とその前の字下げは一覧を出す側が付ける。
    /// ここでは 2 行目以降の字下げを `indent` で受ける。
    pub fn render(&self, indent: &str) -> String {
        let mut out = self.location.to_string();
        for line in self.message.lines() {
            out.push('\n');
            out.push_str(indent);
            out.push_str(line);
        }
        if let Some(hint) = &self.hint {
            // 続きの行は「対処: 」の後ろに揃える。
            // 端末では全角 2 文字 + 「: 」で 6 桁ぶんの幅になる。
            let cont = format!("{indent}      ");
            for (i, line) in hint.lines().enumerate() {
                out.push('\n');
                if i == 0 {
                    out.push_str(indent);
                    out.push_str("対処: ");
                } else {
                    out.push_str(&cont);
                }
                out.push_str(line);
            }
        }
        out
    }
}

#[cfg(test)]
#[path = "../tests/config_problem.rs"]
mod tests;
