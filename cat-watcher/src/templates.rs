pub const GLOBAL_TOML: &str = r#"# ─── リトライ設定（copy / move 失敗時の再試行）─────────────────────────
[retry]
count       = 3       # 再試行回数
interval_ms = 1000    # 再試行間隔（ミリ秒）

# ─── システムログ（プログラム全体の起動日誌・全体で1本）───────────────
# 起動バナー / ルール一覧 / 監視開始 / 終了 と、システム階層のエラーを記録。
# 検知・アクションの結果はルール別ログ（rules.toml の [rules.log]）に出ます。
[system_log]
enabled   = true
dir       = 'C:\logs'
file_name = "system_{Date}.log"   # {Date} / {DateTime}
rotation  = "daily"               # daily / never
level     = "info"                # trace / debug / info / warn / error
console   = true                  # コンソールへの出力 ON/OFF

# ─── 起動時スキャン ───────────────────────────────────────────────────
# 起動直後に監視フォルダを 1 度だけ走査し、既に存在するファイル／フォルダを
# 「新規検知」と同じ扱いでアクション実行します。
#   ・サービス起動前から置いてあったファイル
#   ・サービス停止／再起動中（ダウンタイム）に届いたファイル
# は通常のイベント監視だけでは取りこぼしますが、これを回収する安全網です。
# events に "create" を含むルールにのみ作用します（target / patterns 等の
# フィルタは通常検知と同じく適用されます）。
[startup_scan]
enabled = true

# ─── コピー/移動先フォルダの自動作成 ───────────────────────────────────
# auto_create = true  : 宛先フォルダが無ければ実行時に自動で作ります。
#                       起動時は「ドライブや共有そのものが存在するか」だけ確認します。
# auto_create = false : 自動作成しません。宛先が無ければ起動時にエラーにします。
#                       （typo で意図しない場所へ書き込むのを防ぎたいとき）
# 個別に変えたいアクションは rules.toml 側で auto_create を書けば上書きできます。
[destination]
auto_create = true

# ─── 検知のデバウンス ─────────────────────────────────────────────────
# エディタや同期ソフトは 1 回の保存で何度もイベントを出します。最後の
# イベントから debounce_ms だけ静かになったら「確定」として 1 回だけ処理します。
#   ・巨大ファイルのコピー完了を待ちたい → debounce_ms を長めに（例: 3000）
#   ・とにかく速く反応させたい           → 短めに（例: 200）
# poll_interval_ms は「確定したものが無いか見に行く間隔」です。1 以上必須。
[detect]
debounce_ms      = 500
poll_interval_ms = 100

# ─── ダッシュボード（ブラウザでログをリアルタイム表示）─────────────────
# enabled = true でローカル HTTP サーバを起動し、ブラウザの
#   http://127.0.0.1:8080/  で検知/アクション/システムログをライブ表示します。
#   ※ 既定ビルドに同梱（enabled=false なら起動しません）。
#   ※ bind はローカル限定を推奨（ログにファイルパスが出ます。外部公開は慎重に）。
[dashboard]
enabled = false
bind    = "127.0.0.1:8080"        # 待ち受けアドレス
history = 200                     # 接続時にブラウザへ再生する直近件数

# ─── Windows サービス設定 ─────────────────────────────────────────────
# Windows サービスとして常駐したときの挙動。CLI 起動では参照されません。
# run_as_logged_in_user = true のとき、command / execute で起動する外部プロセス
# （PowerShell・7-Zip など）を「アクティブなログオンユーザー」の権限で実行します。
# サービスは既定で SYSTEM 権限で動くため、これを無効にすると外部プロセスも
# SYSTEM 権限を継承します。誰もログオンしていない状態では自動的にサービス
# アカウント権限へフォールバックします。
[service]
run_as_logged_in_user = true
"#;

pub const RULES_TOML: &str = r#"[[rules]]
enabled = true
name    = "ルール名"

[rules.watch]
path             = 'C:\監視フォルダ'
recursive        = true
target           = "file"          # file / directory / both
include_hidden   = false           # 隠しファイル/フォルダを検知対象に含めるか
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
dir       = 'C:\logs'
file_name = "detect_ルール名_{Date}.log"
rotation  = "daily"

[rules.log.action]
enabled   = true
dir       = 'C:\logs'
file_name = "action_ルール名_{Date}.log"
rotation  = "daily"

# ─── アクション例（使うものだけコメント解除してください） ────────────────

[[rules.actions]]                  # ─── log（ログ出力のみ） ─────────────
type    = "log"
message = "検知: {BaseName}"

# どのアクションにも delay_ms を書けます（実行前に待つミリ秒）。
# 書き込みが終わりきらないうちに処理が走るのを避けたいときに使います。

# [[rules.actions]]                # ─── copy ──────────────────────────────
# type               = "copy"
# destination        = 'D:\backup\{Date}'
# overwrite          = false
# preserve_structure = false
# verify_integrity   = true
# auto_create        = true        # 省略時は global.toml の [destination] に従う
# delay_ms           = 0           # 実行前に待つミリ秒

# [[rules.actions]]                # ─── move ──────────────────────────────
# type               = "move"
# destination        = 'D:\archive\{Date}'
# overwrite          = false
# preserve_structure = false
# verify_integrity   = false

# [[rules.actions]]                # ─── command ────────────────────────────
# type        = "command"
# shell       = "cmd"              # Windows: cmd / powershell / pwsh
#                                  # Linux:   bash / sh / pwsh
# command     = "echo {FullName}"
# working_dir = ""

# [[rules.actions]]                # ─── execute ────────────────────────────
# type        = "execute"
# program     = 'C:\tool\app.exe'
# args        = ["{FullName}"]
# working_dir = ""
"#;

pub const RULES_CSV: &str = "\
rule_name,enabled,watch_path,recursive,target,include_hidden,patterns,regex,exclude_patterns,events,action_type,destination,overwrite,preserve_structure,verify_integrity,shell,command,program,args,working_dir,message,exclude_regex,dir_patterns,dir_regex,exclude_dir_patterns,exclude_dir_regex,auto_create,delay_ms\r\n\
ルール名,true,C:\\監視フォルダ,true,file,false,*.csv,,,create,log,,,,,,,,,,検知: {BaseName},,,,,,,\r\n\
";

#[cfg(test)]
#[path = "tests/templates.rs"]
mod tests;
