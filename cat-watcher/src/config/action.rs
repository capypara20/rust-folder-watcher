//! 検証済みのアクション定義。
//!
//! [`ActionConfig`](super::ActionConfig) は TOML をそのまま受けるための平坦な構造体で、
//! 4 種類のアクションのフィールドが 1 つに同居している。そのため
//! アクション固有の項目はすべて `Option` にせざるを得ず、
//! **「その型では必須」という情報が型から失われていた。**
//!
//! その結果、
//!
//! - 必須チェックを `config/validate.rs` に手書きで書き写す必要があった
//! - 実行時は `action.destination.as_deref().unwrap_or("")` のように
//!   「無ければ空文字」で受けるしかなく、設定漏れが空文字として素通りしていた
//!
//! ここでは種類ごとに分けた [`Action`] を定義し、[`TryFrom`] で変換する。
//! 変換を通った時点で必須項目が揃っていることが型で保証されるので、
//! 実行時に `unwrap_or` を書く必要がなくなる。
//!
//! TOML のパース自体は従来どおり [`ActionConfig`](super::ActionConfig) が行う。
//! `type` の大文字小文字を区別しない挙動もそのまま維持される。
//!
//! ## 「何が必須か」の定義は 1 か所だけ
//!
//! [`requirements`] が返す表が唯一の定義。
//!
//! - バリデーション（[`missing_fields`]）は**欠けているものを全部**返す。
//!   設定ファイルを直す側が「1 つ直して再実行」を繰り返さずに済むため。
//! - [`TryFrom`] は最初の 1 つで打ち切る。値を組み立てるのが目的なので、
//!   1 つでも欠けていれば先へ進めない。
//!
//! 両方が同じ表を見るので、片方だけ直し忘れて食い違うことがない。

use super::{ActionConfig, ActionType};
use crate::error::AppError;

/// 種類ごとに必要な項目だけを持つ、検証済みのアクション。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// ファイル／フォルダをコピーする。
    Copy(Transfer),
    /// ファイル／フォルダを移動する。
    Move(Transfer),
    /// シェル経由でコマンドを実行する。
    Command(Command),
    /// プログラムを直接起動する。
    Execute(Execute),
}

/// `copy` / `move` に必要な項目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transfer {
    /// コピー先／移動先。プレースホルダを含みうる。
    pub destination: String,
    /// 宛先に同名ファイルがあるとき上書きするか。
    pub overwrite: bool,
    /// 監視ルートからの相対パス構造を宛先にも作るか。
    pub preserve_structure: bool,
    /// 転送後に BLAKE3 で内容を検証するか。
    pub verify_integrity: bool,
    /// 宛先フォルダが無いときに自動作成するか。
    /// 設定読み込み時に global.toml の既定値が焼き込まれている。
    pub auto_create: bool,
}

/// `command` に必要な項目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    /// 使用するシェル（`cmd` / `powershell` / `pwsh` / `bash` / `sh`）。
    pub shell: String,
    /// 実行するコマンド。プレースホルダを含みうる。
    pub command: String,
    /// 実行時のカレントディレクトリ。空文字なら変更しない。
    pub working_dir: String,
    /// 終了を待って終了コードを確認するか。
    pub wait: ProcessWait,
}

/// `execute` に必要な項目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Execute {
    /// 起動するプログラム。
    pub program: String,
    /// プログラムへ渡す引数。プレースホルダを含みうる。
    pub args: Vec<String>,
    /// 実行時のカレントディレクトリ。空文字なら変更しない。
    pub working_dir: String,
    /// 終了を待って終了コードを確認するか。
    pub wait: ProcessWait,
}

/// 外部プロセスの終了をどう扱うか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessWait {
    /// 終了を待って終了コードを確認するか。
    pub enabled: bool,
    /// 待つ上限（ミリ秒）。`None` は無制限。
    pub timeout_ms: Option<u64>,
}

/* ---- 必須項目の表 ---------------------------------------- */

/// 「この `type` ではこの項目が要る」を 1 件ぶん表したもの。
pub(crate) struct Requirement {
    /// 設定キー名。
    pub key: &'static str,
    /// 利用者向けの説明。
    pub description: &'static str,
    /// 補足。エラー文の末尾に足される。
    pub hint: Option<&'static str>,
    /// 設定に書かれているか。
    present: fn(&ActionConfig) -> bool,
}

