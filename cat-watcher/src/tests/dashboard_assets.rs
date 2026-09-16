//! index.html が参照しているファイルが、配信テーブルに登録されているかを守るテスト。
//! 登録し忘れるとブラウザ側が 404 で真っ白になり、原因も分かりにくいため。

use super::*;

/// `href="..."` / `src="..."` から `/assets/` 始まりのパスを抜き出す。
fn referenced_asset_paths(html: &str) -> Vec<String> {
    let mut found = Vec::new();
    for attr in ["href=\"", "src=\""] {
        let mut rest = html;
        while let Some(start) = rest.find(attr) {
            rest = &rest[start + attr.len()..];
            if let Some(end) = rest.find('"') {
                let value = &rest[..end];
                if value.starts_with("/assets/") {
                    found.push(value.to_string());
                }
                rest = &rest[end..];
            }
        }
    }
    found
}

#[test]
fn every_referenced_asset_is_registered() {
    let refs = referenced_asset_paths(INDEX.body);
    assert!(!refs.is_empty(), "index.html が /assets/ を1つも参照していない");
    for path in refs {
        assert!(
            find(&path).is_some(),
            "index.html が参照している {path} が ASSETS に登録されていない"
        );
    }
}

/// JS 同士の `import "./xxx.js"` も配信対象になっていること。
/// ES モジュールはブラウザが個別に取りに来るため、1 本でも欠けると動かない。
#[test]
fn every_js_import_is_registered() {
    for asset in ASSETS.iter().filter(|a| a.path.ends_with(".js")) {
        let mut rest = asset.body;
        while let Some(start) = rest.find("from \"./") {
            rest = &rest[start + "from \"./".len()..];
            let end = rest.find('"').expect("import 文が閉じていない");
            let name = &rest[..end];
            let path = format!("/assets/{name}");
            assert!(
                find(&path).is_some(),
                "{} が import している {path} が ASSETS に登録されていない",
                asset.path
            );
            rest = &rest[end..];
        }
    }
}

#[test]
fn asset_paths_are_unique() {
    let mut seen = Vec::new();
    for asset in ASSETS {
        assert!(!seen.contains(&asset.path), "パスが重複している: {}", asset.path);
        seen.push(asset.path);
    }
}

/// 表のヘッダーと本文が同じ列定義（CSS 変数 `--cols`）を共有していること。
/// ここがずれると見出しとデータの列が合わなくなる。
#[test]
fn table_header_and_rows_share_column_definition() {
    let css = find("/assets/style.css").expect("style.css が未登録").body;
    assert!(css.contains("--cols:"), "列定義 --cols が見当たらない");
    for selector in [".thead {", ".row {", ".hrow {"] {
        let start = css.find(selector).unwrap_or_else(|| panic!("{selector} が無い"));
        let block = &css[start..];
        let end = block.find('}').expect("ブロックが閉じていない");
        assert!(
            block[..end].contains("grid-template-columns: var(--cols)"),
            "{selector} が var(--cols) を使っていない"
        );
    }
}

/// JS が `$("xxx")` / `getElementById("xxx")` で触っている要素が
/// index.html に実在すること。
///
/// 欠けていても JS は例外で止まるだけで画面に何も出ないため、
/// ブラウザの開発者ツールを開くまで気づけない。
#[test]
fn every_element_id_used_by_js_exists_in_html() {
    let html = INDEX.body;
    for asset in ASSETS.iter().filter(|a| a.path.ends_with(".js")) {
        for id in element_ids_used(asset.body) {
            assert!(
                html.contains(&format!("id=\"{id}\"")),
                "{} が参照している id=\"{id}\" が index.html に無い",
                asset.path
            );
        }
    }
}

/// `$("name")` と `getElementById("name")` から id を集める。
fn element_ids_used(js: &str) -> Vec<String> {
    let mut found = Vec::new();
    for prefix in ["$(\"", "getElementById(\""] {
        let mut rest = js;
        while let Some(start) = rest.find(prefix) {
            rest = &rest[start + prefix.len()..];
            let Some(end) = rest.find('"') else { break };
            let id = &rest[..end];
            // 変数を渡している呼び出しなど、リテラルでないものは拾わない
            if !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                found.push(id.to_string());
            }
            rest = &rest[end..];
        }
    }
    found
}
