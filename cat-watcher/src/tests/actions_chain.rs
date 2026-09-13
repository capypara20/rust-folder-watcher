//! アクションチェーンの打ち切り表示のテスト。

use super::*;
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
    assert_eq!(action_type_label(&ActionType::Copy), "copy");
    assert_eq!(action_type_label(&ActionType::Move), "move");
    assert_eq!(action_type_label(&ActionType::Command), "command");
    assert_eq!(action_type_label(&ActionType::Execute), "execute");
}
