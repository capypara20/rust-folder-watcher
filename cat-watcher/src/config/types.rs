//! 設定で使う列挙型と、それらの大文字小文字を区別しない Deserialize 実装。

use serde::{Deserialize, Deserializer};

macro_rules! impl_case_insensitive_deserialize {
    ($type:ident, $($variant:ident => $s:literal),+ $(,)?) => {
        impl<'de> Deserialize<'de> for $type {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = String::deserialize(d)?;
                match s.to_lowercase().as_str() {
                    $($s => Ok($type::$variant),)+
                    _ => Err(serde::de::Error::custom(format!("unknown value: {}", s))),
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
    Log,
}

impl_case_insensitive_deserialize!(ActionType,
    Copy    => "copy",
    Move    => "move",
    Command => "command",
    Execute => "execute",
    Log     => "log",
);
