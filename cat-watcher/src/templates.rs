pub const GLOBAL_TOML: &str = r#"[global]
log_level         = "info"    # trace / debug / info / warn / error
log_dir           = "C:\logs"
log_file_name     = "cat-watcher_{Date}.log"  # {Date} / {DateTime}
log_rotation      = "daily"                   # daily / never
retry_count       = 3
retry_interval_ms = 1000
"#;

pub const RULES_TOML: &str = r#"[[rules]]
enabled = true
name    = "ルール名"

[rules.watch]
path             = "C:\監視フォルダ"
recursive        = true
target           = "file"          # file / directory / both
include_hidden   = false
patterns         = ["*"]           # glob（regex と排他）
# regex          = ".*\\.csv$"     # 正規表現（patterns と排他）
exclude_patterns = []
events           = ["create"]      # create / modify / delete / rename

# ─── アクション例（使うものだけコメント解除してください） ────────────────

[[rules.actions]]                  # ─── log（ログ出力のみ） ─────────────
type    = "log"
message = "検知: {BaseName}"

# [[rules.actions]]                # ─── copy ──────────────────────────────
# type               = "copy"
# destination        = "D:\backup\{Date}"
# overwrite          = false
# preserve_structure = false
# verify_integrity   = true

# [[rules.actions]]                # ─── move ──────────────────────────────
# type               = "move"
# destination        = "D:\archive\{Date}"
# overwrite          = false
# preserve_structure = false
# verify_integrity   = false

# [[rules.actions]]                # ─── command ────────────────────────────
# type        = "command"
# shell       = "cmd"              # cmd / powershell / pwsh
# command     = "echo {FullName}"
# working_dir = ""

# [[rules.actions]]                # ─── execute ────────────────────────────
# type        = "execute"
# program     = "C:\tool\app.exe"
# args        = ["{FullName}"]
# working_dir = ""
"#;

pub const RULES_CSV: &str = "\
rule_name,enabled,watch_path,recursive,target,include_hidden,patterns,regex,exclude_patterns,events,action_type,destination,overwrite,preserve_structure,verify_integrity,shell,command,program,args,working_dir,message\r\n\
ルール名,true,C:\\監視フォルダ,true,file,false,*.csv,,,create,log,,,,,,,,,,検知: {BaseName}\r\n\
";
