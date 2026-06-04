use std::path::PathBuf;
use std::sync::Arc;

use chrono::Local;
use clap::Parser;
use colored::Colorize;

use crate::error::AppError;

mod actions;
mod config;
mod csv_import;
mod error;
mod logger;
mod placeholder;
mod router;
#[cfg(windows)]
mod service;
mod templates;
mod watcher;

const AFTER_LONG_HELP: &str = "\
\x1b[33;1m▶ 使い方\x1b[0m
  監視の起動:
    cat-watcher -g global.toml -r rules.toml
    cat-watcher -g global.toml -r rules.toml --validate

  テンプレート生成（複数同時指定可）:
    cat-watcher --init global rules        # global.toml と rules.toml を同時生成
    cat-watcher --init global rules csv

  CSV から rules.toml を生成:
    cat-watcher --from-csv rules.csv --output rules.toml

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

\x1b[33;1m▶ CSV インポート列順\x1b[0m  \x1b[2m（--from-csv）\x1b[0m
  rule_name, enabled, watch_path, recursive, target, include_hidden,
  patterns, regex, exclude_patterns, events,
  action_type, destination, overwrite, preserve_structure, verify_integrity,
  shell, command, program, args, working_dir, message,
  exclude_regex, dir_patterns, dir_regex, exclude_dir_patterns, exclude_dir_regex

  複数アクションのルール: rule_name を同じにして行を追加
  複数値フィールド（patterns / events / args 等）: | で区切る  例: create|modify
  ※ 列22以降（exclude_regex〜）は省略可（既存 CSV との後方互換のため末尾）
";

#[derive(clap::ValueEnum, Clone)]
enum InitType {
    /// global.toml のテンプレートを出力
    Global,
    /// rules.toml のテンプレートを出力
    Rules,
    /// rules.csv のテンプレートを出力
    Csv,
}

/// ファイル監視・自動処理ツール
#[derive(Parser)]
#[command(
    arg_required_else_help = true,
    after_long_help = AFTER_LONG_HELP,
)]
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
    /// CSV ファイルから rules.toml を生成
    #[arg(long, value_name = "CSV")]
    from_csv: Option<PathBuf>,
    /// 出力先ファイルパス（--from-csv または --init と組み合わせて使用）
    #[arg(long, value_name = "FILE")]
    output: Option<PathBuf>,
    /// テンプレートファイルを出力する（複数指定可: global rules csv）
    #[arg(long, value_name = "TYPE", num_args = 1..)]
    init: Vec<InitType>,
}

fn main() {
    // StartServiceCtrlDispatcherW は tokio ランタイム生成より先に
    // メインスレッドから直接呼ぶ必要がある（エラー1053回避）
    #[cfg(windows)]
    if service::try_run_as_service() {
        return;
    }

    let args = Args::parse();

    if let Some(ref csv_path) = args.from_csv {
        exit_on_err(csv_import::run(csv_path, args.output.as_deref()));
        return;
    }

    if !args.init.is_empty() {
        if args.init.len() > 1 && args.output.is_some() {
            let ts = Local::now().format("%Y-%m-%d %H:%M:%S");
            eprintln!("{}", format!("[{ts}] [ERROR] --init を複数指定する場合は --output を同時に使用できません").red().bold());
            std::process::exit(1);
        }
        let mut had_error = false;
        for init_type in &args.init {
            if let Err(e) = run_init(init_type, args.output.as_deref()) {
                let ts = Local::now().format("%Y-%m-%d %H:%M:%S");
                eprintln!("{}", format!("[{ts}] [ERROR] {e}").red().bold());
                had_error = true;
            }
        }
        if had_error {
            std::process::exit(1);
        }
        return;
    }

    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            let ts = Local::now().format("%Y-%m-%d %H:%M:%S");
            eprintln!("{}", format!("[{ts}] [ERROR] tokioランタイム作成失敗: {e}").red().bold());
            std::process::exit(1);
        }
    };

    match rt.block_on(run(&args)) {
        Ok(_) => std::process::exit(0),
        Err(e) => {
            let ts = Local::now().format("%Y-%m-%d %H:%M:%S");
            eprintln!("{}", format!("[{ts}] [ERROR] 実行エラー: {e}").red().bold());
            std::process::exit(1);
        }
    };
}

async fn run(cli: &Args) -> Result<(), AppError> {
    let global_path = cli.global.as_ref().ok_or_else(|| {
        AppError::Config("--global オプションが未指定です".to_string())
    })?;
    let rules_path = cli.rules.as_ref().ok_or_else(|| {
        AppError::Config("--rules オプションが未指定です".to_string())
    })?;

    let global_config = config::load_global_config(global_path)?;
    let rules_conf = config::load_rules_config(rules_path)?;

    config::validate_global_config(&global_config)?;
    config::validate_rules_config(&rules_conf)?;

    if cli.validate {
        let ts = Local::now().format("%Y-%m-%d %H:%M:%S");
        println!("{}", format!("[{ts}] [INFO]    バリデーション処理成功").cyan());
        return Ok(());
    }

    let (log, log_handle) = logger::Logger::new(&global_config.global)?;
    let log = Arc::new(log);

    log.info(format!(
        "cat-watcher 起動  global={} rules={}",
        global_path.display(),
        rules_path.display()
    ));

    watcher::start_watching(&rules_conf.rules, &global_config.global, Arc::clone(&log)).await?;

    log.shutdown();
    let _ = log_handle.await;
    Ok(())
}

fn exit_on_err(result: Result<(), AppError>) {
    if let Err(e) = result {
        let ts = Local::now().format("%Y-%m-%d %H:%M:%S");
        eprintln!("{}", format!("[{ts}] [ERROR] {e}").red().bold());
        std::process::exit(1);
    }
}

fn run_init(init_type: &InitType, output: Option<&std::path::Path>) -> Result<(), AppError> {
    let (content, default_name) = match init_type {
        InitType::Global => (templates::GLOBAL_TOML, "global.toml"),
        InitType::Rules  => (templates::RULES_TOML,  "rules.toml"),
        InitType::Csv    => (templates::RULES_CSV,    "rules.csv"),
    };

    if let Some(path) = output {
        std::fs::write(path, content)
            .map_err(|e| AppError::Config(format!("ファイルの書き込みに失敗: {e}")))?;
        let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        println!("{}", format!("[{ts}] [INFO]    テンプレートを出力しました: {}", path.display()).cyan());
    } else {
        let path = std::path::Path::new(default_name);
        if path.exists() {
            return Err(AppError::Config(format!(
                "{default_name} が既に存在します。上書きする場合は --output で明示的にパスを指定してください"
            )));
        }
        std::fs::write(path, content)
            .map_err(|e| AppError::Config(format!("ファイルの書き込みに失敗: {e}")))?;
        let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        println!("{}", format!("[{ts}] [INFO]    テンプレートを出力しました: {default_name}").cyan());
    }
    Ok(())
}
