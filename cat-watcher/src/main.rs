use std::path::PathBuf;
use std::sync::Arc;

use chrono::Local;
use clap::{CommandFactory, Parser};
use colored::Colorize;

use crate::error::AppError;

mod actions;
mod config;
#[cfg(feature = "dashboard")]
mod dashboard;
mod error;
mod exe_path;
mod logger;
#[cfg(windows)]
mod platform;
mod path_fmt;
mod placeholder;
mod router;
mod templates;
mod watcher;

// テスト共通ヘルパー（テスト本体は src/tests/ に集約）。
#[cfg(test)]
#[path = "tests/support.rs"]
mod test_support;

const AFTER_LONG_HELP: &str = "\
\x1b[33;1m▶ 使い方\x1b[0m
  監視の起動:
    cat-watcher -g global.toml -r rules.toml
    cat-watcher -g global.toml -r rules.toml --validate
    cat-watcher                            # ↓ 既定名を自動で探して起動

  \x1b[2m-g / -r を省略すると global.toml / rules.toml を
   「カレントディレクトリ → 実行ファイルと同じフォルダ」の順に探します。\x1b[0m

  テンプレート生成（複数同時指定可）:
    cat-watcher --init global rules        # global.toml と rules.toml を同時生成

\x1b[33;1m▶ プレースホルダー\x1b[0m  \x1b[2m（rules.toml の destination / command / args などで使用可）\x1b[0m
  \x1b[32m{FullName}\x1b[0m         ファイルのフルパス
  \x1b[32m{Name}\x1b[0m             ファイル名（拡張子なし）
  \x1b[32m{BaseName}\x1b[0m         ファイル名（拡張子あり）
  \x1b[32m{Extension}\x1b[0m        拡張子
  \x1b[32m{DirectoryName}\x1b[0m    親ディレクトリのフルパス
  \x1b[32m{WatchPath}\x1b[0m        監視ルートパス
  \x1b[32m{RelativePath}\x1b[0m     監視ルートからの相対パス
  \x1b[32m{Date}\x1b[0m             検知日       例: 20240302
  \x1b[32m{Time}\x1b[0m             検知時刻     例: 103020
  \x1b[32m{DateTime}\x1b[0m         日時         例: 20240302_103020
  \x1b[32m{Destination}\x1b[0m      直前のアクションの出力先（連鎖用）
";

#[derive(clap::ValueEnum, Clone)]
enum InitType {
    /// global.toml のテンプレートを出力
    Global,
    /// rules.toml のテンプレートを出力
    Rules,
}

/// ファイル監視・自動処理ツール
#[derive(Parser)]
#[command(after_long_help = AFTER_LONG_HELP)]
struct Args {
    /// グローバル設定ファイルのパス
    #[arg(short, long, value_name = "FILE")]
    global: Option<PathBuf>,
    /// ルール設定ファイルのパス
    #[arg(short, long, value_name = "FILE")]
    rules: Option<PathBuf>,
    /// 設定ファイルのバリデーションのみ実行して終了
    #[arg(long)]
    validate: bool,
    /// 出力先ファイルパス（--init と組み合わせて使用）
    #[arg(long, value_name = "FILE")]
    output: Option<PathBuf>,
    /// テンプレートファイルを出力する（複数指定可: global rules）
    #[arg(long, value_name = "TYPE", num_args = 1..)]
    init: Vec<InitType>,
}

fn main() {
    // StartServiceCtrlDispatcherW は tokio ランタイム生成より先に
    // メインスレッドから直接呼ぶ必要がある（エラー1053回避）
    #[cfg(windows)]
    if platform::service::try_run_as_service() {
        return;
    }

    let args = Args::parse();

    // 引数なしで起動された場合、設定ファイルが既定の場所に無ければヘルプを出す。
    // 逆に exe と同じフォルダに global.toml / rules.toml を置いてあれば、
    // ダブルクリックやオプションなしのサービス登録でもそのまま監視を始められる。
    if std::env::args_os().len() <= 1
        && (config::find_config_file("global.toml").is_none()
            || config::find_config_file("rules.toml").is_none())
    {
        let _ = Args::command().print_long_help();
        println!();
        std::process::exit(2);
    }

    if !args.init.is_empty() {
        if args.init.len() > 1 && args.output.is_some() {
            exit_with(&AppError::Usage(
                "--init を複数指定する場合は --output を同時に使用できません".to_string(),
            ));
        }
        let mut first_error = None;
        for init_type in &args.init {
            if let Err(e) = run_init(init_type, args.output.as_deref()) {
                print_error(&e);
                first_error.get_or_insert(e);
            }
        }
        if let Some(e) = first_error {
            std::process::exit(e.exit_code());
        }
        return;
    }

    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => exit_with(&AppError::Runtime(format!("非同期ランタイムを作成できません: {e}"))),
    };

    match rt.block_on(run(&args)) {
        Ok(_) => std::process::exit(0),
        Err(e) => exit_with(&e),
    };
}

/// エラーを表示する。
///
/// 前置きは `[ERROR]` だけにする。「実行エラー:」のような前置きを足すと、
/// エラー自身の文面と重なって読みにくくなる（以前は 3 重になっていた）。
fn print_error(e: &AppError) {
    let ts = Local::now().format("%Y-%m-%d %H:%M:%S");
    eprintln!("{}", format!("[{ts}] [ERROR] {e}").red().bold());
}

/// エラーを表示し、その種類に応じた終了コードで終わる。
fn exit_with(e: &AppError) -> ! {
    print_error(e);
    std::process::exit(e.exit_code());
}

async fn run(cli: &Args) -> Result<(), AppError> {
    // 明示指定が無ければ、カレントディレクトリ／実行ファイル横の既定名を探す。
    let global_path = config::resolve_config_path(cli.global.clone(), "global.toml", "--global")?;
    let rules_path = config::resolve_config_path(cli.rules.clone(), "rules.toml", "--rules")?;

    let (global_config, rules_conf) = config::load(&global_path, &rules_path)?;

    if cli.validate {
        let ts = Local::now().format("%Y-%m-%d %H:%M:%S");
        println!("{}", format!("[{ts}] [INFO]    バリデーション処理成功").cyan());
        return Ok(());
    }

    #[cfg(windows)]
    if !global_config.system_log.console {
        hide_console_window();
    }

    let (log, log_handle) = logger::Logger::new_system(&global_config.system_log, true)?;
    let log = Arc::new(log);

    log.info(format!(
        "cat-watcher 起動  global={} rules={}",
        global_path.display(),
        rules_path.display()
    ));
    // 実行アカウントを出しておく。ネットワーク共有（UNC）が見えるかどうかは
    // このアカウントの権限で決まるため、切り分けの出発点になる。
    #[cfg(windows)]
    log.info(format!("実行アカウント={}", platform::current_account()));

    // ダッシュボード（既定で同梱・設定 enabled=true のときだけ起動）。
    // 監視を開始する前にハブを初期化し、HTTP/SSE サーバを別タスクで起動する。
    #[cfg(feature = "dashboard")]
    dashboard::start(&global_config, &rules_conf.rules, Arc::clone(&log));

    let result =
        watcher::start_watching(&rules_conf.rules, &global_config, Arc::clone(&log)).await;
    if let Err(e) = &result {
        // 監視処理の異常終了はシステム階層のエラーとしてシステムログに残す。
        log.error(format!("監視処理が異常終了しました: {e}"));
    }

    log.shutdown();
    let _ = log_handle.await;
    result
}

#[cfg(windows)]
fn hide_console_window() {
    use windows_sys::Win32::System::Console::GetConsoleWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
    unsafe {
        let hwnd = GetConsoleWindow();
        if !hwnd.is_null() {
            ShowWindow(hwnd, SW_HIDE);
        }
    }
}

fn run_init(init_type: &InitType, output: Option<&std::path::Path>) -> Result<(), AppError> {
    let (content, default_name) = match init_type {
        InitType::Global => (templates::GLOBAL_TOML, "global.toml"),
        InitType::Rules  => (templates::RULES_TOML,  "rules.toml"),
    };

    if let Some(path) = output {
        std::fs::write(path, content).map_err(|source| AppError::TemplateWrite {
            path: path.to_path_buf(),
            source,
        })?;
        let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        println!("{}", format!("[{ts}] [INFO]    テンプレートを出力しました: {}", path.display()).cyan());
    } else {
        let path = std::path::Path::new(default_name);
        if path.exists() {
            return Err(AppError::Usage(format!(
                "{default_name} が既に存在します。上書きする場合は --output で明示的にパスを指定してください"
            )));
        }
        std::fs::write(path, content).map_err(|source| AppError::TemplateWrite {
            path: path.to_path_buf(),
            source,
        })?;
        let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        println!("{}", format!("[{ts}] [INFO]    テンプレートを出力しました: {default_name}").cyan());
    }
    Ok(())
}
