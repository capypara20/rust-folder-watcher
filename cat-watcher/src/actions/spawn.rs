use std::process::Stdio;
use std::time::Duration;

/// 外部プロセスへ渡す引数 1 つ。
///
/// Windows ではコマンドラインが 1 本の文字列としてプロセスへ渡るため、引数を
/// どうクオートするかで結果が変わる。ほとんどのプログラムは
/// `CommandLineToArgvW` の規則で解釈する（内部のダブルクオートはバックスラッシュで
/// エスケープする）が、**`cmd.exe` だけは独自の規則を持ち、バックスラッシュを
/// エスケープ文字として扱わない**。
///
/// そのため `cmd /C` へ渡すコマンドを argv 規則でクオートすると、エスケープ用の
/// バックスラッシュがそのまま文字として残り、コマンドが壊れる。
/// クオートする引数とそのまま載せる引数を、型で区別して取り違えを防ぐ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnArg {
    /// Windows の argv 規則（`CommandLineToArgvW` 準拠）でクオートして渡す。
    Quoted(String),
    /// クオートせずコマンドラインへそのまま載せる。
    /// 受け手が独自のクオート規則を持つ場合（`cmd /C` のコマンド部分）に使う。
    Raw(String),
}

/// 起動した外部プロセスの終了を待つかどうか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitMode {
    /// 待たない。起動できたらすぐ次のアクションへ進む（従来の動作）。
    Detach,
    /// 終了まで待つ。`timeout` が `Some` ならその時間を超えた時点で強制終了する
    /// （`None` は無制限）。
    Wait { timeout: Option<Duration> },
}

/// 設定値（解決済みの `wait` / `timeout_ms`）から待ち方を決める。
///
/// `timeout_ms` が未指定または `0` の場合は「無制限」として扱う。
pub fn wait_mode_from(wait: Option<bool>, timeout_ms: Option<u64>) -> WaitMode {
    if wait.unwrap_or(false) {
        WaitMode::Wait {
            timeout: timeout_ms.filter(|ms| *ms > 0).map(Duration::from_millis),
        }
    } else {
        WaitMode::Detach
    }
}

/// 外部プロセスを起動した結果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnOutcome {
    /// 終了を待たずに起動だけした。
    Detached,
    /// 終了まで待った。終了コードが取得できなかった場合は `None`
    /// （Unix でシグナルにより終了した場合など）。
    Exited(Option<i32>),
    /// 上限時間を超えたので強制終了した。
    TimedOut,
}

