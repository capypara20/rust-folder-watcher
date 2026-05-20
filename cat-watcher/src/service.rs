#![cfg(windows)]

use std::ffi::OsString;
use std::path::PathBuf;
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
    service_dispatcher::start(SERVICE_NAME, ffi_service_main).is_ok()
}

fn service_main(_arguments: Vec<OsString>) {
    let _ = run_service();
}

fn run_service() -> Result<(), AppError> {
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

    let status_handle = service_control_handler::register(SERVICE_NAME, event_handler)
        .map_err(|e| AppError::Config(format!("SCM登録失敗: {e}")))?;

    let result = run_watcher(&status_handle, stop_rx);

    let exit_code = if result.is_ok() { 0 } else { 1 };
    let _ = status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(exit_code),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    });

    result
}

fn run_watcher(
    status_handle: &ServiceStatusHandle,
    stop_rx: tokio::sync::oneshot::Receiver<()>,
) -> Result<(), AppError> {
    let args = parse_service_args()?;

    status_handle
        .set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: ServiceState::Running,
            controls_accepted: ServiceControlAccept::STOP,
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        })
        .map_err(|e| AppError::Config(format!("サービス状態設定失敗: {e}")))?;

    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| AppError::Config(format!("tokioランタイム作成失敗: {e}")))?;

    rt.block_on(async {
        let global_config = config::load_global_config(&args.global)?;
        let rules_conf = config::load_rules_config(&args.rules)?;

        config::validate_global_config(&global_config)?;
        config::validate_rules_config(&rules_conf)?;

        // サービスモードではコンソール出力を無効化する
        let mut service_global = global_config.global.clone();
        service_global.log_to_console = false;

        let (log, log_handle) = Logger::new(&service_global)?;
        let log = Arc::new(log);

        log.info("Windowsサービスとして起動しました".to_string());

        tokio::select! {
            result = watcher::start_watching(&rules_conf.rules, &service_global, Arc::clone(&log)) => {
                result?;
            }
            _ = stop_rx => {
                log.info("サービス停止シグナルを受信しました".to_string());
            }
        }

        log.shutdown();
        let _ = log_handle.await;

        Ok::<(), AppError>(())
    })
}

struct ServiceArgs {
    global: PathBuf,
    rules: PathBuf,
}

/// SCM が binPath= で指定したコマンドラインから --global / --rules を取り出す。
fn parse_service_args() -> Result<ServiceArgs, AppError> {
    let args: Vec<String> = std::env::args().collect();

    let mut global = None;
    let mut rules = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--global" | "-g" => {
                i += 1;
                if i < args.len() {
                    global = Some(PathBuf::from(&args[i]));
                }
            }
            "--rules" | "-r" => {
                i += 1;
                if i < args.len() {
                    rules = Some(PathBuf::from(&args[i]));
                }
            }
            _ => {}
        }
        i += 1;
    }

    let global = global.ok_or_else(|| {
        AppError::Config("--global オプションが未指定です".to_string())
    })?;
    let rules = rules.ok_or_else(|| {
        AppError::Config("--rules オプションが未指定です".to_string())
    })?;

    Ok(ServiceArgs { global, rules })
}
