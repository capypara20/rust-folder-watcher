# cat-watcher

ファイルやフォルダの作成・変更・削除・リネームを検知して、コピー・移動・コマンド実行などのアクションを自動で行う、Rust 製のファイル常駐監視ツールです。

設定は TOML で書き、Excel で管理したいときは CSV からも生成できます。Windows / Linux 用のバイナリを GitHub Releases から配布しています。

## 主な機能

- **リアルタイム監視**: 指定フォルダの create / modify / delete / rename を検知
- **対称設計フィルタ**: ファイル名・フォルダ名 × 包含・除外 × glob・regex の 2×2×2 = 8 通りのフィルタを自由に組み合わせ可能
- **5 種類のアクション**: log / copy / move / command（シェル経由）/ execute（プロセス直接起動）
- **クロスプラットフォームシェル**: Windows は cmd / powershell / pwsh、Linux・macOS は bash / sh / pwsh に対応
- **アクションチェーン**: 1 ルールに複数アクションを順次実行（直前のコピー先を `{Destination}` で参照可能）
- **プレースホルダー**: 監視ファイルのパス・名前・日時などを宛先や引数に埋め込める
- **整合性検証**: BLAKE3 ハッシュでコピー後のファイル一致を確認
- **リトライ機構**: ロック等で失敗したアクションを自動再試行
- **ログローテーション**: 日次でログファイルを切り替え（`log_rotation = "never"` で固定ファイルにも対応）
- **ルール別ログ**: ルールごとに独立したログファイルへ出力（`[rules.log]` セクションで設定）
- **ログ出力先の個別制御**: コンソール・ファイルを個別に有効/無効、ログレベルも別々に指定可能
- **テンプレート生成**: `--init global rules csv` のように複数のテンプレートを一括で出力できる
- **ホームディレクトリ展開**: パス設定で `~` が使用可能（`~/logs` など）
- **ログ CSV 形式**: ファイルログを CSV 出力するので `grep` や `awk` でフィルタリングが容易
- **全件エラー報告**: 設定ファイルに複数の問題があっても、1 回の起動で全エラーをまとめて表示
- **大文字小文字不区別**: 設定値は `create` / `Create` / `CREATE` のいずれでも動作
- **CSV → TOML 変換**: Excel で書いたルールを TOML に変換する `--from-csv` モード

## インストール

