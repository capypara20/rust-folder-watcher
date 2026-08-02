//! プラットフォーム固有のコード。現状はすべて Windows 専用。
//!
//! - [`service`]   Windows サービス（SCM）としての常駐起動。
//! - [`win_runas`] サービス（SYSTEM 権限）からログオンユーザー権限で外部プロセスを起動する。

pub mod service;
pub mod win_runas;

/// 今このプロセスが動いているアカウント名（`DOMAIN\user` 形式）を返す。
///
/// 「ネットワーク共有が見えない」「PowerShell が SYSTEM で動く」といった相談は、
/// ほぼ実行アカウントの問題に行き着く。起動ログに出しておけば、`sc config` で
/// 実行アカウントを変えたつもりが変わっていない、といった食い違いにすぐ気づける。
///
/// 環境変数から組み立てるだけなので失敗しない（取れない場合は `"(不明)"`）。
/// サービスを既定（LocalSystem）で動かしていると `MACHINE\SYSTEM` のようになる。
pub fn current_account() -> String {
    let user = std::env::var("USERNAME").unwrap_or_default();
    if user.is_empty() {
        return "(不明)".to_string();
    }
    match std::env::var("USERDOMAIN") {
        Ok(domain) if !domain.is_empty() => format!("{domain}\\{user}"),
        _ => user,
    }
}
