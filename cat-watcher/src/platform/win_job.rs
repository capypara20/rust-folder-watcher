#![cfg(windows)]
//! Job Object を使って「起動したプロセスとその子孫」をまとめて終了するためのモジュール。
//!
//! Windows では親プロセスを終了させても子プロセスは生き残る。`command` アクションは
//! 必ず `cmd.exe` / `powershell.exe` を経由するため、実際の処理をするプロセスは常に
//! 「孫」になる。つまりシェルだけを終了させても処理は止まらない。
//!
//! 起動したプロセスを Job Object に入れておけば、[`JobHandle::terminate`] で
//! ツリー全体を終了できる。

use std::ptr;

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, TerminateJobObject,
};

/// プロセスツリーをまとめて終了できる Job Object のハンドル。
///
/// `Drop` でハンドルを閉じるだけで、中のプロセスは終了させない
/// （`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` は付けていない）。
/// 正常終了したシェルがバックグラウンドのプロセスを残していた場合に、
/// それを巻き込んで殺してしまわないようにするため。
/// 終了させるのは [`terminate`](Self::terminate) を明示的に呼んだときだけ。
pub struct JobHandle {
    handle: HANDLE,
}

// Job のハンドルは別スレッドへ渡して使える。
unsafe impl Send for JobHandle {}
unsafe impl Sync for JobHandle {}

impl JobHandle {
    /// Job Object を作り、指定したプロセスを割り当てる。
    ///
    /// 失敗しても致命的ではない（ツリーごとの終了ができなくなるだけ）ので、
    /// 呼び出し側はエラーを記録して起動自体は続行してよい。
    pub fn assign(process: HANDLE) -> Result<Self, String> {
        let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if handle.is_null() {
            return Err(format!("CreateJobObjectW 失敗 (code={})", unsafe {
                GetLastError()
            }));
        }
        // ここから先で失敗しても Drop がハンドルを閉じる。
        let job = Self { handle };

        if unsafe { AssignProcessToJobObject(job.handle, process) } == 0 {
            return Err(format!("AssignProcessToJobObject 失敗 (code={})", unsafe {
                GetLastError()
            }));
        }
        Ok(job)
    }

    /// Job に属するプロセスをすべて終了させる（子孫も含む）。
    pub fn terminate(&self) {
        unsafe { TerminateJobObject(self.handle, 1) };
    }
}

impl Drop for JobHandle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.handle) };
    }
}
