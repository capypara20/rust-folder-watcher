#![cfg(windows)]

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use windows_service::{
    define_windows_service,
    service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult, ServiceStatusHandle},
    service_dispatcher,
};

use crate::config;
use crate::error::AppError;
use crate::logger::Logger;
use crate::watcher;

const SERVICE_NAME: &str = "cat-watcher";

define_windows_service!(ffi_service_main, service_main);

/// SCM からの起動を試みる。サービスとして起動されていた場合は処理完了まで
/// ブロックして true を返す。通常の CLI 起動の場合は即座に false を返す。
pub fn try_run_as_service() -> bool {
    match service_dispatcher::start(SERVICE_NAME, ffi_service_main) {
        Ok(_) => true,
        Err(ref e) if is_not_service_context(e) => false,
        Err(e) => {
            eprintln!("サービスディスパッチャー起動失敗: {e}");
            std::process::exit(1);
        }
    }
}

// ERROR_FAILED_SERVICE_CONTROLLER_CONNECT (1063) = CLI から直接起動された
fn is_not_service_context(e: &windows_service::Error) -> bool {
    matches!(e, windows_service::Error::Winapi(ref io_err) if io_err.raw_os_error() == Some(1063))
}

fn service_main(arguments: Vec<OsString>) {
    if let Err(e) = run_service(&arguments) {
        // サービスプロセスには標準エラー出力の行き先が無いため、これは
        // どこにも表示されない。実際の記録は run_watcher 側の
        // write_startup_error_log が行う。ここは CLI から誤って
        // サービス経路に入った場合の保険。
        eprintln!("サービス実行エラー: {e}");
    }
}

/// サービスの終了コード。`sc query` から見えるので、原因の種類が区別できるようにする。
///
/// 従来は常に `Win32(1)`（= `ERROR_INVALID_FUNCTION`「ファンクションが間違っています」）
/// で、イベントログを見ても何も分からなかった。
mod exit_code {
    /// 設定ファイルの読み込み・パース・バリデーションに失敗した。
    pub const CONFIG: u32 = 10;
    /// ログの初期化に失敗した（出力先が作れない等）。
    pub const LOG: u32 = 11;
    /// 監視の実行中に致命的エラーが起きた。
    pub const RUNTIME: u32 = 12;
}

/// 起動失敗の内容を書き出すファイル名。
const STARTUP_ERROR_LOG: &str = "cat-watcher-startup-error.log";

/// 起動失敗の記録先の候補。前から順に試し、1 つ書けたら終わりにする。
///
/// 実行ファイル横が第一候補。`Program Files` 配下など書き込めない場合の保険として
/// カレントディレクトリと TEMP も見る。
fn startup_error_log_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            paths.push(dir.join(STARTUP_ERROR_LOG));
        }
    }
    if let Ok(dir) = std::env::current_dir() {
        paths.push(dir.join(STARTUP_ERROR_LOG));
    }
    if let Some(tmp) = std::env::var_os("TEMP") {
        paths.push(PathBuf::from(tmp).join(STARTUP_ERROR_LOG));
    }
    paths
}

/// 設定が読めなくても必ず書ける場所へ、起動失敗の内容を残す。
///
/// サービスは標準エラー出力の行き先が無く、設定エラーのときはログ設定自体が
/// 読めていないためログファイルも作られない。イベントログには
/// 「ファンクションが間違っています」しか出ないので調査に使えない。
/// そこで固定名のファイルへ追記する。
fn write_startup_error_log(args: Option<&ServiceArgs>, err: &AppError) {
    let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let mut body = format!("[{ts}] サービス起動に失敗しました\n");
    match args {
        Some(a) => {
            body.push_str(&format!("  --global : {}\n", a.global.display()));
            body.push_str(&format!("  --rules  : {}\n", a.rules.display()));
        }
        None => body.push_str("  引数の解析に失敗したため、設定ファイルのパスは不明です\n"),
    }
    body.push_str(&format!(
        "  実行アカウント : {}\n",
        super::current_account()
    ));
    body.push_str(&format!("  エラー : {err}\n\n"));

    for path in startup_error_log_paths() {
        if append_text(&path, &body).is_ok() {
            return;
        }
    }
}

