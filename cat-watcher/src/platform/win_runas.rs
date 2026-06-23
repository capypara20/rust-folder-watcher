#![cfg(windows)]
//! Windows サービス（SYSTEM 権限）から、アクティブなログオンユーザーの権限で
//! 外部プロセスを起動するためのモジュール。
//!
//! サービスは既定で LocalSystem（SYSTEM）アカウントで実行される。そのままだと
//! `command` / `execute` で起動する PowerShell・7-Zip などの外部プロセスも SYSTEM
//! 権限を継承してしまい、ログオンユーザーの環境（PATH・プロファイル・ネットワーク
//! ドライブ等）で動かず不便。そこで、アクティブなコンソールセッションのユーザー
//! トークンを取得し、`CreateProcessAsUserW` でログオンユーザー権限のプロセスを起動する。
//!
//! 誰もログオンしていない（アクティブセッションが無い・ユーザートークンを取得
//! できない）場合は [`RunAsResult::NoActiveUser`] を返し、呼び出し側は従来どおり
//! サービスアカウント権限での起動へフォールバックする。

use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, FALSE, HANDLE};
use windows_sys::Win32::Security::{
    DuplicateTokenEx, SecurityImpersonation, TokenPrimary, TOKEN_ALL_ACCESS,
};
use windows_sys::Win32::System::Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock};
use windows_sys::Win32::System::RemoteDesktop::{WTSGetActiveConsoleSessionId, WTSQueryUserToken};
use windows_sys::Win32::System::Threading::{
    CreateProcessAsUserW, CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION,
    STARTUPINFOW,
};

/// サービス起動時にログオンユーザー権限での実行を試みるか。
/// `main` の起動経路では設定されず、サービス起動経路でのみ設定される。
static RUN_AS_LOGGED_IN_USER: AtomicBool = AtomicBool::new(false);

/// ログオンユーザー権限での起動を有効化／無効化する。サービス起動時に
/// 設定値（`global.toml` の `[service].run_as_logged_in_user`）で呼ぶ。
pub fn set_enabled(enabled: bool) {
    RUN_AS_LOGGED_IN_USER.store(enabled, Ordering::SeqCst);
}

/// ログオンユーザー権限での起動が有効か。
pub fn enabled() -> bool {
    RUN_AS_LOGGED_IN_USER.load(Ordering::SeqCst)
}

/// [`spawn_as_active_user`] の結果。
pub enum RunAsResult {
    /// ログオンユーザー権限でプロセスを起動できた。
    Spawned,
    /// アクティブなログオンユーザーがいない。呼び出し側はサービスアカウント
    /// 権限での起動へフォールバックすべき。
    NoActiveUser,
    /// 起動を試みたが失敗した（フォールバックせずエラーとして扱う）。
    Err(String),
}

/// アクティブなコンソールセッションのログオンユーザー権限で外部プロセスを起動する。
pub fn spawn_as_active_user(
    program: &str,
    args: &[String],
    working_dir: Option<&str>,
) -> RunAsResult {
    // 1. 物理コンソールに紐づくアクティブセッションを取得する。
    //    0xFFFFFFFF はアクティブセッション無し（誰もログオンしていない）。
    let session_id = unsafe { WTSGetActiveConsoleSessionId() };
    if session_id == 0xFFFF_FFFF {
        return RunAsResult::NoActiveUser;
    }

    // 2. そのセッションのログオンユーザーのプライマリトークンを取得する。
    //    失敗（ログオンユーザー不在・権限不足等）はフォールバック対象。
    let mut user_token: HANDLE = ptr::null_mut();
    if unsafe { WTSQueryUserToken(session_id, &mut user_token) } == 0 {
        return RunAsResult::NoActiveUser;
    }

    // 3. CreateProcessAsUserW 用にプライマリトークンへ複製する。
    let mut primary_token: HANDLE = ptr::null_mut();
    let dup_ok = unsafe {
        DuplicateTokenEx(
            user_token,
            TOKEN_ALL_ACCESS,
            ptr::null(),
            SecurityImpersonation,
            TokenPrimary,
            &mut primary_token,
        )
    };
    unsafe { CloseHandle(user_token) };
    if dup_ok == 0 {
        return RunAsResult::Err(format!(
            "DuplicateTokenEx 失敗 (code={})",
            unsafe { GetLastError() }
        ));
    }

    let result = unsafe { create_process(primary_token, program, args, working_dir) };
    unsafe { CloseHandle(primary_token) };
    result
}

