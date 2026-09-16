//! 設定の問題（`config/problem.rs`）の表示のテスト。

use super::*;

fn rule(name: &str) -> RuleRef {
    RuleRef::Named(name.to_string())
}

// =========================================================
// 場所の表記
// =========================================================

#[test]
fn location_of_global_key_is_the_key_itself() {
    assert_eq!(Location::Global("system_log.dir".into()).to_string(), "system_log.dir");
}

#[test]
fn location_of_rule_uses_its_name() {
    let at = Location::Rule { rule: rule("backup"), key: "watch.path".into() };
    assert_eq!(at.to_string(), r#"rules "backup" > watch.path"#);
}

#[test]
fn location_of_action_uses_one_based_number() {
    let at = Location::Action { rule: rule("backup"), index: 2, key: "program".into() };
    assert_eq!(at.to_string(), r#"rules "backup" > actions[2] > program"#);
}

/// 名前が空のルールは、名前の代わりに何番目かで指すこと。
/// 空の名前をそのまま出すと `rules "" > …` になり、どのルールか分からない。
#[test]
fn unnamed_rule_is_referred_to_by_position() {
    assert_eq!(RuleRef::new("", 3), RuleRef::Nth(3));
    assert_eq!(RuleRef::new("   ", 3), RuleRef::Nth(3));
    assert_eq!(RuleRef::new("backup", 3), rule("backup"));
    assert_eq!(RuleRef::Nth(3).to_string(), "rules[3]");
}

// =========================================================
// 1 件の表示
// =========================================================

/// 見出し（場所）→ 内容 → 対処 の順に、字下げを揃えて並ぶこと。
#[test]
fn render_puts_location_message_and_hint_on_separate_lines() {
    let problem = Problem::new(Location::Global("detect.poll_interval_ms".into()), "0 は指定できません")
        .with_hint("1 以上の値を指定してください");
    let lines: Vec<String> = problem.render("    ").lines().map(str::to_string).collect();

    assert_eq!(
        lines,
        vec![
            "detect.poll_interval_ms".to_string(),
            "    0 は指定できません".to_string(),
            "    対処: 1 以上の値を指定してください".to_string(),
        ]
    );
}

/// 内容や対処が複数行でも、続きの行が字下げされること。
/// 字下げが崩れると、どの行がどの問題の話か読み取れなくなる。
#[test]
fn render_indents_continuation_lines() {
    let problem = Problem::new(Location::File, "1 行目\n2 行目").with_hint("対処 1\n対処 2");
    let lines: Vec<String> = problem.render("  ").lines().map(str::to_string).collect();

    assert_eq!(lines[1], "  1 行目");
    assert_eq!(lines[2], "  2 行目");
    assert_eq!(lines[3], "  対処: 対処 1");
    // 「対処: 」の後ろに揃う
    assert_eq!(lines[4], "        対処 2");
}

#[test]
fn render_without_hint_has_no_hint_line() {
    let text = Problem::new(Location::File, "ルールが 1 つもありません").render("  ");
    assert!(!text.contains("対処"), "{text}");
}

// =========================================================
// 文言の約束
// =========================================================

/// 利用者に見せる文言に、特定の道具や開発者の環境を前提にした言葉が入っていないこと。
///
/// 以前は PATH の対処に「（scoop 等）」と、開発者の PC で見た道具の名前を書いていた。
/// 利用者の環境はさまざまなので、対処は一般的な手順だけにする。
#[test]
fn messages_do_not_assume_a_specific_environment() {
    let sources = [
        ("config/validate.rs", include_str!("../config/validate.rs")),
        ("config/action.rs", include_str!("../config/action.rs")),
        ("config/problem.rs", include_str!("../config/problem.rs")),
        ("config/loader.rs", include_str!("../config/loader.rs")),
        ("error.rs", include_str!("../error.rs")),
        ("exe_path.rs", include_str!("../exe_path.rs")),
        ("actions/mod.rs", include_str!("../actions/mod.rs")),
        ("actions/common.rs", include_str!("../actions/common.rs")),
        ("actions/copy.rs", include_str!("../actions/copy.rs")),
        ("actions/move.rs", include_str!("../actions/move.rs")),
        ("actions/command.rs", include_str!("../actions/command.rs")),
        ("actions/execute.rs", include_str!("../actions/execute.rs")),
    ];
    let banned = ["scoop", "chocolatey", "winget", "homebrew", "roze", "capypara"];
    for (file, body) in sources {
        let lower = body.to_lowercase();
        for word in banned {
            assert!(!lower.contains(word), "{file} に環境依存の言葉 '{word}' が入っている");
        }
    }
}
