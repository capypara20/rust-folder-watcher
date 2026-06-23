fn main() {
    // CAT_WATCHER_SKIP_WINRES=1 のときは Windows リソース埋め込みをスキップする。
    // Windows 以外のホストから `cargo check --target *-windows-*` で
    // Windows 専用コードを型検査したいときに使う（リソースコンパイラ不要にする）。
    if std::env::var("CAT_WATCHER_SKIP_WINRES").as_deref() == Ok("1") {
        return;
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winres::WindowsResource::new();
        res.set("FileDescription", "ファイル監視・自動処理ツール");
        res.set("ProductName", "cat-watcher");
        res.set("LegalCopyright", "Copyright \u{00a9} 2026 capypara20");
        res.compile().expect("Windows リソースのコンパイルに失敗");
    }
}