/// `copy` / `move` の必須項目。
const TRANSFER: &[Requirement] = &[
    Requirement {
        key: "destination",
        description: "コピー先/移動先",
        hint: None,
        present: |a| a.destination.is_some(),
    },
    Requirement {
        key: "overwrite",
        description: "上書きの有無",
        hint: None,
        present: |a| a.overwrite.is_some(),
    },
    Requirement {
        key: "preserve_structure",
        description: "ディレクトリ構造を保持するか",
        hint: None,
        present: |a| a.preserve_structure.is_some(),
    },
    Requirement {
        key: "verify_integrity",
        description: "コピー後にファイルの完全性を検証するか",
        hint: None,
        present: |a| a.verify_integrity.is_some(),
    },
];

/// `command` の必須項目。
const COMMAND: &[Requirement] = &[
    Requirement {
        key: "shell",
        description: "コマンドを実行するシェル",
        hint: None,
        present: |a| a.shell.is_some(),
    },
    Requirement {
        key: "command",
        description: "実行するコマンド",
        hint: None,
        present: |a| a.command.is_some(),
    },
    Requirement {
        key: "working_dir",
        description: "コマンド/プログラムを実行するディレクトリ",
        hint: None,
        present: |a| a.working_dir.is_some(),
    },
];

/// `execute` の必須項目。
const EXECUTE: &[Requirement] = &[
    Requirement {
        key: "program",
        description: "実行するプログラム",
        hint: None,
        present: |a| a.program.is_some(),
    },
    Requirement {
        key: "args",
        description: "プログラムに渡す引数",
        hint: Some("引数がない場合は空の配列を指定してください"),
        present: |a| a.args.is_some(),
    },
    Requirement {
        key: "working_dir",
        description: "コマンド/プログラムを実行するディレクトリ",
        hint: None,
        present: |a| a.working_dir.is_some(),
    },
];

/// `type` ごとの必須項目。**必須かどうかの定義はここだけ。**
pub(crate) fn requirements(type_: ActionType) -> &'static [Requirement] {
    match type_ {
        ActionType::Copy | ActionType::Move => TRANSFER,
        ActionType::Command => COMMAND,
        ActionType::Execute => EXECUTE,
    }
}

/// 欠けている必須項目を**すべて**返す。
///
/// 1 つ目で止めないのは、設定ファイルを直す側が
/// 「あと何を足せばいいか」を一度で知りたいため。
pub fn missing_fields(raw: &ActionConfig) -> Vec<MissingField> {
    requirements(raw.type_)
        .iter()
        .filter(|r| !(r.present)(raw))
        .map(MissingField::from_requirement)
        .collect()
}

/* ---- その type では効かない項目 --------------------------- */

/// 書かれていても効かない項目を返す。
///
/// `wait` / `timeout_ms` は外部プロセスを起動する `command` / `execute` 専用。
/// 黙って無視すると「設定したのに効かない」に気づけないので、起動時に弾く。
///
/// 設定読み込み時に `copy` / `move` へは焼き込んでいないので、
/// ここに値が入っているのは利用者が自分で書いた場合だけ。
pub fn rejected_fields(raw: &ActionConfig) -> Vec<RejectedField> {
    if spawns_process(raw.type_) {
        return Vec::new();
    }
    [
        ("wait", raw.wait.is_some()),
        ("timeout_ms", raw.timeout_ms.is_some()),
    ]
    .into_iter()
    .filter(|(_, written)| *written)
    .map(|(key, _)| RejectedField { key })
    .collect()
}

/// 外部プロセスを起動する種類か。
fn spawns_process(type_: ActionType) -> bool {
    matches!(type_, ActionType::Command | ActionType::Execute)
}

/* ---- 変換 ------------------------------------------------- */

impl TryFrom<&ActionConfig> for Action {
    type Error = MissingField;

    fn try_from(raw: &ActionConfig) -> Result<Self, Self::Error> {
        // 先に表で確かめる。ここを通れば以降の取り出しは必ず成功するが、
        // 「絶対に失敗しない」を unwrap で書かずに済むよう `?` のまま通す。
        if let Some(missing) = missing_fields(raw).into_iter().next() {
            return Err(missing);
        }

        match raw.type_ {
            ActionType::Copy => Ok(Action::Copy(transfer_from(raw)?)),
            ActionType::Move => Ok(Action::Move(transfer_from(raw)?)),
            ActionType::Command => Ok(Action::Command(Command {
                shell: text(raw, raw.shell.as_deref(), "shell")?,
                command: text(raw, raw.command.as_deref(), "command")?,
                working_dir: text(raw, raw.working_dir.as_deref(), "working_dir")?,
                wait: process_wait(raw),
            })),
            ActionType::Execute => Ok(Action::Execute(Execute {
                program: text(raw, raw.program.as_deref(), "program")?,
                args: raw.args.clone().ok_or_else(|| field(raw.type_, "args"))?,
                working_dir: text(raw, raw.working_dir.as_deref(), "working_dir")?,
                wait: process_wait(raw),
            })),
        }
    }
}