/// プライマリトークンを使って実際にプロセスを生成する。`primary_token` の
/// クローズは呼び出し側が行う。
unsafe fn create_process(
    token: HANDLE,
    program: &str,
    args: &[String],
    working_dir: Option<&str>,
) -> RunAsResult {
    // ログオンユーザーの環境変数ブロックを作る（PATH・USERPROFILE 等の引き継ぎ）。
    // 失敗してもプロセス起動自体は続行できるよう、null 環境でフォールバックする。
    let mut env_block: *mut c_void = ptr::null_mut();
    let env_ok = CreateEnvironmentBlock(&mut env_block, token, FALSE) != 0;

    let mut command_line = build_command_line(program, args);

    // 対話ユーザーのデスクトップに紐づける（GUI を出すツール対策）。
    let mut desktop = to_wide("winsta0\\default");

    let mut startup: STARTUPINFOW = std::mem::zeroed();
    startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    startup.lpDesktop = desktop.as_mut_ptr();

    let mut process_info: PROCESS_INFORMATION = std::mem::zeroed();

    let working_dir_wide = working_dir
        .filter(|d| !d.is_empty())
        .map(to_wide);
    let working_dir_ptr = working_dir_wide
        .as_ref()
        .map_or(ptr::null(), |w| w.as_ptr());

    let env_ptr: *const c_void = if env_ok { env_block } else { ptr::null() };

    let created = CreateProcessAsUserW(
        token,
        ptr::null(),                  // lpApplicationName（コマンドラインから解決）
        command_line.as_mut_ptr(),    // lpCommandLine（書き換え可能である必要がある）
        ptr::null(),                  // プロセスのセキュリティ属性
        ptr::null(),                  // スレッドのセキュリティ属性
        FALSE,                        // ハンドル継承なし
        CREATE_UNICODE_ENVIRONMENT | CREATE_NO_WINDOW,
        env_ptr,
        working_dir_ptr,
        &startup,
        &mut process_info,
    );

    let last_error = if created == 0 { GetLastError() } else { 0 };

    if env_ok {
        DestroyEnvironmentBlock(env_block);
    }

    if created == 0 {
        return RunAsResult::Err(format!("CreateProcessAsUserW 失敗 (code={last_error})"));
    }

    // 起動できたら fire-and-forget。プロセス／スレッドハンドルは閉じてよい
    // （プロセス自体は動き続ける）。
    CloseHandle(process_info.hProcess);
    CloseHandle(process_info.hThread);
    RunAsResult::Spawned
}

/// NUL 終端の UTF-16 文字列を作る。
fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// program + args から、CreateProcessAsUserW へ渡す書き換え可能な UTF-16 の
/// コマンドラインを組み立てる。クオート規則は Windows の `CommandLineToArgvW`
/// に合わせる（Rust 標準ライブラリの `make_command_line` と同じアルゴリズム）。
fn build_command_line(program: &str, args: &[String]) -> Vec<u16> {
    let mut cmd: Vec<u16> = Vec::new();
    append_quoted(program, &mut cmd);
    for arg in args {
        cmd.push(b' ' as u16);
        append_quoted(arg, &mut cmd);
    }
    cmd.push(0); // NUL 終端
    cmd
}

/// 1 つの引数を必要に応じてクオートしながら `cmd` へ追記する。
fn append_quoted(arg: &str, cmd: &mut Vec<u16>) {
    let units: Vec<u16> = arg.encode_utf16().collect();
    let needs_quotes = units.is_empty()
        || units
            .iter()
            .any(|&c| c == b' ' as u16 || c == b'\t' as u16 || c == b'"' as u16);

    if needs_quotes {
        cmd.push(b'"' as u16);
    }

    let mut backslashes: usize = 0;
    for &unit in &units {
        if unit == b'\\' as u16 {
            backslashes += 1;
        } else {
            if unit == b'"' as u16 {
                // 直前のバックスラッシュ列を 2 倍にし、さらに 1 つ足してから '"' を出す。
                for _ in 0..=backslashes {
                    cmd.push(b'\\' as u16);
                }
            }
            backslashes = 0;
        }
        cmd.push(unit);
    }

    if needs_quotes {
        // 閉じクオート直前のバックスラッシュ列も 2 倍にする。
        for _ in 0..backslashes {
            cmd.push(b'\\' as u16);
        }
        cmd.push(b'"' as u16);
    }
}

#[cfg(test)]
#[path = "win_runas_tests.rs"]
mod tests;
