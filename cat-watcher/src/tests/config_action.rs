//! 検証済みアクション（`config/action.rs`）のテスト。
//!
//! いちばん守りたいのは「必須項目の定義が 1 か所だけである」こと。
//! 表（`requirements`）とバリデーションと `TryFrom` が食い違うと、
//! 「起動は通ったのに実行時に空文字で動く」という一番たちの悪い壊れ方をする。

use super::*;
use crate::config::ActionType;
use crate::test_support::base_action;

/// すべての種類。テストを型ごとに書き写さなくて済むように回す。
const ALL_TYPES: &[ActionType] = &[
    ActionType::Copy,
    ActionType::Move,
    ActionType::Command,
    ActionType::Execute,
];

/// その種類の必須項目をすべて埋めた、変換が通るはずの設定。
fn filled(type_: ActionType) -> ActionConfig {
    let mut a = base_action(type_);
    match type_ {
        ActionType::Copy | ActionType::Move => {
            a.destination = Some("C:/dest".to_string());
            a.overwrite = Some(true);
            a.preserve_structure = Some(false);
            a.verify_integrity = Some(false);
        }
        ActionType::Command => {
            a.shell = Some("cmd".to_string());
            a.command = Some("echo hello".to_string());
            a.working_dir = Some(String::new());
        }
        ActionType::Execute => {
            a.program = Some("cmd".to_string());
            a.args = Some(vec!["/c".to_string(), "echo".to_string()]);
            a.working_dir = Some(String::new());
        }
    }
    a
}

/// その項目を設定から消す。`requirements` のキーに対応している。
fn clear(a: &mut ActionConfig, key: &str) {
    match key {
        "destination" => a.destination = None,
        "overwrite" => a.overwrite = None,
        "preserve_structure" => a.preserve_structure = None,
        "verify_integrity" => a.verify_integrity = None,
        "shell" => a.shell = None,
        "command" => a.command = None,
        "working_dir" => a.working_dir = None,
        "program" => a.program = None,
        "args" => a.args = None,
        other => panic!("テスト側が知らないキー: {other}"),
    }
}

// =========================================================
// 表・バリデーション・変換が食い違わないこと
// =========================================================

/// 必須項目が揃っていれば、どの種類も変換できること。
#[test]
fn filled_config_converts_for_every_type() {
    for &type_ in ALL_TYPES {
        let raw = filled(type_);
        assert!(
            missing_fields(&raw).is_empty(),
            "{:?}: 埋めたのに不足が報告された: {:?}",
            type_,
            missing_fields(&raw)
        );
        assert!(
            Action::try_from(&raw).is_ok(),
            "{type_:?}: 埋めたのに変換できない"
        );
    }
}

/// 表にある項目を 1 つ消したら、**必ず**
/// `missing_fields` がそれを挙げ、`TryFrom` も失敗すること。
///
/// これが落ちるときは、表と変換のどちらかだけを直した状態になっている。
#[test]
fn clearing_any_required_field_is_caught_by_both_paths() {
    for &type_ in ALL_TYPES {
        for req in requirements(type_) {
            let mut raw = filled(type_);
            clear(&mut raw, req.key);

            let missing = missing_fields(&raw);
            assert_eq!(
                missing.iter().map(|m| m.key).collect::<Vec<_>>(),
                vec![req.key],
                "{:?}: {} を消したのに報告が合わない",
                type_,
                req.key
            );

            let err = Action::try_from(&raw)
                .expect_err(&format!("{:?}: {} が無いのに変換が通った", type_, req.key));
            assert_eq!(err.key, req.key, "{type_:?}: 変換のエラーが別の項目を指している");
        }
    }
}

/// 複数欠けていたら、全部まとめて返すこと。
/// 1 つずつ直して再実行、を繰り返させないため。
#[test]
fn missing_fields_reports_all_at_once() {
    let mut raw = filled(ActionType::Copy);
    raw.destination = None;
    raw.overwrite = None;

    let keys: Vec<_> = missing_fields(&raw).iter().map(|m| m.key).collect();
    assert_eq!(keys, vec!["destination", "overwrite"]);

    // 一方 TryFrom は最初の 1 つで止まる（値を組み立てるのが目的なので）
    assert_eq!(Action::try_from(&raw).unwrap_err().key, "destination");
}

// =========================================================
// 変換結果の中身
// =========================================================

#[test]
fn copy_keeps_every_transfer_field() {
    let mut raw = filled(ActionType::Copy);
    raw.destination = Some("D:/backup".to_string());
    raw.overwrite = Some(false);
    raw.preserve_structure = Some(true);
    raw.verify_integrity = Some(true);
    raw.auto_create = Some(false);

    let Action::Copy(t) = Action::try_from(&raw).unwrap() else {
        panic!("Copy にならない");
    };
    assert_eq!(t.destination, "D:/backup");
    assert!(!t.overwrite);
    assert!(t.preserve_structure);
    assert!(t.verify_integrity);
    assert!(!t.auto_create);
}

