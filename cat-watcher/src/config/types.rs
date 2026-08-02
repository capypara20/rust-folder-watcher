//! 設定で使う列挙型と、それらの大文字小文字を区別しない Deserialize 実装。

use serde::{Deserialize, Deserializer};

/// 設定の列挙値を大文字小文字を区別せずに読む Deserialize を実装する。
///
/// 値が違ったときは「指定できる値の一覧」を添えてエラーにする。
/// `note = "..."` を付けると、その型固有の補足（廃止した値の移行案内など）を
/// メッセージの末尾に足せる。
macro_rules! impl_case_insensitive_deserialize {
    ($type:ident, note = $note:literal, $($variant:ident => $s:literal),+ $(,)?) => {
        impl_case_insensitive_deserialize!(@build $type, $note, $($variant => $s),+);
    };
    ($type:ident, $($variant:ident => $s:literal),+ $(,)?) => {
        impl_case_insensitive_deserialize!(@build $type, "", $($variant => $s),+);
    };
    (@build $type:ident, $note:expr, $($variant:ident => $s:literal),+) => {
        impl<'de> Deserialize<'de> for $type {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = String::deserialize(d)?;
                match s.to_lowercase().as_str() {
                    $($s => Ok($type::$variant),)+
                    _ => Err(serde::de::Error::custom(format!(
                        "'{}' は使用できません。指定できるのは {} です{}",
                        s,
                        [$($s),+].join(" / "),
                        $note
                    ))),
                }
            }
        }
    };
}

#[derive(Debug, Clone)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl_case_insensitive_deserialize!(LogLevel,
    Trace => "trace",
    Debug => "debug",
    Info  => "info",
    Warn  => "warn",
    Error => "error",
);

#[derive(Debug, Clone)]
pub enum LogRotation {
	Daily,
	Never,
}

impl_case_insensitive_deserialize!(LogRotation,
    Daily => "daily",
    Never => "never",
);

#[derive(Debug, Clone)]
pub enum WatchTarget {
    File,
    Directory,
    Both,
}

impl_case_insensitive_deserialize!(WatchTarget,
    File      => "file",
    Directory => "directory",
    Both      => "both",
);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Event {
    Create,
    Modify,
    Delete,
    Rename,
}

impl_case_insensitive_deserialize!(Event,
    Create => "create",
    Modify => "modify",
    Delete => "delete",
    Rename => "rename",
);

#[derive(Debug, Clone)]
pub enum ActionType {
    Copy,
    Move,
    Command,
    Execute,
}

impl_case_insensitive_deserialize!(ActionType,
    note = "。※ log は v2.0 で廃止しました（検知内容もアクションの結果も自動でログに記録されるため）。そのアクションの行ごと削除してください",
    Copy    => "copy",
    Move    => "move",
    Command => "command",
    Execute => "execute",
);
