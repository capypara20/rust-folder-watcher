pub mod command;
pub mod common;
pub mod copy;
pub mod execute;
pub mod r#move;
pub(crate) mod spawn;

use std::path::Path;
use std::sync::Arc;

use crate::config::{Action, ActionConfig, RetryConfig};
use crate::error::AppError;
use crate::logger::Logger;
use crate::placeholder::PlaceholderContext;

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

/// 開始ログに出す「何をするか」の 1 行。
///
/// 変換済みの [`Action`] を受けるので、`unwrap_or("")` で
/// 設定漏れを空文字として素通りさせることがない。
fn action_detail(action: &Action) -> String {
    match action {
        // 設定に書かれた文字列をそのまま出すと、他のログ行（OS 由来の区切り）と
        // 表記が食い違う。パスなので区切り文字を揃えてよい。
        Action::Copy(t) | Action::Move(t) => format!(
            "destination={}  overwrite={}",
            crate::path_fmt::normalize(&t.destination),
            t.overwrite
        ),
        Action::Command(c) => format!("shell={}  command={}", c.shell, c.command),
        Action::Execute(e) => {
            // program はパスなので揃える。args は値やスイッチが混ざるので触らない。
            let program = crate::path_fmt::normalize(&e.program);
            format!("{program} {}", e.args.join(" ")).trim_end().to_string()
        }
    }
}

/// 中断によって実行されなかったアクションの一覧を、ログ用の 1 行にまとめる。
///
/// `failed_index` は失敗したアクションの番号（1 始まり）。
/// 残りが無ければ `None`（中断した旨を出す必要がない）。
fn skipped_summary(actions: &[ActionConfig], failed_index: usize) -> Option<String> {
    let remaining = actions.get(failed_index..).unwrap_or(&[]);
    if remaining.is_empty() {
        return None;
    }
    let list = remaining
        .iter()
        .enumerate()
        .map(|(offset, a)| {
            format!(
                "{}.{}",
                failed_index + 1 + offset,
                a.type_.as_str()
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(
        "以降の {} 件を実行せず中断しました: {list}",
        remaining.len()
    ))
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

        // delay_ms が設定されていれば、このアクションの直前で待つ。
        // 書き込みが終わりきらないうちにコピーが走るのを避けたいとき等に使う。
        if let Some(delay) = action.delay_ms.filter(|ms| *ms > 0) {
            sink.note(index, total, format!("delay_ms={delay} のため待機します"));
            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
        }

        // 種類ごとに必須項目を揃えた形へ変換する。起動時のバリデーションを
        // 通っていれば必ず成功するので、ここが失敗するのは設定の読み込み経路の
        // バグ。黙って空文字で動かさず、そのアクションを失敗させる。
        let validated = match Action::try_from(action) {
            Ok(v) => v,
            Err(missing) => {
                let e = AppError::from(missing);
                sink.err(index, total, format!("{e}"));
                if let Some(msg) = skipped_summary(actions, index) {
                    sink.warn(index, total, msg);
                }
                return Err(e);
            }
        };

        let detail = action_detail(&validated);
        sink.action_start(index, total, action.type_.as_str(), detail);

        let result: Result<Option<std::path::PathBuf>, AppError> = match &validated {
            Action::Copy(t) => copy::execute(t, src, &ctx, retry, &sink, step).await,
            Action::Move(t) => r#move::execute(t, src, &ctx, retry, &sink, step).await,
            Action::Command(c) => command::execute(c, &ctx, &sink, step).await.map(|_| None),
            Action::Execute(e) => execute::execute(e, &ctx, &sink, step).await.map(|_| None),
        };

        match result {
            Ok(Some(dest_file)) => {
                ctx.destination = dest_file.to_string_lossy().replace('\\', "/");
            }
            Ok(None) => {}
            Err(e) => {
                // アクション失敗は action ログ＋ターミナルに記録（システムログには残さない）
                sink.err(index, total, format!("{e}"));
                // ここで打ち切るので残りのアクションは実行されない。何も出さないと
                // 「ヘッダに actions=10 と書いてあるのにログが 1 件しか無い」状態になり、
                // 2 件目以降が動いていないことに気づけない。
                if let Some(msg) = skipped_summary(actions, index) {
                    sink.warn(index, total, msg);
                }
                return Err(e);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "../tests/actions_chain.rs"]
mod tests;
