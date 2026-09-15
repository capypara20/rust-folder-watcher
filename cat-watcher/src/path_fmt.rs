//! ログへ出すパスの表記を、その OS が標準とする区切り文字へ揃える。
//!
//! 監視イベントのパスは「設定に書かれた監視ルートの文字列」＋「OS が繋いだ相対部分」で
//! できている。そのため設定に `C:/watch` と書くと、ログには
//! `C:/watch\file.txt` のように区切り文字が混ざって出る。読みにくいうえ、
//! ログを検索するときにも邪魔になる。
//!
//! **表記を揃えるだけで、実際のファイル操作に使うパスは変えない。**
//! プレースホルダ（`{FullName}` など）は従来どおり `/` へ正規化される。
//! こちらは README に明記されている仕様なので、ここでは触らない。

use std::path::Path;

/// ログへ出すパスの文字列を作る。
pub fn for_log<P: AsRef<Path>>(path: P) -> String {
    normalize(&path.as_ref().display().to_string())
}

/// すでに文字列になっているパスの区切り文字を揃える。
///
/// **Windows でだけ置き換える。** Unix ではバックスラッシュがファイル名に使える
/// 文字なので、区切り文字と決めつけて置き換えるとファイル名を壊してしまう。
/// Unix のパスは元から `/` 区切りなので、そもそも揃える必要がない。
pub fn normalize(text: &str) -> String {
    #[cfg(windows)]
    {
        // Windows はファイル名に / を使えないため、区切り文字とみなして置き換えてよい。
        text.replace('/', "\u{5c}")
    }
    #[cfg(not(windows))]
    {
        text.to_string()
    }
}

#[cfg(test)]
#[path = "tests/path_fmt.rs"]
mod tests;
