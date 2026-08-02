//! ブラウザへ配信する静的アセット（HTML / CSS / JS）。
//!
//! 実体は `assets/` 配下のふつうのファイルで、`include_str!` でバイナリに
//! 埋め込む。こうすると「単一 exe で配布できる」ことは変えずに、編集時は
//! HTML は HTML、CSS は CSS として（エディタの補完や色分けを効かせて）
//! 触れるようになる。
//!
//! アセットを増やしたら [`ASSETS`] に 1 行足すこと。`index.html` から
//! 参照しているのに登録し忘れると 404 になるが、テストで検出できる。

pub(super) struct Asset {
    /// リクエストパス（`/assets/style.css` のような絶対パス）。
    pub path: &'static str,
    /// `Content-Type` ヘッダにそのまま載せる値。
    pub content_type: &'static str,
    pub body: &'static str,
}

/// ルート（`/`）で返すダッシュボード本体。
pub(super) const INDEX: Asset = Asset {
    path: "/",
    content_type: "text/html; charset=utf-8",
    body: include_str!("assets/index.html"),
};

const CSS: &str = "text/css; charset=utf-8";
const JS: &str = "text/javascript; charset=utf-8";

pub(super) const ASSETS: &[Asset] = &[
    Asset { path: "/assets/style.css",  content_type: CSS, body: include_str!("assets/style.css") },
    Asset { path: "/assets/app.js",     content_type: JS,  body: include_str!("assets/app.js") },
    Asset { path: "/assets/search.js",  content_type: JS,  body: include_str!("assets/search.js") },
    Asset { path: "/assets/render.js",  content_type: JS,  body: include_str!("assets/render.js") },
];

/// リクエストパスに対応するアセットを探す。
pub(super) fn find(path: &str) -> Option<&'static Asset> {
    ASSETS.iter().find(|a| a.path == path)
}

#[cfg(test)]
#[path = "../tests/dashboard_assets.rs"]
mod tests;
