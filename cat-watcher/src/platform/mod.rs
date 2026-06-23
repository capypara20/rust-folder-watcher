//! プラットフォーム固有のコード。現状はすべて Windows 専用。
//!
//! - [`service`]   Windows サービス（SCM）としての常駐起動。
//! - [`win_runas`] サービス（SYSTEM 権限）からログオンユーザー権限で外部プロセスを起動する。

pub mod service;
pub mod win_runas;