[Releases ページ](https://github.com/capypara20/cat-watcher/releases) から OS に合わせたバイナリをダウンロードしてください。

- Windows: `cat-watcher.exe`
- Linux: `cat-watcher`

ソースからビルドする場合：

```bash
cargo build --release --manifest-path cat-watcher/Cargo.toml
```

## 使い方

### 基本

```bash
# 設定ファイルのテンプレートを生成（まずここから）
cat-watcher --init global          # global.toml を生成
cat-watcher --init rules           # rules.toml を生成
cat-watcher --init csv             # rules.csv を生成

# 複数テンプレートを一括生成
cat-watcher --init global rules
cat-watcher --init global rules csv

# 出力先ファイルを明示する場合（--init が1種類のときのみ使用可）
cat-watcher --init global --output config\global.toml
cat-watcher --init rules  --output config\rules.toml

# 設定を確認してから監視を開始
cat-watcher --global global.toml --rules rules.toml --validate
cat-watcher --global global.toml --rules rules.toml

# 引数の短縮系
cat-watcher -g global.toml -r rules.toml

# CSV をルール TOML に変換
cat-watcher --from-csv rules.csv --output rules.toml
```

引数なしで起動すると使い方のガイドが表示されます。

### global.toml（グローバル設定）

```toml
[global]
log_level         = "info"                    # trace / debug / info / warn / error
log_dir           = "C:\\logs"               # ~ によるホームディレクトリ展開も可（例: ~/logs）
log_file_name     = "cat-watcher_{Date}.log"  # {Date} / {DateTime} を埋め込み可
log_rotation      = "daily"                   # daily / never
retry_count       = 3
retry_interval_ms = 1000

# ログ出力先の制御（省略時はどちらも true）
log_to_console    = true
log_to_file       = true

# コンソール・ファイルで異なるログレベルを使いたい場合に設定（省略時は log_level を使用）
# terminal_log_level = "info"
# file_log_level     = "debug"
```

### rules.toml（ルール定義）

```toml
[[rules]]
enabled = true
name    = "csv-backup"

[rules.watch]
path             = "C:\\data\\incoming"      # ~ によるホームディレクトリ展開も可（例: ~/data）
recursive        = true
target           = "file"                # file / directory / both
include_hidden   = false
patterns         = ["*.csv", "*.xlsx"]   # glob（regex と排他）
# regex          = ".*\\.csv$"           # 正規表現（patterns と排他）
exclude_patterns = ["temp_*"]            # glob（exclude_regex と排他）
# exclude_regex  = "^temp_"             # 正規表現（exclude_patterns と排他）
dir_patterns     = ["incoming", "drop"]  # 包含フォルダ名 glob（dir_regex と排他）
# dir_regex      = "^drop_\\d+"         # 包含フォルダ名 正規表現（dir_patterns と排他）
exclude_dir_patterns = ["node_modules"]  # 除外フォルダ名 glob（exclude_dir_regex と排他）
# exclude_dir_regex  = "^\\..*"         # 除外フォルダ名 正規表現（exclude_dir_patterns と排他）
events           = ["create", "modify"]  # create / modify / delete / rename

# ── ルール別ログ（省略可）──────────────────────────────────────────────────
# このルールにマッチしたイベントをルール専用のログファイルにも書き出す
[rules.log]
enabled       = true
log_dir       = "D:\\logs\\csv-backup"
log_file_name = "csv-backup_{Date}.log"  # {Date} / {DateTime} を埋め込み可
log_rotation  = "daily"                  # daily / never

# ──────────── アクションチェーン ────────────
[[rules.actions]]
type    = "log"
message = "検知: {BaseName}"

[[rules.actions]]
type               = "copy"
destination        = "D:\\backup\\{Date}"
overwrite          = false
preserve_structure = true
verify_integrity   = true                # BLAKE3 でコピー検証

[[rules.actions]]
type        = "command"
shell       = "powershell"               # Windows: cmd / powershell / pwsh
                                         # Linux・macOS: bash / sh / pwsh
command     = "Write-Host 'Backed up: {Name} -> {Destination}'"
working_dir = ""
```

## フィルタ設定

ファイル名・フォルダ名それぞれに包含・除外フィルタを設定できます。各カテゴリで **glob と regex は排他**（両方設定するとバリデーションエラー）。

| | ファイル名 | フォルダ名 |
|---|---|---|
| **包含** | `patterns` / `regex` | `dir_patterns` / `dir_regex` |
| **除外** | `exclude_patterns` / `exclude_regex` | `exclude_dir_patterns` / `exclude_dir_regex` |

### フォルダフィルタの動作

`dir_patterns` / `dir_regex` は、イベントパスを監視ルートからの相対パスに変換し、その**親ディレクトリ成分のいずれか**にマッチすれば通過します。

```
watch_path = C:\project、dir_patterns = ["src"] の場合:

C:\project\src\main.rs          → 親: [src]             ✅ src にマッチ → 通過
C:\project\lib\utils.rs         → 親: [lib]             ❌ マッチなし → 除外
C:\project\packages\app\src\index.ts → 親: [packages, app, src]  ✅ src にマッチ → 通過
C:\project\config.toml          → 親: []（なし）         ❌ マッチなし → 除外
```

> **注意**: 監視ルート直下のファイルはフォルダを経由しないため、`dir_patterns` / `dir_regex` が設定されていると常に除外されます。

### 除外優先ルール

包含フィルタと除外フィルタが同じフォルダ・ファイルにマッチする場合、**除外が優先**されます。

```
dir_patterns         = ["src"]
exclude_dir_patterns = ["src"]

→ src 配下のファイルは包含チェックを通過するが、その後の除外チェックで除外される
→ 結果: 何も検知されない（設定の矛盾に注意）
```

実用的な組み合わせ例:

```
dir_patterns         = ["src"]           # src フォルダ配下のみ対象
exclude_dir_patterns = ["node_modules"]  # ただし node_modules は除外

→ src\components\Button.ts    ✅（src にマッチ、node_modules なし）
→ src\node_modules\react\index.ts  ❌（node_modules が除外優先）
```

## アクションの種類

| type | 用途 | 主なオプション |
|------|------|----------------|
| `log`     | イベントをログファイルに記録するだけ（コマンド実行なし） | `message` |
| `copy`    | ファイル / ディレクトリをコピー | `destination`, `overwrite`, `preserve_structure`, `verify_integrity` |
| `move`    | ファイル / ディレクトリを移動（異ボリュームは copy + delete にフォールバック） | `destination`, `overwrite`, `preserve_structure`, `verify_integrity` |
| `command` | シェル経由でコマンド実行 | `shell`（Windows: `cmd` / `powershell` / `pwsh`、Linux: `bash` / `sh` / `pwsh`）, `command`, `working_dir` |
| `execute` | プログラムを直接起動 | `program`, `args`, `working_dir` |

## プレースホルダー

ルール内の `message` / `destination` / `command` / `args` などで使えます。

| プレースホルダー | 内容 | 例 |
|----------------|------|----|
| `{FullName}`      | ファイルのフルパス | `C:\data\report.csv` |
| `{Name}`          | ファイル名（拡張子なし） | `report` |
| `{BaseName}`      | ファイル名（拡張子あり） | `report.csv` |
| `{Extension}`     | 拡張子 | `.csv` |
| `{DirectoryName}` | 親ディレクトリのフルパス | `C:\data` |
| `{WatchPath}`     | 監視ルートパス | `C:\data` |
| `{RelativePath}`  | 監視ルートからの相対パス | `sub\report.csv` |
| `{Date}`          | 検知日 | `20240302` |
| `{Time}`          | 検知時刻 | `103020` |
| `{DateTime}`      | 日時 | `20240302_103020` |
| `{Destination}`   | 直前のアクションの出力先（チェーン用） | コピー後のフルパス |

## バリデーション

`--validate` フラグを付けると、設定ファイルの妥当性チェックのみ実行して終了します。複数の問題があるときはすべて一覧で表示されます。

```
バリデーションエラーが 3 件見つかりました:
  [1] log_dir が存在しません: C:\logs\app
  [2] 監視ルール名 csv-backup の watch.path が存在しません: C:\data\incoming
  [3] 監視ルール名 log-processor のアクションの type が Command のとき、shell を定義してください
```

## CSV からの変換

CSV の列順（1 行目はヘッダー、自動でスキップ）：
** 未検証... **

```
rule_name, enabled, watch_path, recursive, target, include_hidden,
patterns, regex, exclude_patterns, exclude_regex,
dir_patterns, dir_regex, exclude_dir_patterns, exclude_dir_regex, events,
action_type, destination, overwrite, preserve_structure, verify_integrity,
shell, command, program, args, working_dir, message
```

- 同じ `rule_name` の行を複数並べると、1 ルールに複数アクションを定義できます
- 配列フィールド（`patterns` / `events` / `args` 等）は `|` 区切り（例: `create|modify`）
- `log` アクションは `action_type = "log"` とし、`message` 列にメッセージを記入します

`--init csv` でヘッダー付きのサンプル CSV を生成できます。

## ログ

ターミナルとファイルでフォーマットが異なります。

**ターミナル出力**（カラー付き）

```
──────────────────────────────────────────────────────────────
[2026-05-07 10:30:20] [MATCH]  パス=C:\data\report.csv | イベント=Create|Modify | ルール=csv-backup
[2026-05-07 10:30:20] [ACTION]  (1/3) log
[2026-05-07 10:30:20] [INFO]    検知: report.csv
[2026-05-07 10:30:20] [ACTION]  (2/3) copy  C:\data\report.csv → D:\backup\20260507
[2026-05-07 10:30:20] [OK]      コピー完了: C:\data\report.csv → D:\backup\20260507\report.csv  [BLAKE3: ...]
[2026-05-07 10:30:20] [ACTION]  (3/3) command  shell=powershell  cmd=Write-Host 'Backed up: ...'
[2026-05-07 10:30:20] [OK]      起動
```

**ファイル出力**（CSV 形式）

```
2026-05-07 10:30:20,INFO,cat-watcher 起動
2026-05-07 10:30:20,INFO,監視ルール [csv-backup]  パス=C:\data\incoming  サブフォルダ=あり
2026-05-07 10:30:20,MATCH,C:\data\report.csv,Create|Modify,csv-backup
2026-05-07 10:30:20,ACTION,1/3,log,
2026-05-07 10:30:20,SUCCESS,1/3,検知: report.csv
2026-05-07 10:30:20,ACTION,2/3,copy,C:\data\report.csv → D:\backup\{Date}
2026-05-07 10:30:20,SUCCESS,2/3,コピー完了: C:\data\report.csv → D:\backup\20260507\report.csv  [BLAKE3: ...]
2026-05-07 10:30:20,ACTION,3/3,command,shell=powershell  cmd=Write-Host 'Backed up: ...'
2026-05-07 10:30:20,SUCCESS,3/3,起動
```

CSV 形式なので `grep` / `awk` / Excel での集計が容易です。

```bash
# MATCH レコードのみ抽出
grep ",MATCH," cat-watcher.log

# MATCH のパス一覧（3列目）
grep ",MATCH," cat-watcher.log | cut -d, -f3

# ルール名でフィルタ（5列目）
grep ",MATCH," cat-watcher.log | awk -F, '$5=="csv-backup"'
```

各カラムの意味：

| レコード種別 | フォーマット |
|---|---|
| MATCH | `timestamp,MATCH,path,events,rule_name` |
| ACTION | `timestamp,ACTION,index/total,type,detail` |
| SUCCESS | `timestamp,SUCCESS,index/total,message` |
| INFO / WARN / ERROR | `timestamp,LEVEL,message` |

`log_to_console = false` でターミナル出力を、`log_to_file = false` でファイル出力を無効にできます。`terminal_log_level` / `file_log_level` でそれぞれのログレベルを個別に設定することもできます。

## Windows サービスとして登録する

`cat-watcher.exe` は Windows サービスとして登録・常駐できます。OS 起動時に自動で監視を開始したい場合に使います。

### 登録手順

管理者権限のコマンドプロンプト（または PowerShell）で実行してください。

```cmd
:: サービスを登録（--global / --rules のパスは絶対パスで指定）
sc create cat-watcher ^
  binPath= "C:\tools\cat-watcher.exe --global C:\tools\global.toml --rules C:\tools\rules.toml" ^
  start= auto ^
  DisplayName= "cat-watcher"

:: サービスを開始
sc start cat-watcher

:: サービスの状態を確認
sc query cat-watcher
```

### 停止・削除

```cmd
sc stop cat-watcher
sc delete cat-watcher
```

### 注意事項

- `binPath=` の後のパスはすべて **絶対パス** で指定してください（`~` や相対パスは SCM が解釈できません）
- サービスモードでは **コンソール出力は自動で無効** になり、ファイルログのみ出力されます
- `global.toml` の `log_to_file = true` と `log_dir` を正しく設定しておく必要があります
- 通常の CLI 起動（`cat-watcher -g ... -r ...`）の動作は変わりません

## 開発

```bash
# テスト
cargo test --manifest-path cat-watcher/Cargo.toml

# リリースビルド
cargo build --release --manifest-path cat-watcher/Cargo.toml
```

`main` への push で `.github/workflows/release.yml` が走り、`Cargo.toml` のバージョンを元に `vX.Y.Z` タグを作成し、Windows / Linux のバイナリを GitHub Releases に公開します。

## ドキュメント

詳細な仕様は [`doc/specification.md`](doc/specification.md)、設計資料は [`doc/`](doc/) 配下を参照してください。
