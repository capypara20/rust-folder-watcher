use std::process::Stdio;

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
    args: &[SpawnArg],
    working_dir: Option<&str>,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        use crate::platform::win_runas::{self, RunAsResult};
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

    cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
}
