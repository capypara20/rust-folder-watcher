use super::*;
use crate::config::Event;

fn events(list: &[Event]) -> HashSet<Event> {
    list.iter().cloned().collect()
}

#[test]
fn test_pad_left_display() {
    let s = pad_left_display("INFO", SYS_LEVEL_WIDTH);
    assert_eq!(s, "INFO ");
    assert_eq!(UnicodeWidthStr::width_cjk(s.as_str()), SYS_LEVEL_WIDTH);
    // 幅を超える文字列は切り詰めない
    assert_eq!(pad_left_display("VERYLONGTEXT", 5), "VERYLONGTEXT");
}

/// ステップカラムはペア番号付き（`1. copy` ↔ `1. OK`）で揃う。
/// index 0 は番号なしで描画する（チェーン全体エラー等）。
#[test]
fn test_render_step_col() {
    let start = render_step_col(1, "copy");
    let ok = render_step_col(1, "OK");
    assert!(start.starts_with("1. copy"));
    assert!(ok.starts_with("1. OK"));
    assert_eq!(UnicodeWidthStr::width_cjk(start.as_str()), ACTION_STEP_WIDTH);
    assert_eq!(UnicodeWidthStr::width_cjk(ok.as_str()), ACTION_STEP_WIDTH);

    let s = render_step_col(0, "ERR");
    assert!(s.starts_with("ERR"));
    assert!(!s.contains('.'));
}

/// ブロック開始セパレータに連番・パス・イベント・アクション数が入る。
#[test]
fn test_render_block_start_contains_fields() {
    let line = render_block_start(
        1,
        "2026-06-07 10:05:30",
        "C:/watch/csv/data.csv",
        &events(&[Event::Create]),
        2,
    );
    assert!(line.starts_with("═══ #1"));
    assert!(line.contains("C:/watch/csv/data.csv"));
    assert!(line.contains("(Create)"));
    assert!(line.contains("actions=2"));
}

/// 検知ログは 3 カラム（ts │ events │ path）で出力する。
#[test]
fn test_render_detect_line_three_columns() {
    let line = render_detect_line(
        "2026-06-07 10:05:30",
        &events(&[Event::Create, Event::Modify]),
        "C:/watch/a.csv",
    );
    assert_eq!(line.matches('│').count(), 2);
    assert!(line.contains("Create,Modify"));
    assert!(line.contains("C:/watch/a.csv"));
}

/// システムログには Info/Warn/Error のみラベルが付き、検知/アクション系は None。
#[test]
fn test_sys_level_label_skips_action_entries() {
    assert_eq!(sys_level_label(&LogEntry::Info("x".into())), Some("INFO"));
    assert_eq!(sys_level_label(&LogEntry::Warn("x".into())), Some("WARN"));
    assert_eq!(sys_level_label(&LogEntry::Error("x".into())), Some("ERROR"));
    assert_eq!(
        sys_level_label(&LogEntry::Match {
            rule_name: "r".into(),
            path: "p".into(),
            events: events(&[Event::Create]),
        }),
        None
    );
    assert_eq!(
        sys_level_label(&LogEntry::ActionOk { index: 1, total: 1, msg: "ok".into() }),
        None
    );
    assert_eq!(
        sys_level_label(&LogEntry::ActionErr { index: 1, total: 1, msg: "e".into() }),
        None
    );
}

/// action_step_parts が各 Action 系エントリを正しく分解する。
#[test]
fn test_action_step_parts_labels() {
    let ok = LogEntry::ActionOk { index: 2, total: 3, msg: "done".into() };
    assert_eq!(action_step_parts(&ok), Some((2, "OK".to_string(), "done".to_string())));
    let err = LogEntry::ActionErr { index: 1, total: 2, msg: "boom".into() };
    assert_eq!(action_step_parts(&err), Some((1, "ERR".to_string(), "boom".to_string())));
    let cmd = LogEntry::Action {
        index: 1,
        total: 1,
        action_type: "command".into(),
        detail: "shell=cmd".into(),
    };
    assert_eq!(action_step_parts(&cmd), Some((1, "cmd".to_string(), "shell=cmd".to_string())));
    // Match は Action 系ではない
    let m = LogEntry::Match {
        rule_name: "r".into(),
        path: "p".into(),
        events: events(&[Event::Create]),
    };
    assert_eq!(action_step_parts(&m), None);
}
