//! アクションチェーンの打ち切り表示のテスト。

use super::*;
use crate::config::ActionType;
use crate::test_support::base_action;

/// 失敗で打ち切ったとき、実行されなかったアクションが一覧に出ること。
///
/// これが出ないと「ヘッダに actions=10 と書いてあるのにログが 1 件しか無い」状態になり、
/// 2 件目以降が動いていないことに気づけない。
#[test]
fn skipped_summary_lists_remaining_actions() {
    let actions = vec![
        base_action(ActionType::Copy),
        base_action(ActionType::Command),
        base_action(ActionType::Execute),
    ];

    // 1 件目で失敗 → 2 件目以降が中断される
    let msg = skipped_summary(&actions, 1).expect("残りがあるので出ること");
    assert!(msg.contains("2 件"), "{msg}");
    assert!(msg.contains("2.command"), "{msg}");
    assert!(msg.contains("3.execute"), "{msg}");

    // 2 件目で失敗 → 残りは 3 件目だけ
    let msg = skipped_summary(&actions, 2).expect("残りがあるので出ること");
    assert!(msg.contains("1 件"), "{msg}");
    assert!(msg.contains("3.execute"), "{msg}");
    assert!(!msg.contains("2.command"), "{msg}");

    // 最後のアクションで失敗 → 中断するものが無いので出さない
    assert!(skipped_summary(&actions, 3).is_none());
    // 範囲外でも panic しないこと
    assert!(skipped_summary(&actions, 99).is_none());
}

/// アクション種別の表記が設定の type と一致していること。
/// ログの表記と設定の書き方がズレると、利用者がログから設定を追えなくなる。
#[test]
fn action_type_label_matches_config_names() {
    assert_eq!(ActionType::Copy.as_str(), "copy");
    assert_eq!(ActionType::Move.as_str(), "move");
    assert_eq!(ActionType::Command.as_str(), "command");
    assert_eq!(ActionType::Execute.as_str(), "execute");
}

/// 開始ログの detail 行が、設定漏れを空文字で素通りさせずに組み立てられること。
///
/// 以前は `action.destination.as_deref().unwrap_or("")` で作っていたため、
/// destination を書き忘れても `destination=` と出るだけで気づけなかった。
#[test]
fn action_detail_is_built_from_validated_fields() {
    let mut raw = base_action(ActionType::Copy);
    raw.destination = Some("C:/backup".to_string());
    raw.overwrite = Some(true);
    raw.preserve_structure = Some(false);
    raw.verify_integrity = Some(false);

    let validated = Action::try_from(&raw).expect("必須項目は揃っている");
    let detail = action_detail(&validated);
    assert!(detail.contains("overwrite=true"), "{detail}");
    assert!(detail.contains("backup"), "{detail}");
}

/// 必須項目が欠けた設定は、空文字で動かさず変換の時点で弾かれること。
#[test]
fn incomplete_action_is_rejected_instead_of_running_with_empty_values() {
    // destination が無い copy。起動時のバリデーションを通っていれば
    // ここには来ないが、来てしまったときに黙って動かないことを確かめる。
    let raw = base_action(ActionType::Copy);
    let err = Action::try_from(&raw).expect_err("必須項目が無いので変換できない");
    assert_eq!(err.key, "destination");
}
