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
mod templates;
mod watcher;

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
#[command(disable_help_flag = false)]
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
    if std::env::args_os().len() == 1 {
        init_colors();
        print_guide();
        return;
    }

    let args = Args::parse();

    if let Some(ref csv_path) = args.from_csv {
        exit_on_err(csv_import::run(csv_path, args.output.as_deref()));
        return;
    }

    if !args.init.is_empty() {
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

    let result = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokioランタイム作成失敗")
        .block_on(run(&args));
    match result {
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

fn init_colors() {
    #[cfg(windows)]
    colored::control::set_virtual_terminal(true).ok();
    colored::control::set_override(true);
}

fn print_guide() {
    let sep = "━".repeat(58);
    println!("{}", sep.bright_cyan());
    println!("  {}  ファイル監視・自動処理ツール", "cat-watcher".bright_white().bold());
    println!("{}", sep.bright_cyan());

    println!("\n{}", "▶ 使い方".bright_yellow().bold());
    println!("  cat-watcher.exe --global global.toml --rules rules.toml");
    println!("  cat-watcher.exe --global global.toml --rules rules.toml --validate");
    println!("  cat-watcher.exe --from-csv rules.csv [--output rules.toml]");

    println!("\n{}", "▶ global.toml（グローバル設定）".bright_yellow().bold());
    println!("{}", r#"
[global]
log_level         = "info"    # trace / debug / info / warn / error
log_dir           = "C:\logs"
log_file_name     = "cat-watcher_{Date}.log"  # {Date} or {DateTime}
log_rotation      = "daily"                   # daily / never
retry_count       = 3
retry_interval_ms = 1000"#.trim_start_matches('\n'));

    println!("\n{}", "▶ rules.toml（ルール設定）".bright_yellow().bold());
    println!("{}", r#"
[[rules]]
enabled = true
name    = "ルール名"

[rules.watch]
path             = "C:\監視フォルダ"
recursive        = true          # サブフォルダも対象にするか
target           = "file"        # file / directory / both
include_hidden   = false
patterns         = ["*.pdf", "*.docx"]  # glob（regex と排他）
# regex          = ".*\\.pdf$"          # 正規表現（patterns と排他）
exclude_patterns = []
events           = ["create", "modify"] # create / modify / delete / rename

[[rules.actions]]               # ─── copy ───────────────────────────
type               = "copy"
destination        = "D:\backup\{Date}"
overwrite          = true
preserve_structure = false
verify_integrity   = true       # BLAKE3 ハッシュで整合性検証

# [[rules.actions]]             # ─── move ───────────────────────────
# type        = "move"
# destination = "D:\archive\{Date}\{Time}"
# overwrite   = false

# [[rules.actions]]             # ─── command ────────────────────────
# type        = "command"
# shell       = "cmd"           # cmd / powershell / pwsh
# command     = "echo {FullName}"
# working_dir = ""

# [[rules.actions]]             # ─── execute ────────────────────────
# type        = "execute"
# program     = "C:\tool\app.exe"
# args        = ["{FullName}"]
# working_dir = """#.trim_start_matches('\n'));

    println!("\n{}", "▶ プレースホルダー一覧".bright_yellow().bold());
    let placeholders = [
        ("{FullName}",      "ファイルのフルパス"),
        ("{Name}",          "ファイル名（拡張子なし）"),
        ("{BaseName}",      "ファイル名（拡張子あり）"),
        ("{Extension}",     "拡張子"),
        ("{DirectoryName}", "親ディレクトリのフルパス"),
        ("{WatchPath}",     "監視ルートパス"),
        ("{RelativePath}",  "監視ルートからの相対パス"),
        ("{Date}",          "検知日       例: 20240302"),
        ("{Time}",          "検知時刻     例: 103020"),
        ("{DateTime}",      "日時         例: 20240302_103020"),
        ("{Destination}",   "直前のアクションの出力先（連鎖用）"),
    ];
    for (key, desc) in placeholders {
        println!("  {:<18} {}", key.bright_green(), desc);
    }

    println!("\n{}", "▶ CSV インポート（--from-csv）".bright_yellow().bold());
    println!("  CSV の列順（1行目はヘッダー行として自動スキップ）:");
    println!("  {}", "rule_name, enabled, watch_path, recursive, target, include_hidden,".dimmed());
    println!("  {}", "patterns, regex, exclude_patterns, events,".dimmed());
    println!("  {}", "action_type, destination, overwrite, preserve_structure, verify_integrity,".dimmed());
    println!("  {}", "shell, command, program, args, working_dir".dimmed());
    println!("  複数アクションのルール: rule_name を同じにして行を追加");
    println!("  複数値フィールド（patterns / events / args 等）: | で区切る  例: create|modify");

    println!("\n{}", sep.bright_cyan());
}
