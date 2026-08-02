use super::*;
use crate::config::RulesConfig;

/// 変換した TOML を実際にパースして、rules.toml として成立しているか確かめる。
fn convert_and_parse(csv: &str) -> RulesConfig {
    let toml_text = convert(csv).expect("変換に失敗");
    toml::from_str::<RulesConfig>(&toml_text)
        .unwrap_or_else(|e| panic!("生成された TOML がパースできない: {e}\n---\n{toml_text}"))
}

const HEADER: &str = "rule_name,enabled,watch_path,recursive,target,include_hidden,patterns,regex,exclude_patterns,events,action_type,destination,overwrite,preserve_structure,verify_integrity,shell,command,program,args,working_dir,message,exclude_regex,dir_patterns,dir_regex,exclude_dir_patterns,exclude_dir_regex,auto_create,delay_ms";

// Windows パスをそのまま埋め込むと \t などがエスケープと解釈されて
// 壊れた TOML になっていたバグの回帰テスト。
#[test]
fn windows_paths_are_escaped() {
    let csv = format!(
        "{HEADER}\n\
         win,true,C:\\watch,true,file,false,*.csv,,,create,execute,,,,,,,C:\\tool\\app.exe,C:\\temp\\out|--flag,C:\\work,,,,,,,\n"
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
        "{HEADER}\n\
         excel,TRUE,C:\\watch,TRUE,file,FALSE,*.csv,,,create,copy,D:\\out,TRUE,FALSE,TRUE,,,,,,,,,,,,,\n"
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
        "{HEADER}\n\
         bad,はい,C:\\watch,true,file,false,*.csv,,,create,log,,,,,,,,,,msg,,,,,,,\n"
    );
    let err = convert(&csv).unwrap_err().to_string();
    assert!(err.contains("enabled"), "どの列か分かるメッセージであること: {err}");
}

// 末尾に追加した auto_create / delay_ms 列が反映されること。
#[test]
fn auto_create_and_delay_columns_are_applied() {
    let csv = format!(
        "{HEADER}\n\
         cols,true,C:\\watch,true,file,false,*.csv,,,create,copy,D:\\out,false,false,false,,,,,,,,,,,,false,1500\n"
    );
    let parsed = convert_and_parse(&csv);
    let action = &parsed.rules[0].actions[0];

    assert_eq!(action.auto_create, Some(false));
    assert_eq!(action.delay_ms, Some(1500));
}

// 列を後ろに足したので、古い（短い）CSV もそのまま読めること。
#[test]
fn legacy_short_rows_still_work() {
    // 21 列だけの旧フォーマット（exclude_regex 以降が無い）
    let csv = "rule_name,enabled,watch_path,recursive,target,include_hidden,patterns,regex,exclude_patterns,events,action_type,destination,overwrite,preserve_structure,verify_integrity,shell,command,program,args,working_dir,message\n\
               legacy,true,C:\\watch,true,file,false,*.csv,,,create,log,,,,,,,,,,検知: {BaseName}\n";
    let parsed = convert_and_parse(csv);

    assert_eq!(parsed.rules[0].name, "legacy");
    assert_eq!(parsed.rules[0].actions[0].message.as_deref(), Some("検知: {BaseName}"));
    assert_eq!(parsed.rules[0].actions[0].auto_create, None, "未指定なら global に従う");
}

// 同じ rule_name の行を並べると 1 ルールの複数アクションになる。
#[test]
fn same_rule_name_becomes_multiple_actions() {
    let csv = format!(
        "{HEADER}\n\
         multi,true,C:\\watch,true,file,false,*.csv,,,create,copy,D:\\out,false,false,false,,,,,,,,,,,,,\n\
         multi,true,C:\\watch,true,file,false,*.csv,,,create,log,,,,,,,,,,done,,,,,,,\n"
    );
    let parsed = convert_and_parse(&csv);

    assert_eq!(parsed.rules.len(), 1);
    assert_eq!(parsed.rules[0].actions.len(), 2);
}

// ルール名やメッセージに引用符が入っていても壊れない。
#[test]
fn quotes_inside_values_are_escaped() {
    let csv = format!(
        "{HEADER}\n\
         \"say \"\"hi\"\"\",true,C:\\watch,true,file,false,*.csv,,,create,log,,,,,,,,,,\"a \"\"b\"\" c\",,,,,,,\n"
    );
    let parsed = convert_and_parse(&csv);

    assert_eq!(parsed.rules[0].name, r#"say "hi""#);
    assert_eq!(parsed.rules[0].actions[0].message.as_deref(), Some(r#"a "b" c"#));
}