/// ファイルへ追記する。親ディレクトリは作らない（固定の既存フォルダが前提）。
fn append_text(path: &Path, body: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    f.write_all(body.as_bytes())
}

/// `AppError` から終了コードを決める。
fn exit_code_for(err: &AppError) -> u32 {
    match err {
        AppError::Config(_) | AppError::Validation(_) | AppError::TomlParse(_) => exit_code::CONFIG,
        AppError::Io(_) => exit_code::LOG,
        _ => exit_code::RUNTIME,
    }
}

fn run_service(arguments: &[OsString]) -> Result<(), AppError> {
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let stop_tx = Arc::new(Mutex::new(Some(stop_tx)));

    let stop_tx_for_handler = Arc::clone(&stop_tx);
    let event_handler = move |control_event| -> ServiceControlHandlerResult {
        match control_event {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                if let Ok(mut guard) = stop_tx_for_handler.lock() {
                    if let Some(tx) = guard.take() {
                        let _ = tx.send(());
                    }
                }
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };

    let service_name = arguments
        .first()
        .and_then(|s| s.to_str())
        .unwrap_or(SERVICE_NAME);

    let status_handle = service_control_handler::register(service_name, event_handler)
        .map_err(|e| AppError::Config(format!("SCM登録失敗: {e}")))?;

    let result = run_watcher(status_handle, stop_rx);

    // 失敗の種類が `sc query` から区別できるように、固有の終了コードを返す。
    let exit_code = match &result {
        Ok(_) => ServiceExitCode::Win32(0),
        Err(e) => ServiceExitCode::ServiceSpecific(exit_code_for(e)),
    };
    let _ = status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code,
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    });

    result
}

/// 起動途中であることを SCM に伝える。`checkpoint` を進めると「進行中」と見なされ、
/// `wait_hint` の間は待ってもらえる。
fn set_start_pending(status_handle: &ServiceStatusHandle, checkpoint: u32) {
    let _ = status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::StartPending,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint,
        wait_hint: Duration::from_secs(30),
        process_id: None,
    });
}

fn run_watcher(
    status_handle: ServiceStatusHandle,
    stop_rx: tokio::sync::oneshot::Receiver<()>,
) -> Result<(), AppError> {
    // 設定を読む前に StartPending を報告する。ここで Running と言ってしまうと、
    // 設定エラーで死んでも SCM には「起動成功 → 勝手に停止」に見えてしまい、
    // sc start も 0 を返してしまう（＝失敗に気づけない）。
    set_start_pending(&status_handle, 1);

    let args = match parse_service_args() {
        Ok(args) => args,
        Err(e) => {
            write_startup_error_log(None, &e);
            return Err(e);
        }
    };

    // Running を報告できたかどうか。起動途中で失敗した場合だけ、固定パスへ
    // エラーを残す（起動後の実行時エラーは通常のシステムログに出る）。
    let started = AtomicBool::new(false);
    let result = run_watcher_inner(&status_handle, stop_rx, &args, &started);
    if let Err(e) = &result {
        if !started.load(Ordering::SeqCst) {
            write_startup_error_log(Some(&args), e);
        }
    }
    result
}

