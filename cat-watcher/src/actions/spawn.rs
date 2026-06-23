use std::process::Stdio;

/// 外部プロセスを fire-and-forget で起動する共通ヘルパ。
///
/// 通常は OS 既定の権限で起動する（CLI 実行ならログオンユーザー、サービス実行
/// ならサービスアカウント）。Windows のサービスモードかつ `run_as_logged_in_user`
/// が有効な場合のみ、アクティブなログオンユーザーの権限での起動を試み、
/// ログオンユーザーがいなければサービスアカウント権限へフォールバックする。
///
/// 起動失敗時はエラー内容の文字列を返す。呼び出し側でアクション種別ごとの
/// メッセージへ整形する。
pub fn spawn_detached(
    program: &str,
    args: &[String],
    working_dir: Option<&str>,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        use crate::win_runas::{self, RunAsResult};
        if win_runas::enabled() {
            match win_runas::spawn_as_active_user(program, args, working_dir) {
                RunAsResult::Spawned => return Ok(()),
                // ログオンユーザー不在: サービスアカウント権限での起動へフォールバック
                RunAsResult::NoActiveUser => {}
                RunAsResult::Err(e) => return Err(e),
            }
        }
    }

    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args).stdout(Stdio::null()).stderr(Stdio::null());

    if let Some(dir) = working_dir.filter(|s| !s.is_empty()) {
        cmd.current_dir(dir);
    }

    cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
}
