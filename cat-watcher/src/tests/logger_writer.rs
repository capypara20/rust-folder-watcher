//! ログファイル名をいつ決め直すか（`logger/writer.rs` の `LogFile`）のテスト。
//!
//! #100: 以前は書き出すたびに現在時刻でファイル名を作っていたため、
//! `rotation = "never"` でも日付が変わると別ファイルになり、
//! `{DateTime}` では書き出すたびに別ファイルになっていた。

use super::*;
use chrono::NaiveDate;

fn at(day: u32, hour: u32, min: u32, sec: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 9, day)
        .unwrap()
        .and_hms_opt(hour, min, sec)
        .unwrap()
}

fn file_name(log: &LogFile) -> String {
    log.path.file_name().unwrap().to_string_lossy().into_owned()
}

/// never: 日付が変わっても、時刻が進んでも、起動時のファイルに書き続けること。
#[test]
fn never_keeps_the_file_decided_at_startup() {
    for pattern in ["app_{Date}.log", "app_{DateTime}.log"] {
        let mut log = LogFile::new("logs", pattern, LogRotation::Never, at(16, 23, 59, 50));
        let first = file_name(&log);

        // 同じ日のうちに時刻が進む
        assert!(!log.roll_if_new_day(at(16, 23, 59, 58)));
        // 日付が変わる
        assert!(!log.roll_if_new_day(at(17, 0, 0, 5)), "never では切り替えない: {pattern}");

        assert_eq!(file_name(&log), first, "{pattern}");
    }
    let log = LogFile::new("logs", "app_{DateTime}.log", LogRotation::Never, at(16, 9, 30, 0));
    assert_eq!(file_name(&log), "app_20260916_093000.log");
}

/// daily: 同じ日のうちは、`{DateTime}` でも同じファイルに書き続けること。
#[test]
fn daily_keeps_one_file_within_a_day() {
    let mut log = LogFile::new("logs", "app_{DateTime}.log", LogRotation::Daily, at(16, 9, 0, 0));
    assert!(!log.roll_if_new_day(at(16, 9, 0, 1)));
    assert!(!log.roll_if_new_day(at(16, 23, 59, 59)));
    assert_eq!(file_name(&log), "app_20260916_090000.log");
}

/// daily: 日付が変わったら、その時点の日時でファイル名を決め直すこと。
#[test]
fn daily_switches_to_a_new_file_on_a_new_day() {
    let mut log = LogFile::new("logs", "app_{Date}.log", LogRotation::Daily, at(16, 23, 59, 50));
    assert_eq!(file_name(&log), "app_20260916.log");

    assert!(log.roll_if_new_day(at(17, 0, 0, 5)), "日付が変わったら切り替える");
    assert_eq!(file_name(&log), "app_20260917.log");

    // 切り替えた後は、同じ日のうちはもう切り替えない
    assert!(!log.roll_if_new_day(at(17, 12, 0, 0)));
}

/// プレースホルダーの無いファイル名は、daily でも同じファイルのまま（名前が変わりようがない）。
#[test]
fn fixed_name_stays_the_same_even_when_daily() {
    let mut log = LogFile::new("logs", "app.log", LogRotation::Daily, at(16, 12, 0, 0));
    assert!(log.roll_if_new_day(at(17, 12, 0, 0)), "日付の切り替え自体は起きる（連番のリセット用）");
    assert_eq!(file_name(&log), "app.log");
}
