//! 実行アカウント取得のテスト。

use super::*;

/// プロセストークンから取得できていること。
///
/// 環境変数フォールバックだと SID が付かないので、SID の有無で
/// どちらの経路を通ったかが判別できる。テストは通常のユーザーとして
/// 動くため、トークン経路が成功するのが正しい。
#[test]
fn current_account_comes_from_the_process_token() {
    let account = current_account();

    assert!(!account.is_empty(), "空文字が返っている");
    assert_ne!(account, "(不明)", "アカウントを取得できていない");
    assert!(
        account.contains("(S-1-"),
        "SID が付いていない。環境変数へフォールバックした可能性がある: {account}"
    );
}

/// SID の文字列表現が取れること。
///
/// SYSTEM なら S-1-5-18、LocalService なら S-1-5-19 のように、
/// 名前がローカライズされていても SID で判別できるのが狙い。
#[test]
fn account_contains_well_formed_sid() {
    let account = current_account();
    let sid = account
        .rsplit_once("(S-1-")
        .map(|(_, rest)| rest.trim_end_matches(')'))
        .expect("SID 部分が見つからない");

    // S-1- に続くのは数字とハイフンだけ
    assert!(
        sid.chars().all(|c| c.is_ascii_digit() || c == '-'),
        "SID の形式が不正: {account}"
    );
}

/// 環境変数フォールバックは、取れないときに "(不明)" を返すこと。
/// トークン経路が失敗した場合でもログに何か残るようにするための保険。
#[test]
fn env_fallback_never_returns_empty() {
    let account = account_from_env();
    assert!(!account.is_empty(), "フォールバックが空文字を返した");
}
