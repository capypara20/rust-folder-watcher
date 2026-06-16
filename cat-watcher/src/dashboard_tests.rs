use super::*;
use tempfile::tempdir;

#[test]
fn test_percent_decode() {
    assert_eq!(percent_decode("a%20b+c"), "a b c");
    assert_eq!(percent_decode("%E6%97%A5"), "日"); // UTF-8 percent-encoded
    assert_eq!(percent_decode("plain"), "plain");
    assert_eq!(percent_decode("100%"), "100%"); // 末尾の不完全な % はそのまま
}

#[test]
fn test_query_param() {
    let q = "q=beta%20gamma&regex=1&ci=0";
    assert_eq!(query_param(q, "q").as_deref(), Some("beta gamma"));
    assert_eq!(query_param(q, "regex").as_deref(), Some("1"));
    assert_eq!(query_param(q, "ci").as_deref(), Some("0"));
    assert_eq!(query_param(q, "missing"), None);
}

#[test]
fn test_static_affixes() {
    assert_eq!(
        static_affixes("detect_test_{Date}.log"),
        ("detect_test_".to_string(), ".log".to_string())
    );
    assert_eq!(
        static_affixes("system.log"),
        ("system.log".to_string(), String::new())
    );
}

fn write_log(dir: &std::path::Path, name: &str, body: &str) {
    std::fs::write(dir.join(name), body).unwrap();
}

#[test]
fn test_search_sources_substring_and_case() {
    let dir = tempdir().unwrap();
    write_log(
        dir.path(),
        "system_20260101.log",
        "2026-01-01 00:00:00 │ INFO │ alpha\n2026-01-01 00:00:01 │ ERROR │ beta gamma\n",
    );
    let sources = vec![LogSource {
        kind: "system",
        dir: dir.path().to_string_lossy().into_owned(),
        file_name: "system_{Date}.log".to_string(),
    }];

    let res = search_sources(&sources, "beta", false, true, 500);
    assert_eq!(res.hits.len(), 1);
    assert!(res.hits[0].line.contains("beta gamma"));
    assert_eq!(res.hits[0].kind, "system");
    assert_eq!(res.scanned_files, 1);

    // 大文字小文字を区別しない（既定）
    assert_eq!(search_sources(&sources, "ALPHA", false, true, 500).hits.len(), 1);
    // 区別する指定なら 0 件
    assert_eq!(search_sources(&sources, "ALPHA", false, false, 500).hits.len(), 0);
}

#[test]
fn test_search_sources_regex_limit_and_invalid() {
    let dir = tempdir().unwrap();
    write_log(dir.path(), "a.log", "one\ntwo\nthree\n");
    let sources = vec![LogSource {
        kind: "detect",
        dir: dir.path().to_string_lossy().into_owned(),
        file_name: "a.log".to_string(),
    }];

    // 正規表現: 行頭 t → two, three
    let res = search_sources(&sources, "^t", true, true, 500);
    assert_eq!(res.hits.len(), 2);

    // limit で打ち切り → truncated
    let res = search_sources(&sources, "e", true, true, 1);
    assert!(res.truncated);
    assert_eq!(res.hits.len(), 1);

    // 不正な正規表現 → 空結果（パニックしない）
    let res = search_sources(&sources, "(", true, true, 500);
    assert_eq!(res.hits.len(), 0);
}