fn transfer_from(raw: &ActionConfig) -> Result<Transfer, MissingField> {
    Ok(Transfer {
        destination: text(raw, raw.destination.as_deref(), "destination")?,
        overwrite: flag(raw, raw.overwrite, "overwrite")?,
        preserve_structure: flag(raw, raw.preserve_structure, "preserve_structure")?,
        verify_integrity: flag(raw, raw.verify_integrity, "verify_integrity")?,
        // auto_create は設定読み込み時に global の既定値が焼き込まれている。
        // 未解決のまま来た場合は自動作成側を既定とする（従来と同じ）。
        auto_create: raw.auto_create.unwrap_or(true),
    })
}

/// `wait` / `timeout_ms` は設定読み込み時に global の既定値が焼き込まれている。
/// `timeout_ms` の `0` と未指定はどちらも「無制限」として扱う。
fn process_wait(raw: &ActionConfig) -> ProcessWait {
    ProcessWait {
        enabled: raw.wait.unwrap_or(false),
        timeout_ms: raw.timeout_ms.filter(|ms| *ms > 0),
    }
}

fn text(
    raw: &ActionConfig,
    value: Option<&str>,
    key: &'static str,
) -> Result<String, MissingField> {
    value
        .map(str::to_string)
        .ok_or_else(|| field(raw.type_, key))
}

fn flag(raw: &ActionConfig, value: Option<bool>, key: &'static str) -> Result<bool, MissingField> {
    value.ok_or_else(|| field(raw.type_, key))
}

/// 表からその項目の説明を引く。表に無いキーを渡すのは書き間違いなので、
/// エラーにはせず説明なしで埋める（実行時に落とすほどの話ではない）。
fn field(type_: ActionType, key: &'static str) -> MissingField {
    requirements(type_)
        .iter()
        .find(|r| r.key == key)
        .map(MissingField::from_requirement)
        .unwrap_or(MissingField {
            key,
            description: "",
            hint: None,
        })
}

/* ---- エラー型 --------------------------------------------- */

/// 必須項目が欠けていることを表す。
///
/// エラー文の組み立ては呼び出し側（バリデーション）に任せる。
/// ルール名などの文脈をここでは持たないため。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingField {
    /// 設定キー名。
    pub key: &'static str,
    /// 利用者向けの説明。
    pub description: &'static str,
    /// 補足。エラー文の末尾に足される。
    pub hint: Option<&'static str>,
}

impl MissingField {
    fn from_requirement(r: &'static Requirement) -> Self {
        MissingField {
            key: r.key,
            description: r.description,
            hint: r.hint,
        }
    }

    /// バリデーションエラーの文面にする。
    ///
    /// `type` は利用者が TOML に書いた綴り（`copy` など）で出す。
    /// `Debug` の `Copy` だと設定ファイルを探すときに引っかからない。
    pub fn message(&self, rule_name: &str, action_type: ActionType) -> String {
        let head = format!(
            "監視ルール名 {} のアクションの type が {} のとき、{}({}) を定義してください",
            rule_name,
            action_type.as_str(),
            self.key,
            self.description
        );
        match self.hint {
            Some(hint) => format!("{head}。{hint}"),
            None => head,
        }
    }
}

impl From<MissingField> for AppError {
    fn from(e: MissingField) -> Self {
        AppError::Action(format!("{}({}) が未指定", e.key, e.description))
    }
}

/// その `type` では効かない項目が書かれていることを表す。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectedField {
    /// 設定キー名。
    pub key: &'static str,
}

impl RejectedField {
    /// バリデーションエラーの文面にする。
    pub fn message(&self, rule_name: &str, action_type: ActionType) -> String {
        format!(
            "監視ルール名 {} のアクションの {} は type が command / execute のときだけ指定できます（{} では外部プロセスを起動しません）",
            rule_name,
            self.key,
            action_type.as_str()
        )
    }
}

#[cfg(test)]
#[path = "../tests/config_action.rs"]
mod tests;