/// `move` は `copy` と同じ項目だが、別のバリアントになること。
#[test]
fn move_uses_the_same_fields_but_a_different_variant() {
    let raw = filled(ActionType::Move);
    assert!(matches!(Action::try_from(&raw).unwrap(), Action::Move(_)));
}

/// auto_create 未解決のまま来たら自動作成側（従来の既定）。
#[test]
fn auto_create_defaults_to_true_when_unresolved() {
    let mut raw = filled(ActionType::Copy);
    raw.auto_create = None;
    let Action::Copy(t) = Action::try_from(&raw).unwrap() else {
        panic!("Copy にならない");
    };
    assert!(t.auto_create);
}

#[test]
fn execute_keeps_args_in_order() {
    let mut raw = filled(ActionType::Execute);
    raw.args = Some(vec!["-a".to_string(), "-b".to_string()]);
    let Action::Execute(e) = Action::try_from(&raw).unwrap() else {
        panic!("Execute にならない");
    };
    assert_eq!(e.args, vec!["-a", "-b"]);
}

/// 引数なしは「空配列」で表す。未指定（None）とは区別する。
#[test]
fn execute_accepts_empty_args_but_not_missing_args() {
    let mut raw = filled(ActionType::Execute);
    raw.args = Some(vec![]);
    assert!(Action::try_from(&raw).is_ok(), "空配列は正しい指定");

    raw.args = None;
    assert_eq!(Action::try_from(&raw).unwrap_err().key, "args");
}

// =========================================================
// wait / timeout_ms
// =========================================================

/// `timeout_ms` の `0` と未指定はどちらも「無制限」。
#[test]
fn timeout_zero_and_absent_both_mean_unlimited() {
    for value in [None, Some(0)] {
        let mut raw = filled(ActionType::Command);
        raw.wait = Some(true);
        raw.timeout_ms = value;
        let Action::Command(c) = Action::try_from(&raw).unwrap() else {
            panic!("Command にならない");
        };
        assert!(c.wait.enabled);
        assert_eq!(c.wait.timeout_ms, None, "timeout_ms={value:?} は無制限のはず");
    }
}

#[test]
fn positive_timeout_is_kept() {
    let mut raw = filled(ActionType::Execute);
    raw.wait = Some(true);
    raw.timeout_ms = Some(2000);
    let Action::Execute(e) = Action::try_from(&raw).unwrap() else {
        panic!("Execute にならない");
    };
    assert_eq!(e.wait.timeout_ms, Some(2000));
}

/// wait が未解決なら「待たない」（従来の既定）。
#[test]
fn wait_defaults_to_false_when_unresolved() {
    let raw = filled(ActionType::Command);
    let Action::Command(c) = Action::try_from(&raw).unwrap() else {
        panic!("Command にならない");
    };
    assert!(!c.wait.enabled);
}

/// copy / move に wait や timeout_ms を書いても効かないので、黙って無視せず弾く。
#[test]
fn transfer_types_reject_process_only_fields() {
    for &type_ in &[ActionType::Copy, ActionType::Move] {
        let mut raw = filled(type_);
        raw.wait = Some(true);
        raw.timeout_ms = Some(500);
        let keys: Vec<_> = rejected_fields(&raw).iter().map(|r| r.key).collect();
        assert_eq!(keys, vec!["wait", "timeout_ms"], "{type_:?}");
    }
}

#[test]
fn process_types_accept_wait_and_timeout() {
    for &type_ in &[ActionType::Command, ActionType::Execute] {
        let mut raw = filled(type_);
        raw.wait = Some(true);
        raw.timeout_ms = Some(500);
        assert!(rejected_fields(&raw).is_empty(), "{type_:?}");
    }
}

// =========================================================
// エラー文
// =========================================================

/// エラー文の type は、利用者が TOML に書いた綴りで出すこと。
/// `Copy` と書かれても設定ファイルの中は `copy` なので、探すときに困る。
#[test]
fn message_uses_the_spelling_from_the_toml() {
    let mut raw = filled(ActionType::Copy);
    raw.destination = None;
    let text = missing_fields(&raw)[0].message("my-rule", ActionType::Copy);

    assert!(text.contains("my-rule"), "ルール名が入っていない: {text}");
    assert!(text.contains("type が copy のとき"), "綴りが copy でない: {text}");
    assert!(text.contains("destination(コピー先/移動先)"), "{text}");
}

/// 補足（hint）付きの項目は、文末に案内が足されること。
#[test]
fn message_appends_the_hint() {
    let mut raw = filled(ActionType::Execute);
    raw.args = None;
    let text = missing_fields(&raw)[0].message("r", ActionType::Execute);
    assert!(
        text.contains("引数がない場合は空の配列を指定してください"),
        "補足が出ていない: {text}"
    );
}

#[test]
fn rejected_message_names_the_key_and_type() {
    let mut raw = filled(ActionType::Copy);
    raw.wait = Some(true);
    let text = rejected_fields(&raw)[0].message("r", ActionType::Copy);
    assert!(text.contains("wait"), "{text}");
    assert!(text.contains("copy"), "{text}");
}
