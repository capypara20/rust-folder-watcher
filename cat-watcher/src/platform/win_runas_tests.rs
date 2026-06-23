use super::*;

/// UTF-16 のコマンドラインを Rust 文字列へ戻す（NUL 終端は取り除く）。
fn render(program: &str, args: &[String]) -> String {
    let mut units = build_command_line(program, args);
    assert_eq!(units.last(), Some(&0), "コマンドラインは NUL 終端である必要がある");
    units.pop();
    String::from_utf16(&units).unwrap()
}

#[test]
fn plain_program_and_args_are_space_joined() {
    let line = render("7z.exe", &["a".to_string(), "out.7z".to_string()]);
    assert_eq!(line, "7z.exe a out.7z");
}

#[test]
fn args_with_spaces_get_quoted() {
    let line = render(
        "7z.exe",
        &["a".to_string(), "C:\\Program Files\\out.7z".to_string()],
    );
    assert_eq!(line, "7z.exe a \"C:\\Program Files\\out.7z\"");
}

#[test]
fn empty_arg_is_quoted() {
    let line = render("prog.exe", &["".to_string()]);
    assert_eq!(line, "prog.exe \"\"");
}

#[test]
fn embedded_quote_is_escaped() {
    let line = render("prog.exe", &["a\"b".to_string()]);
    // クオートが含まれるので全体をクオートし、内部の '"' は \" にエスケープする。
    assert_eq!(line, "prog.exe \"a\\\"b\"");
}

#[test]
fn trailing_backslashes_before_closing_quote_are_doubled() {
    // スペースを含むのでクオートされ、末尾のバックスラッシュは 2 倍になる。
    let line = render("prog.exe", &["a b\\\\".to_string()]);
    assert_eq!(line, "prog.exe \"a b\\\\\\\\\"");
}
