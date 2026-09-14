//! プラットフォーム固有のコード。現状はすべて Windows 専用。
//!
//! - [`service`]   Windows サービス（SCM）としての常駐起動。
//! - [`win_runas`] サービス（SYSTEM 権限）からログオンユーザー権限で外部プロセスを起動する。
//! - [`win_job`]   起動した外部プロセスとその子孫をまとめて終了できるようにする。

pub mod service;
pub mod win_job;
pub mod win_runas;

/// 今このプロセスが動いているアカウント名を返す。形式は `DOMAIN\user (SID)`。
///
/// | 実行のしかた | 返る値 |
/// |---|---|
/// | サービス（LocalSystem） | `NT AUTHORITY\SYSTEM (S-1-5-18)` |
/// | サービス（LocalService） | `NT AUTHORITY\LOCAL SERVICE (S-1-5-19)` |
/// | サービス（NetworkService） | `NT AUTHORITY\NETWORK SERVICE (S-1-5-20)` |
/// | CLI 実行 | `DESKTOP-XXXX\roze (S-1-5-21-...)` |
///
/// 「ネットワーク共有が見えない」「PowerShell が SYSTEM で動く」といった相談は、
/// ほぼ実行アカウントの問題に行き着く。起動ログに出しておけば、`sc config` で
/// 実行アカウントを変えたつもりが変わっていない、といった食い違いにすぐ気づける。
///
/// **プロセストークンから引く。** 環境変数（`USERNAME` / `USERDOMAIN`）だと
/// SYSTEM で動いていてもマシンアカウント名（`WORKGROUP\DESKTOP-XXXX$`）になり、
/// LocalSystem / LocalService / NetworkService の区別がつかない。
/// SID を併記するのは、名前がローカライズされていても
/// `S-1-5-18` / `S-1-5-19` / `S-1-5-20` で判別できるようにするため。
///
/// 取得に失敗したときだけ環境変数へフォールバックする（この関数は失敗しない契約）。
pub fn current_account() -> String {
    account_from_token().unwrap_or_else(account_from_env)
}

/// プロセストークンのユーザー SID から `DOMAIN\user (SID)` を組み立てる。
fn account_from_token() -> Option<String> {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return None;
        }

        // 1 回目は必要なバッファ長を知るためだけに呼ぶ（必ず失敗する）。
        let mut len: u32 = 0;
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut len);

        let account = if len == 0 {
            None
        } else {
            let mut buf = vec![0u8; len as usize];
            if GetTokenInformation(
                token,
                TokenUser,
                buf.as_mut_ptr() as *mut core::ffi::c_void,
                len,
                &mut len,
            ) == 0
            {
                None
            } else {
                // TOKEN_USER の先頭は SID_AND_ATTRIBUTES。Sid は buf の中を指す。
                let sid = (*(buf.as_ptr() as *const TOKEN_USER)).User.Sid;
                match (lookup_account_name(sid), sid_to_string(sid)) {
                    (Some(name), Some(sid_text)) => Some(format!("{name} ({sid_text})")),
                    // 名前が引けなくても SID だけで SYSTEM かどうかは判別できる。
                    (None, Some(sid_text)) => Some(sid_text),
                    (Some(name), None) => Some(name),
                    (None, None) => None,
                }
            }
        };

        CloseHandle(token);
        account
    }
}

/// SID を `DOMAIN\user` に変換する。ドメイン部が無ければ名前だけ返す。
unsafe fn lookup_account_name(sid: windows_sys::Win32::Security::PSID) -> Option<String> {
    use windows_sys::Win32::Security::{LookupAccountSidW, SID_NAME_USE};

    let mut name_len: u32 = 0;
    let mut domain_len: u32 = 0;
    let mut sid_type: SID_NAME_USE = 0;

    // 1 回目は長さの問い合わせ。
    LookupAccountSidW(
        std::ptr::null(),
        sid,
        std::ptr::null_mut(),
        &mut name_len,
        std::ptr::null_mut(),
        &mut domain_len,
        &mut sid_type,
    );
    if name_len == 0 {
        return None;
    }

    let mut name = vec![0u16; name_len as usize];
    let mut domain = vec![0u16; domain_len.max(1) as usize];
    if LookupAccountSidW(
        std::ptr::null(),
        sid,
        name.as_mut_ptr(),
        &mut name_len,
        domain.as_mut_ptr(),
        &mut domain_len,
        &mut sid_type,
    ) == 0
    {
        return None;
    }

    let name = wide_to_string(&name);
    let domain = wide_to_string(&domain);
    if domain.is_empty() {
        Some(name)
    } else {
        Some(format!("{domain}\\{name}"))
    }
}

/// SID を `S-1-5-18` のような文字列表現にする。
unsafe fn sid_to_string(sid: windows_sys::Win32::Security::PSID) -> Option<String> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;

    let mut raw: windows_sys::core::PWSTR = std::ptr::null_mut();
    if ConvertSidToStringSidW(sid, &mut raw) == 0 || raw.is_null() {
        return None;
    }

    let mut len = 0usize;
    while *raw.add(len) != 0 {
        len += 1;
    }
    let text = String::from_utf16_lossy(std::slice::from_raw_parts(raw, len));

    // ConvertSidToStringSidW が確保したバッファは呼び出し側が解放する。
    LocalFree(raw as *mut core::ffi::c_void);
    Some(text)
}

/// NUL 終端の UTF-16 バッファを String にする。
fn wide_to_string(units: &[u16]) -> String {
    let end = units.iter().position(|&c| c == 0).unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

/// トークンから引けなかったときのフォールバック。
///
/// SYSTEM で動いているとマシンアカウント名になってしまうが、
/// 何も出さないよりは手がかりになる。
fn account_from_env() -> String {
    let user = std::env::var("USERNAME").unwrap_or_default();
    if user.is_empty() {
        return "(不明)".to_string();
    }
    match std::env::var("USERDOMAIN") {
        Ok(domain) if !domain.is_empty() => format!("{domain}\\{user}"),
        _ => user,
    }
}

#[cfg(test)]
#[path = "../tests/platform.rs"]
mod tests;
