pub mod command;
pub mod common;
pub mod copy;
pub mod execute;
pub mod r#move;

#[cfg(test)]
pub(crate) mod test_support;

use std::path::Path;
use std::sync::Arc;

use crate::config::{ActionConfig, ActionType, RetryConfig};
use crate::error::AppError;
use crate::logger::Logger;
use crate::placeholder::{expand_placeholders, PlaceholderContext};

/// アクション結果（開始・成功・失敗・警告・補足）を
/// system ロガー（ターミナル表示用）と action ロガー（ファイル記録用）の
/// 両系統へ送るファンアウト。system 側はコンソールに出るがシステムログ
/// ファイルには書かれない（writer_task の System 分岐が Action 系を捨てる）。
pub struct ActionSink {
    system: Arc<Logger>,
    action: Option<Arc<Logger>>,
}

impl ActionSink {
    pub fn new(system: Arc<Logger>, action: Option<Arc<Logger>>) -> Self {
        Self { system, action }
    }

    /// ステップ開始行（[ACTION] / "N. copy"）。
    pub fn action_start(&self, index: usize, total: usize, action_type: &str, detail: String) {
        self.system.log_action(index, total, action_type, detail.clone());
        if let Some(a) = &self.action {
            a.log_action(index, total, action_type, detail);
        }
    }

    /// ステップ成功（[OK] / "N. OK"）。
    pub fn ok(&self, index: usize, total: usize, msg: String) {
        self.system.log_action_ok(index, total, msg.clone());
        if let Some(a) = &self.action {
            a.log_action_ok(index, total, msg);
        }
    }

    /// ステップ失敗（[ERROR] / "N. ERR"）。
    pub fn err(&self, index: usize, total: usize, msg: String) {
        self.system.log_action_err(index, total, msg.clone());
        if let Some(a) = &self.action {
            a.log_action_err(index, total, msg);
        }
    }

    /// ステップ警告（スキップ・リトライ等。[WARN] / "N. WARN"）。
    pub fn warn(&self, index: usize, total: usize, msg: String) {
        self.system.log_action_warn(index, total, msg.clone());
        if let Some(a) = &self.action {
            a.log_action_warn(index, total, msg);
        }
    }

    /// 補足情報（別ボリュームへの copy+delete 等。[INFO] / "N. --"）。
    pub fn note(&self, index: usize, total: usize, msg: String) {
        self.system.log_action_note(index, total, msg.clone());
        if let Some(a) = &self.action {
            a.log_action_note(index, total, msg);
        }
    }
}

/// 1 つの監視イベントに対して、ルールの actions を順に実行する。
/// アクション間で PlaceholderContext を保持し、copy/move 完了後に {Destination} を更新する。
///
/// `log` は system ロガー（ターミナル表示）、`action_log` はルール別アクションログ
/// （ファイル）。アクション失敗エラーは両系統へ送るが、システムログ**ファイル**には
/// 残らない（アクション失敗は action ログにのみ記録する方針）。
pub async fn execute_chain(
    actions: &[ActionConfig],
    src: &Path,
    watch_path: &Path,
    retry: &RetryConfig,
    log: Arc<Logger>,
    action_log: Option<Arc<Logger>>,
) -> Result<(), AppError> {
    let sink = ActionSink::new(log, action_log);
    let mut ctx = PlaceholderContext::new(src, watch_path, "");
    let total = actions.len();

    for (i, action) in actions.iter().enumerate() {
        let index = i + 1;
        let step = (index, total);

        let result: Result<Option<std::path::PathBuf>, AppError> = match action.type_ {
            ActionType::Copy => {
                let dest_str = action.destination.as_deref().unwrap_or("");
                let overwrite = action.overwrite.unwrap_or(false);
                let detail = format!("destination={dest_str}  overwrite={overwrite}");
                sink.action_start(index, total, "copy", detail);
                copy::execute(action, src, &ctx, retry, &sink, step).await
            }
            ActionType::Move => {
                let dest_str = action.destination.as_deref().unwrap_or("");
                let overwrite = action.overwrite.unwrap_or(false);
                let detail = format!("destination={dest_str}  overwrite={overwrite}");
                sink.action_start(index, total, "move", detail);
                r#move::execute(action, src, &ctx, retry, &sink, step).await
            }
            ActionType::Command => {
                let shell = action.shell.as_deref().unwrap_or("");
                let cmd = action.command.as_deref().unwrap_or("");
                let detail = format!("shell={shell}  command={cmd}");
                sink.action_start(index, total, "command", detail);
                command::execute(action, &ctx, &sink, step).await.map(|_| None)
            }
            ActionType::Execute => {
                let program = action.program.as_deref().unwrap_or("");
                let args = action.args.as_deref().unwrap_or(&[]);
                let args_str = args.join(" ");
                let detail = format!("{program} {args_str}").trim_end().to_string();
                sink.action_start(index, total, "execute", detail);
                execute::execute(action, &ctx, &sink, step).await.map(|_| None)
            }
            ActionType::Log => {
                let raw = action.message.as_deref().unwrap_or("");
                sink.action_start(index, total, "log", String::new());
                match expand_placeholders(raw, &ctx) {
                    Ok(msg) => {
                        sink.ok(index, total, msg);
                        Ok(None)
                    }
                    Err(e) => Err(e),
                }
            }
        };

        match result {
            Ok(Some(dest_file)) => {
                ctx.destination = dest_file.to_string_lossy().replace('\\', "/");
            }
            Ok(None) => {}
            Err(e) => {
                // アクション失敗は action ログ＋ターミナルに記録（システムログには残さない）
                sink.err(index, total, format!("{e}"));
                return Err(e);
            }
        }
    }
    Ok(())
}
