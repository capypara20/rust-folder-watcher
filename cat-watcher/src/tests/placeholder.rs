use super::*;
use std::path::PathBuf;

/// テスト用の固定値コンテキストを生成する
fn make_ctx() -> PlaceholderContext {
    PlaceholderContext {
        full_name: "C:/data/incoming/report.csv".to_string(),
        directory_name: "C:/data/incoming".to_string(),
        name: "report.csv".to_string(),
        base_name: "report".to_string(),
        extension: "csv".to_string(),
        relative_path: "report.csv".to_string(),
        watch_path: "C:/data/incoming".to_string(),
        destination: "C:/data/outgoing".to_string(),
        date: "20260412".to_string(),
        time: "153000".to_string(),
        datetime: "20260412_153000".to_string(),
    }
}

#[test]
fn test_expand_each_placeholder() {
    let ctx = make_ctx();
    let cases = [
        ("{FullName}", "C:/data/incoming/report.csv"),
        ("{DirectoryName}", "C:/data/incoming"),
        ("{Name}", "report.csv"),
        ("{BaseName}", "report"),
        ("{Extension}", "csv"),
        ("{RelativePath}", "report.csv"),
        ("{WatchPath}", "C:/data/incoming"),
        ("{Destination}", "C:/data/outgoing"),
        ("{Date}", "20260412"),
        ("{Time}", "153000"),
        ("{DateTime}", "20260412_153000"),
    ];
    for (template, expected) in cases {
        assert_eq!(expand_placeholders(template, &ctx), expected, "template: {template}");
    }
}

#[test]
fn test_expand_composite_and_escape() {
    let ctx = make_ctx();
    // 複合テンプレート
    assert_eq!(
        expand_placeholders("{DirectoryName}/{BaseName}_{DateTime}.{Extension}", &ctx),
        "C:/data/incoming/report_20260412_153000.csv"
    );
    // {{ }} エスケープ
    assert_eq!(expand_placeholders("{{literal}}", &ctx), "{literal}");
    assert_eq!(expand_placeholders("{{prefix}}_{Name}", &ctx), "{prefix}_report.csv");
    // プレースホルダなし
    assert_eq!(
        expand_placeholders("plain text without placeholders", &ctx),
        "plain text without placeholders"
    );
}

#[test]
fn test_expand_extension_empty_for_no_extension_file() {
    let mut ctx = make_ctx();
    ctx.extension = "".to_string();
    assert_eq!(expand_placeholders("{BaseName}.{Extension}", &ctx), "report.");
}

#[test]
fn test_validate_known_inputs_ok() {
    let all = "{FullName}{DirectoryName}{Name}{BaseName}{Extension}{RelativePath}{WatchPath}{Destination}{Date}{Time}{DateTime}";
    assert_eq!(find_unknown_placeholder(all), None);
    assert_eq!(find_unknown_placeholder("{{escaped}}"), None);
    assert_eq!(find_unknown_placeholder("just plain text"), None);
}

#[test]
fn test_unknown_placeholder_is_reported_by_name() {
    assert_eq!(find_unknown_placeholder("a {Name} b {Bad} c {Worse}"), Some("Bad".to_string()));
}

#[test]
fn test_new_context_basic_and_subdirectory() {
    let watch_path = PathBuf::from("C:/data/incoming");
    let ctx =
        PlaceholderContext::new(&PathBuf::from("C:/data/incoming/report.csv"), &watch_path, "C:/dest");
    assert_eq!(ctx.full_name, "C:/data/incoming/report.csv");
    assert_eq!(ctx.directory_name, "C:/data/incoming");
    assert_eq!(ctx.name, "report.csv");
    assert_eq!(ctx.base_name, "report");
    assert_eq!(ctx.extension, "csv");
    assert_eq!(ctx.relative_path, "report.csv");
    assert_eq!(ctx.watch_path, "C:/data/incoming");
    assert_eq!(ctx.destination, "C:/dest");

    // サブディレクトリは relative_path に反映される
    let ctx = PlaceholderContext::new(
        &PathBuf::from("C:/data/incoming/sub/deep/file.txt"),
        &watch_path,
        "C:/dest",
    );
    assert_eq!(ctx.relative_path, "sub/deep/file.txt");
    assert_eq!(ctx.name, "file.txt");
}

#[test]
fn test_new_context_no_extension() {
    let ctx = PlaceholderContext::new(
        &PathBuf::from("C:/data/incoming/Makefile"),
        &PathBuf::from("C:/data/incoming"),
        "C:/dest",
    );
    assert_eq!(ctx.name, "Makefile");
    assert_eq!(ctx.base_name, "Makefile");
    assert_eq!(ctx.extension, "");
}

#[test]
fn test_new_context_date_format() {
    let ctx =
        PlaceholderContext::new(&PathBuf::from("C:/data/file.txt"), &PathBuf::from("C:/data"), "");
    assert_eq!(ctx.date.len(), 8); // YYYYMMDD
    assert_eq!(ctx.time.len(), 6); // HHmmss
    assert_eq!(ctx.datetime.len(), 15); // YYYYMMDD_HHmmss
    assert!(ctx.datetime.contains('_'));
}

#[test]
fn find_any_placeholder_ignores_escapes() {
    assert_eq!(find_any_placeholder("{{literal}}"), None);
    assert_eq!(find_any_placeholder("C:/tools/app.exe"), None);
    assert_eq!(find_any_placeholder("C:/{WatchPath}/x"), Some("WatchPath".to_string()));
}
