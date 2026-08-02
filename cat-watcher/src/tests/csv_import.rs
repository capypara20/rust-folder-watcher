use super::*;
use crate::config::RulesConfig;

/// 変換した TOML を実際にパースして、rules.toml として成立しているか確かめる。
fn convert_and_parse(csv: &str) -> RulesConfig {
    let toml_text = convert(csv).expect("変換に失敗");
    toml::from_str::<RulesConfig>(&toml_text)
        .unwrap_or_else(|e| panic!("生成された TOML がパースできない: {e}\n---\n{toml_text}"))
}

/// ヘッダー行は実装の列順（COLUMNS）から組み立てる。
/// 手書きするとズレたときに気づけないため。
fn header() -> String {
    COLUMNS.join(",")
}

// Windows パスをそのまま埋め込むと \t などがエスケープと解釈されて
// 壊れた TOML になっていたバグの回帰テスト。
#[test]
fn windows_paths_are_escaped() {
    let csv = format!(
        "{}\n\
         win,true,C:\\watch,true,file,false,*.csv,,,create,execute,,,,,,,C:\\tool\\app.exe,C:\\temp\\out|--flag,C:\\work\n",
        header()
    );
    let parsed = convert_and_parse(&csv);
    let action = &parsed.rules[0].actions[0];

    assert_eq!(action.program.as_deref(), Some(r"C:\tool\app.exe"));
    assert_eq!(
        action.args.as_deref(),
        Some(&[r"C:\temp\out".to_string(), "--flag".to_string()][..])
    );
    assert_eq!(parsed.rules[0].watch.path, r"C:\watch");
}

// Excel は論理値を大文字の TRUE / FALSE で書き出す。そのまま埋めると
// TOML が壊れるため、正規化されること。
#[test]
fn excel_style_booleans_are_normalized() {
    let csv = format!(
        "{}\n\
         excel,TRUE,C:\\watch,TRUE,file,FALSE,*.csv,,,create,copy,D:\\out,TRUE,FALSE,TRUE\n",
        header()
    );
    let parsed = convert_and_parse(&csv);

    assert!(parsed.rules[0].enabled);
    assert!(parsed.rules[0].watch.recursive);
    assert!(!parsed.rules[0].watch.include_hidden);
    assert_eq!(parsed.rules[0].actions[0].overwrite, Some(true));
    assert_eq!(parsed.rules[0].actions[0].preserve_structure, Some(false));
    assert_eq!(parsed.rules[0].actions[0].verify_integrity, Some(true));
}

// 解釈できない bool は分かるメッセージでエラーにする。
#[test]
fn invalid_boolean_is_rejected_with_message() {
    let csv = format!(
        "{}\n\
         bad,はい,C:\\watch,true,file,false,*.csv,,,create,copy,D:\\out,false,false,false\n",
        header()
    );
    let err = convert(&csv).unwrap_err().to_string();
    assert!(err.contains("enabled"), "どの列か分かるメッセージであること: {err}");
}

// 末尾に追加した auto_create / delay_ms 列が反映されること。
#[test]
fn auto_create_and_delay_columns_are_applied() {
    let csv = format!(
        "{}\n\
         cols,true,C:\\watch,true,file,false,*.csv,,,create,copy,D:\\out,false,false,false,,,,,,,,,,,false,1500\n",
        header()
    );
    let parsed = convert_and_parse(&csv);
    let action = &parsed.rules[0].actions[0];

    assert_eq!(action.auto_create, Some(false));
    assert_eq!(action.delay_ms, Some(1500));
}

// 列を後ろに足したので、古い（短い）CSV もそのまま読めること。
#[test]
fn legacy_short_rows_still_work() {
    // working_dir までの 20 列だけ（exclude_regex 以降が無い旧フォーマット）
    let csv = "rule_name,enabled,watch_path,recursive,target,include_hidden,patterns,regex,exclude_patterns,events,action_type,destination,overwrite,preserve_structure,verify_integrity,shell,command,program,args,working_dir\n\
               legacy,true,C:\\watch,true,file,false,*.csv,,,create,copy,D:\\out,false,false,false,,,,,\n";
    let parsed = convert_and_parse(csv);

    assert_eq!(parsed.rules[0].name, "legacy");
    assert_eq!(parsed.rules[0].actions[0].destination.as_deref(), Some(r"D:\out"));
    assert_eq!(parsed.rules[0].actions[0].auto_create, None, "未指定なら global に従う");
}

// 同じ rule_name の行を並べると 1 ルールの複数アクションになる。
#[test]
fn same_rule_name_becomes_multiple_actions() {
    let csv = format!(
        "{h}\n\
         multi,true,C:\\watch,true,file,false,*.csv,,,create,copy,D:\\out,false,false,false\n\
         multi,true,C:\\watch,true,file,false,*.csv,,,create,move,E:\\arc,false,false,false\n",
        h = header()
    );
    let parsed = convert_and_parse(&csv);

    assert_eq!(parsed.rules.len(), 1);
    assert_eq!(parsed.rules[0].actions.len(), 2);
}

// ルール名やコマンドに引用符が入っていても壊れない。
#[test]
fn quotes_inside_values_are_escaped() {
    let shell = if cfg!(windows) { "cmd" } else { "bash" };
    let csv = format!(
        "{h}\n\
         \"say \"\"hi\"\"\",true,C:\\watch,true,file,false,*.csv,,,create,command,,,,,{shell},\"echo \"\"a b\"\"\",,,\n",
        h = header()
    );
    let parsed = convert_and_parse(&csv);

    assert_eq!(parsed.rules[0].name, r#"say "hi""#);
    assert_eq!(parsed.rules[0].actions[0].command.as_deref(), Some(r#"echo "a b""#));
}

// v2.0 で廃止した log アクションは、理由が分かるエラーにする。
#[test]
fn log_action_is_rejected_with_migration_hint() {
    let csv = format!(
        "{}\n\
         old,true,C:\\watch,true,file,false,*.csv,,,create,log,,,,,\n",
        header()
    );
    let err = convert(&csv).unwrap_err().to_string();
    assert!(err.contains("廃止"), "廃止された旨が分かること: {err}");
}

// 旧フォーマット（message 列つき）は、列がずれたまま読まずにエラーにする。
#[test]
fn old_header_with_message_column_is_rejected() {
    let csv = "rule_name,enabled,watch_path,recursive,target,include_hidden,patterns,regex,exclude_patterns,events,action_type,destination,overwrite,preserve_structure,verify_integrity,shell,command,program,args,working_dir,message,exclude_regex\n\
               old,true,C:\\watch,true,file,false,*.csv,,,create,copy,D:\\out,false,false,false,,,,,,,\n";
    let err = convert(csv).unwrap_err().to_string();
    assert!(err.contains("21 列目"), "食い違った位置が分かること: {err}");
    assert!(err.contains("message"), "原因の列名が出ること: {err}");
}