fn run_watcher_inner(
    status_handle: &ServiceStatusHandle,
    stop_rx: tokio::sync::oneshot::Receiver<()>,
    args: &ServiceArgs,
    started: &AtomicBool,
) -> Result<(), AppError> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| AppError::Config(format!("tokioランタイム作成失敗: {e}")))?;

    rt.block_on(async {
        set_start_pending(status_handle, 2);

        let global_config = config::load_global_config(&args.global)?;
        let mut rules_conf = config::load_rules_config(&args.rules)?;
        config::apply_global_defaults(&global_config, &mut rules_conf);

        config::validate_global_config(&global_config)?;
        config::validate_rules_config(&rules_conf)?;

        // サービスモードではコンソール出力を無効化する（allow_console=false）
        let (log, log_handle) = Logger::new_system(&global_config.system_log, false)?;
        let log = Arc::new(log);

        // 設定が読めてログも開けた。ここで初めて Running を報告する。
        // これより前に失敗した場合は StartPending のまま Stopped へ落ちるので、
        // SCM が起動失敗として扱い、sc start も非ゼロを返す。
        status_handle
            .set_service_status(ServiceStatus {
                service_type: ServiceType::OWN_PROCESS,
                current_state: ServiceState::Running,
                controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
                exit_code: ServiceExitCode::Win32(0),
                checkpoint: 0,
                wait_hint: Duration::default(),
                process_id: None,
            })
            .map_err(|e| AppError::Config(format!("サービス状態設定失敗: {e}")))?;
        started.store(true, Ordering::SeqCst);

        // 実行アカウントを最初に出す。ネットワーク共有が見えない／外部プロセスが
        // SYSTEM で動く、といった相談はここを確認するのが出発点になる。
        log.info(format!(
            "Windowsサービスとして起動しました  実行アカウント={}",
            crate::platform::current_account()
        ));

        // サービスは既定で SYSTEM 権限で動くため、設定が有効なら外部プロセス
        // （command / execute）をアクティブなログオンユーザー権限で起動する。
        // ログオンユーザーがいなければサービスアカウント権限へフォールバックする。
        let run_as_user = global_config.run_as_logged_in_user();
        crate::platform::win_runas::set_enabled(run_as_user);
        if run_as_user {
            log.info(
                "外部プロセスはアクティブなログオンユーザー権限で実行します（ログオンユーザー不在時はサービス権限）".to_string(),
            );
        } else {
            log.info("外部プロセスはサービスアカウント権限で実行します".to_string());
        }

        // ダッシュボード（既定で同梱）。サービス常駐でもブラウザから閲覧できるよう、
        // CLI 起動と同じ共通入口でローカル HTTP/SSE サーバを起動する。
        #[cfg(feature = "dashboard")]
        crate::dashboard::start(&global_config, &rules_conf.rules, Arc::clone(&log));

        let result = tokio::select! {
            result = watcher::start_watching(&rules_conf.rules, &global_config, Arc::clone(&log)) => result,
            _ = stop_rx => {
                let _ = status_handle.set_service_status(ServiceStatus {
                    service_type: ServiceType::OWN_PROCESS,
                    current_state: ServiceState::StopPending,
                    controls_accepted: ServiceControlAccept::empty(),
                    exit_code: ServiceExitCode::Win32(0),
                    checkpoint: 0,
                    wait_hint: Duration::from_secs(5),
                    process_id: None,
                });
                log.info("サービス停止シグナルを受信しました".to_string());
                Ok(())
            }
        };

        log.shutdown();
        let _ = log_handle.await;

        result
    })
}

struct ServiceArgs {
    global: PathBuf,
    rules: PathBuf,
}

/// SCM が binPath= で指定したコマンドラインから --global / --rules を取り出す。
fn parse_service_args() -> Result<ServiceArgs, AppError> {
    let args: Vec<OsString> = std::env::args_os().collect();

    let mut global = None;
    let mut rules = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].to_str() {
            Some("--global") | Some("-g") => {
                i += 1;
                if i < args.len() {
                    global = Some(PathBuf::from(&args[i]));
                } else {
                    return Err(AppError::Config("--global の値が未指定です".to_string()));
                }
            }
            Some("--rules") | Some("-r") => {
                i += 1;
                if i < args.len() {
                    rules = Some(PathBuf::from(&args[i]));
                } else {
                    return Err(AppError::Config("--rules の値が未指定です".to_string()));
                }
            }
            _ => {}
        }
        i += 1;
    }

    // binPath= にオプションを書かずにサービス登録した場合でも、
    // 実行ファイルと同じフォルダの global.toml / rules.toml を拾って起動できる。
    Ok(ServiceArgs {
        global: config::resolve_config_path(global, "global.toml", "--global")?,
        rules: config::resolve_config_path(rules, "rules.toml", "--rules")?,
    })
}

#[cfg(test)]
#[path = "../tests/service.rs"]
mod tests;
