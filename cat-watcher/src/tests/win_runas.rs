use super::*;

/// argv 規則でクオートする引数を作るショートカット。
fn q(s: &str) -> SpawnArg {
    SpawnArg::Quoted(s.to_string())
}

/// UTF-16 のコマンドラインを Rust 文字列へ戻す（NUL 終端は取り除く）。
fn render(program: &str, args: &[SpawnArg]) -> String {
    let mut units = build_command_line(program, args);
    assert_eq!(units.last(), Some(&0), "コマンドラインは NUL 終端である必要がある");
    units.pop();
    String::from_utf16(&units).unwrap()
}

#[test]
fn plain_program_and_args_are_space_joined() {
    let line = render("7z.exe", &[q("a"), q("out.7z")]);
    assert_eq!(line, "7z.exe a out.7z");
}

#[test]
fn args_with_spaces_get_quoted() {
    let line = render("7z.exe", &[q("a"), q(r"C:\Program Files\out.7z")]);
    assert_eq!(line, r#"7z.exe a "C:\Program Files\out.7z""#);
}

#[test]
fn empty_arg_is_quoted() {
    let line = render("prog.exe", &[q("")]);
    assert_eq!(line, "prog.exe \"\"");
}

#[test]
fn embedded_quote_is_escaped() {
    let line = render("prog.exe", &[q("a\"b")]);
    // クオートが含まれるので全体をクオートし、内部の '"' はエスケープする。
    assert_eq!(line, "prog.exe \"a\\\"b\"");
}

#[test]
fn trailing_backslashes_before_closing_quote_are_doubled() {
    // スペースを含むのでクオートされ、末尾のバックスラッシュは 2 倍になる。
    let line = render("prog.exe", &[q("a b\\\\")]);
    assert_eq!(line, "prog.exe \"a b\\\\\\\\\"");
}

/// Raw はクオート処理を通さず、そのままコマンドラインへ載ること。
///
/// cmd.exe はバックスラッシュをエスケープ文字として扱わないため、ここで
/// argv 規則のクオートが混ざると渡すコマンドが壊れる（それが元のバグ）。
#[test]
fn raw_arg_is_appended_verbatim() {
    let raw = r#""echo RAN > "C:\out\ran.txt"""#;
    let line = render("cmd.exe", &[q("/C"), SpawnArg::Raw(raw.to_string())]);
    assert_eq!(line, format!("cmd.exe /C {raw}"));
    assert!(
        !line.contains("\\\""),
        "Raw 引数にエスケープが入ってはいけない: {line}"
    );
}
