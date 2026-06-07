pub const GLOBAL_TOML: &str = r#"# ─── リトライ設定（copy / move 失敗時の再試行）─────────────────────────
[retry]
count       = 3       # 再試行回数
interval_ms = 1000    # 再試行間隔（ミリ秒）

# ─── システムログ（プログラム全体の起動日誌・全体で1本）───────────────
# 起動バナー / ルール一覧 / 監視開始 / 終了 と、システム階層のエラーを記録。
# 検知・アクションの結果はルール別ログ（rules.toml の [rules.log]）に出ます。
[system_log]
enabled   = true
dir       = "C:\logs"
file_name = "system_{Date}.log"   # {Date} / {DateTime}
rotation  = "daily"               # daily / never
level     = "info"                # trace / debug / info / warn / error
console   = true                  # コンソールへの出力 ON/OFF
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

# ─── ルール別ログ（検知ログ・アクションログ）────────────────────────────
# detect: 検知イベントだけを記録（timestamp │ events │ 検知パス）
# action: 検知ごとのアクション実行内容をブロック形式で記録
# どちらも enabled で個別に ON/OFF できます。

[rules.log.detect]
enabled   = true
dir       = "C:\logs"
file_name = "detect_ルール名_{Date}.log"
rotation  = "daily"

[rules.log.action]
enabled   = true
dir       = "C:\logs"
file_name = "action_ルール名_{Date}.log"
rotation  = "daily"

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
rule_name,enabled,watch_path,recursive,target,include_hidden,patterns,regex,exclude_patterns,events,action_type,destination,overwrite,preserve_structure,verify_integrity,shell,command,program,args,working_dir,message,exclude_regex,dir_patterns,dir_regex,exclude_dir_patterns,exclude_dir_regex\r\n\
ルール名,true,C:\\監視フォルダ,true,file,false,*.csv,,,create,log,,,,,,,,,,検知: {BaseName},,,,,\r\n\
";
