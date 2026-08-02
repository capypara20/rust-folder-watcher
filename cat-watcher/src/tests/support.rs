//! アクション系テストの共通ヘルパー。

use std::path::Path;
use std::sync::Arc;

use tempfile::tempdir;

use crate::actions::ActionSink;
use crate::config::{ActionConfig, ActionType, LogRotation, RetryConfig};
use crate::logger::Logger;

/// 全フィールド None の ActionConfig を返す。各テストで必要な項目だけ埋める。
pub(crate) fn base_action(type_: ActionType) -> ActionConfig {
    ActionConfig {
        type_,
        destination: None,
        overwrite: None,
        verify_integrity: None,
        preserve_structure: None,
        working_dir: None,
        shell: None,
        command: None,
        program: None,
        args: None,
        auto_create: None,
        delay_ms: None,
    }
}

pub(crate) fn make_retry(count: u32) -> RetryConfig {
    RetryConfig { count, interval_ms: 10 }
}

/// 親ディレクトリを作りつつファイルを書き込む。
pub(crate) fn write_file(path: &Path, body: &[u8]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, body).unwrap();
}

/// 一時ディレクトリにログを書き捨てる ActionSink を作る。
pub(crate) fn make_sink() -> ActionSink {
    let dir = tempdir().unwrap();
    let (logger, _) = Logger::for_action(
        dir.path().to_str().unwrap().to_string(),
        "test.log".to_string(),
        LogRotation::Never,
    )
    .unwrap();
    std::mem::forget(dir);
    ActionSink::new(Arc::new(logger), None)
}