/// 外部プロセスを起動する。
///
/// 通常は OS 既定の権限で起動する（CLI 実行ならログオンユーザー、サービス実行
/// ならサービスアカウント）。Windows のサービスモードかつ `run_as_logged_in_user`
/// が有効な場合のみ、アクティブなログオンユーザーの権限での起動を試み、
/// ログオンユーザーがいなければサービスアカウント権限へフォールバックする。
///
/// 起動失敗時はエラー内容の文字列を返す。呼び出し側でアクション種別ごとの
/// メッセージへ整形する。
pub async fn spawn_process(
    program: &str,
    args: &[SpawnArg],
    working_dir: Option<&str>,
    wait: WaitMode,
) -> Result<SpawnOutcome, String> {
    #[cfg(windows)]
    {
        use crate::platform::win_runas::{self, RunAsResult};
        if win_runas::enabled() {
            // CreateProcessAsUserW も WaitForSingleObject もブロッキング呼び出しな
            // ので、tokio のワーカースレッドを塞がないよう専用スレッドへ出す。
            let program = program.to_string();
            let args = args.to_vec();
            let working_dir = working_dir.map(|s| s.to_string());
            let result = tokio::task::spawn_blocking(move || {
                win_runas::spawn_as_active_user(&program, &args, working_dir.as_deref(), wait)
            })
            .await
            .map_err(|e| format!("起動タスクの実行に失敗: {e}"))?;

            match result {
                RunAsResult::Spawned(outcome) => return Ok(outcome),
                // ログオンユーザー不在: サービスアカウント権限での起動へフォールバック
                RunAsResult::NoActiveUser => {}
                RunAsResult::Err(e) => return Err(e),
            }
        }
    }

    let mut cmd = tokio::process::Command::new(program);
    cmd.stdout(Stdio::null()).stderr(Stdio::null());

    #[cfg(windows)]
    {
        for arg in args {
            match arg {
                SpawnArg::Quoted(s) => cmd.arg(s),
                // raw_arg はクオート処理をせずコマンドラインへそのまま追記する。
                SpawnArg::Raw(s) => cmd.raw_arg(s),
            };
        }
    }
    #[cfg(not(windows))]
    {
        // Unix では引数が配列のまま execve へ渡るので、クオートの問題は起きない。
        // Raw と Quoted を区別する必要がないため、どちらもそのまま 1 引数として渡す。
        for arg in args {
            match arg {
                SpawnArg::Quoted(s) | SpawnArg::Raw(s) => cmd.arg(s),
            };
        }
    }

    if let Some(dir) = working_dir.filter(|s| !s.is_empty()) {
        cmd.current_dir(dir);
    }

    match wait {
        WaitMode::Detach => cmd
            .spawn()
            .map(|_| SpawnOutcome::Detached)
            .map_err(|e| e.to_string()),
        WaitMode::Wait { timeout } => {
            // 上限を超えたときに「シェルだけ」ではなくツリー全体を終了させたい。
            // command アクションは必ず cmd.exe / bash を経由するので、実際の処理を
            // するプロセスは常に孫になる。シェルを殺しても処理は止まらない。
            #[cfg(unix)]
            if timeout.is_some() {
                // 新しいプロセスグループにしておけば killpg でまとめて落とせる。
                cmd.process_group(0);
            }

            let mut child = cmd.spawn().map_err(|e| e.to_string())?;

            // Windows は Job Object に入れておき、TerminateJobObject でツリーごと
            // 終了させる。割り当てに失敗してもツリー終了ができなくなるだけなので、
            // 起動自体は続行する。
            #[cfg(windows)]
            let job = timeout.and_then(|_| {
                child
                    .raw_handle()
                    .and_then(|h| crate::platform::win_job::JobHandle::assign(h).ok())
            });

            let Some(limit) = timeout else {
                return child
                    .wait()
                    .await
                    .map(|status| SpawnOutcome::Exited(status.code()))
                    .map_err(|e| e.to_string());
            };

            match tokio::time::timeout(limit, child.wait()).await {
                Ok(Ok(status)) => Ok(SpawnOutcome::Exited(status.code())),
                Ok(Err(e)) => Err(e.to_string()),
                Err(_) => {
                    // 上限超過。子孫を含めて終了させる。
                    #[cfg(windows)]
                    if let Some(job) = &job {
                        job.terminate();
                    }
                    #[cfg(unix)]
                    if let Some(pid) = child.id() {
                        // プロセスグループ全体へ SIGKILL を送る。
                        unsafe { libc::killpg(pid as i32, libc::SIGKILL) };
                    }
                    // シェル自身も確実に終了させ、ゾンビを残さないよう回収する。
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                    Ok(SpawnOutcome::TimedOut)
                }
            }
        }
    }
}

/// 起動結果をアクションログ用の文言へ変える。
///
/// `Ok` は成功としてそのまま表示する文言、`Err` は失敗理由。
/// 呼び出し側が「どのアクションか」の情報を足してエラーメッセージを組み立てる。
pub fn outcome_message(outcome: SpawnOutcome) -> Result<String, String> {
    match outcome {
        // wait = false のときの表記。従来と同じにしてログの形を変えない。
        SpawnOutcome::Detached => Ok("起動".to_string()),
        SpawnOutcome::Exited(Some(0)) => Ok("完了 (exit=0)".to_string()),
        SpawnOutcome::Exited(Some(code)) => Err(format!("異常終了しました (exit={code})")),
        // 終了はしたが終了コードが取れなかった場合。失敗とは断定できないので成功扱い。
        SpawnOutcome::Exited(None) => Ok("完了（終了コードは取得できず）".to_string()),
        SpawnOutcome::TimedOut => {
            Err("時間内に終わらないため強制終了しました".to_string())
        }
    }
}
