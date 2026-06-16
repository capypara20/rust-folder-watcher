# cat-watcher

ファイルやフォルダの作成・変更・削除・リネームを検知して、コピー・移動・コマンド実行などのアクションを自動で行う、Rust 製のファイル常駐監視ツールです。

設定は TOML で書き、Excel で管理したいときは CSV からも生成できます。Windows / Linux 用のバイナリを GitHub Releases から配布しています。

## 目次

- [対応プラットフォーム](#対応プラットフォーム)
- [主な機能](#主な機能)
- [インストール](#インストール)
- [クイックスタート](#クイックスタート)
- [設定リファレンス](#設定リファレンス)
  - [global.toml（グローバル設定）](#globaltomlグローバル設定)
  - [rules.toml（ルール定義）](#rulestomlルール定義)
- [フィルタ設定](#フィルタ設定)
- [アクションの種類](#アクションの種類)
- [プレースホルダー](#プレースホルダー)
- [検知の動作仕様](#検知の動作仕様)
- [プラットフォーム別の仕様](#プラットフォーム別の仕様)
- [ログ](#ログ)
- [ダッシュボード（ブラウザでリアルタイム表示）](#ダッシュボードブラウザでリアルタイム表示)
- [常駐化（サービス登録）](#常駐化サービス登録)
- [バリデーション](#バリデーション)
- [CSV からの変換](#csv-からの変換)
- [開発](#開発)
- [ドキュメント](#ドキュメント)

## 対応プラットフォーム

| | Windows | Linux |
|---|---|---|
| 配布バイナリ | `cat-watcher.exe`（x86_64） | `cat-watcher`（x86_64） |
| 監視バックエンド | `ReadDirectoryChangesExW`（Win10 1709+ / Server 2019+、ローカル NTFS）<br>それ以外は `ReadDirectoryChangesW` にフォールバック | `inotify` |
| ネットワークパス | UNC パス（`\\server\share`）対応 ※[制約あり](#プラットフォーム別の仕様) | OS のファイル変更通知に依存（NFS 等は非保証） |
| `command` のシェル | `cmd` / `powershell` / `pwsh` | `bash` / `sh` / `pwsh` |
| 常駐化 | Windows サービス（`sc create`） | systemd 等の汎用サービス管理 |

> macOS もソースビルドで動作します（シェルは Linux と同じ）が、バイナリ配布と動作検証の対象外です。

## 主な機能

**監視・フィルタ**

- 指定フォルダの create / modify / delete / rename をリアルタイム検知（サブフォルダの再帰監視可）
- 対称設計フィルタ: ファイル名・フォルダ名 × 包含・除外 × glob・regex の 2×2×2 = 8 通りを自由に組み合わせ可能
- 監視対象の種別指定: `target = "file" / "directory" / "both"`

**アクション**

- 5 種類のアクション: log / copy / move / command（シェル経由）/ execute（プロセス直接起動）
- アクションチェーン: 1 ルールに複数アクションを順次実行（直前のコピー先を `{Destination}` で参照可能）
- プレースホルダー: 監視ファイルのパス・名前・日時などを宛先や引数に埋め込み
- 整合性検証: BLAKE3 ハッシュでコピー後のファイル一致を確認
- リトライ機構: ロック等で失敗したアクションを自動再試行

**ログ・運用**

- ログ 3 分割: システムログ（全体の起動日誌）／検知ログ／アクションログを目的別に分離
- ログローテーション: 日次切り替え（`rotation = "never"` で固定ファイルにも対応）
- ルール別ログ: ルールごとに検知ログ・アクションログを独立出力
- 全件エラー報告: 設定に複数の問題があっても 1 回の起動で全エラーをまとめて表示

**設定まわり**

- テンプレート生成: `--init global rules csv` で複数テンプレートを一括出力
- CSV → TOML 変換: Excel で書いたルールを TOML に変換する `--from-csv` モード
- ホームディレクトリ展開: パス設定で `~` が使用可能（`~/logs` など）
- 設定値の大文字小文字不区別: `create` / `Create` / `CREATE` のいずれでも動作

## インストール

[Releases ページ](https://github.com/capypara20/cat-watcher/releases) から OS に合わせたバイナリをダウンロードしてください。

- Windows: `cat-watcher.exe`
- Linux: `cat-watcher`

ソースからビルドする場合：

```bash
cargo build --release --locked --manifest-path cat-watcher/Cargo.toml
```

[ダッシュボード](#ダッシュボードブラウザでリアルタイム表示)（ブラウザでのリアルタイム表示）は**既定で同梱**されます。スリムにしたい場合のみ `--no-default-features` で外せます：

```bash
cargo build --release --locked --no-default-features --manifest-path cat-watcher/Cargo.toml
```

## クイックスタート

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

## 設定リファレンス

### global.toml（グローバル設定）

```toml
# リトライ設定（copy / move 失敗時の再試行）
[retry]
count       = 3
interval_ms = 1000

# システムログ（プログラム全体の起動日誌・全体で1本）
# 起動バナー / ルール一覧 / 監視開始 / 終了 と、システム階層のエラーを記録する。
# 検知・アクションの結果は rules.toml の [rules.log] 側に出力される。
[system_log]
enabled   = true
dir       = "C:\\logs"               # ~ によるホームディレクトリ展開も可（例: ~/logs）
file_name = "system_{Date}.log"       # {Date} / {DateTime} を埋め込み可
rotation  = "daily"                   # daily / never
level     = "info"                    # trace / debug / info / warn / error
console   = true                      # コンソールへの出力 ON/OFF
```

> **v1.3.0 でログ設定は破壊的に変更されました。** 旧 `[global]`（`log_level` /
> `log_dir` / `log_to_file` など）は廃止され、未知のキーがあると起動時にエラーで
> 停止します。`--init global` で新しいテンプレートを出力できます。

### rules.toml（ルール定義）

```toml
[[rules]]
enabled = true
name    = "csv-backup"

[rules.watch]
path             = "C:\\data\\incoming"      # ~ によるホームディレクトリ展開も可（例: ~/data）
recursive        = true
target           = "file"                # file / directory / both
include_hidden   = false                 # ※現バージョンでは未実装（値は無視される）
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
# detect: 検知イベントのみを記録（timestamp │ events │ 検知パス）
# action: 検知ごとのアクション実行内容をブロック形式で記録
# どちらも enabled で個別に ON/OFF でき、dir / file_name / rotation を各自指定する。
[rules.log.detect]
enabled   = true
dir       = "D:\\logs\\csv-backup"
file_name = "detect_csv-backup_{Date}.log"  # {Date} / {DateTime} を埋め込み可
rotation  = "daily"                          # daily / never

[rules.log.action]
enabled   = true
dir       = "D:\\logs\\csv-backup"
file_name = "action_csv-backup_{Date}.log"
rotation  = "daily"

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

ファイル名・フォルダ名それぞれに包含・除外フィルタを設定できます。各カテゴリで **glob と regex は排他**（両方設定するとバリデーションエラー）。glob / regex とも **大文字小文字を区別** します。

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
| `move`    | ファイル / ディレクトリを移動 | `destination`, `overwrite`, `preserve_structure`, `verify_integrity` |
| `command` | シェル経由でコマンド実行 | `shell`（Windows: `cmd` / `powershell` / `pwsh`、Linux: `bash` / `sh` / `pwsh`）, `command`, `working_dir` |
| `execute` | プログラムを直接起動 | `program`, `args`, `working_dir` |

### move の動作

1. まず OS の rename（同一ボリューム内の移動）を試みます
2. 移動先が別ボリューム（別ドライブ・別マウント）の場合は、**copy + 元ファイル削除** に自動フォールバックします（`verify_integrity = true` ならコピー後に BLAKE3 で検証してから元ファイルを削除）
3. `overwrite = false` で移動先に同名ファイルが存在する場合はスキップし、元ファイルは保持されます

## プレースホルダー

ルール内の `message` / `destination` / `command` / `args` などで使えます。

| プレースホルダー | 内容 | 例 |
|----------------|------|----|
| `{FullName}`      | ファイルのフルパス | `C:/data/report.csv` |
| `{DirectoryName}` | 親ディレクトリのフルパス | `C:/data` |
| `{Name}`          | ファイル名（拡張子あり） | `report.csv` |
| `{BaseName}`      | ファイル名（拡張子なし） | `report` |
| `{Extension}`     | 拡張子（ドットなし） | `csv` |
| `{RelativePath}`  | 監視ルートからの相対パス | `sub/report.csv` |
| `{WatchPath}`     | 監視ルートパス | `C:/data` |
| `{Destination}`   | 直前のアクションの出力先（チェーン用） | コピー後のフルパス |
| `{Date}`          | 検知日（YYYYMMDD） | `20240302` |
| `{Time}`          | 検知時刻（HHmmss） | `103020` |
| `{DateTime}`      | 日時（YYYYMMDD_HHmmss） | `20240302_103020` |

- パス系のプレースホルダーは **Windows でも `/` 区切りに正規化** されます（`C:\data\report.csv` → `C:/data/report.csv`）。コマンドに渡す際に `\` 区切りが必要な場合は注意してください
- `{` `}` をそのまま出力したい場合は `{{` `}}` とエスケープします
- 未知のプレースホルダーは設定読み込み時のバリデーションでエラーになります

## 検知の動作仕様

Windows / Linux 共通の挙動です。

- **イベントの集約（デバウンス）**: 同一パスへの連続イベントは集約され、**最後のイベントから 500ms 静止した時点** で評価・アクション実行されます。エディタの保存やコピー中の書き込みで modify が連発しても、アクションは 1 回にまとまります
- **events の判定**: 集約されたイベント集合とルールの `events` に 1 つでも共通があればマッチします（検知ログには `Create,Modify` のように集約結果がすべて記録されます）
- **rename の扱い**: OS からのリネーム通知は `rename` イベントとして扱われます。リネームでは旧パス・新パスそれぞれでイベントが発生することがあります
- **target の判定**: `create` / `delete` は OS の通知に含まれる種別（ファイル / フォルダ）で判定します。`modify` / `rename` はパスの実体を確認して判定します（[旧 Windows の制約](#プラットフォーム別の仕様) を参照）

## プラットフォーム別の仕様

### Windows

| 項目 | 内容 |
|---|---|
| 監視 API | **Windows 10 1709 / Server 2019 以降 + ローカル NTFS**: `ReadDirectoryChangesExW`。作成・削除イベントでファイル / フォルダの種別が通知される |
| フォールバック | **UNC（ネットワーク）パス・非 NTFS・旧 Windows**（Win10 1709 未満 / Server 2016 等）: `ReadDirectoryChangesW` |
| UNC パス | `\\server\share` 形式に対応。ただし SMB 環境で OS レベルのファイル変更通知が届かない場合は動作保証外。**サービス起動時は実行アカウントの制約あり**（[常駐化の注意事項](#windows-サービス) を参照） |
| シェル | `command` アクションは `cmd` / `powershell` / `pwsh` |
| 常駐化 | Windows サービスとして登録可能（[後述](#windows-サービス)） |

> **旧 Windows / UNC パスでの削除検知の制約**: フォールバック時は削除イベントで
> ファイル / フォルダの種別が通知されず、削除後はパスの実体も確認できないため、
> `target = "file"` / `"directory"` のルールでは **delete イベントを検知できません**。
> この環境で削除を確実に拾いたい場合は `target = "both"` を使ってください。

### Linux

| 項目 | 内容 |
|---|---|
| 監視 API | `inotify`。作成・削除イベントでファイル / フォルダの種別が通知される（上記の制約なし） |
| 再帰監視の上限 | `recursive = true` はサブディレクトリごとに inotify ウォッチを登録するため、大規模ツリーでは `fs.inotify.max_user_watches` の引き上げが必要になる場合あり |
| ネットワーク FS | NFS / CIFS マウント等では inotify 通知が届かないことがあり、動作保証外 |
| シェル | `command` アクションは `bash` / `sh` / `pwsh`（`pwsh` は PowerShell インストール時のみ） |
| 常駐化 | systemd 等の汎用サービス管理で常駐（[後述](#linux-systemd)） |

### 共通

- `~` のホームディレクトリ展開は `HOME` → `USERPROFILE` の順で解決されます（Windows でも `~/logs` と書けます）
- 設定ファイル内のパスは `C:\\data`（エスケープ）/ `C:/data` のどちらの区切りでも書けます

## ログ

ログは目的別に **3 種類** に分かれます。ターミナルにはすべてが色付きで流れますが、ファイルは用途ごとに分割されます。

| 種類 | 出力先 | 内容 |
|---|---|---|
| システムログ | 全体で1本（`[system_log]`） | 起動バナー / ルール一覧 / 監視開始 / 終了 とシステム階層のエラー |
| 検知ログ | ルール別（`[rules.log.detect]`） | 検知したイベントのみ（`timestamp │ events │ 検知パス`） |
| アクションログ | ルール別（`[rules.log.action]`） | 検知ごとのアクション実行内容（ブロック構造） |

**ターミナル出力**（カラー付き・全種類が流れる）

```
──────────────────────────────────────────────────────────────
[2026-05-07 10:30:20] [MATCH]   ルール=csv-backup | パス=C:\data\report.csv | Create,Modify
[2026-05-07 10:30:20] [ACTION]  (1/2) copy  destination=D:\backup\{Date}  overwrite=false
[2026-05-07 10:30:20] [OK]      コピー完了: C:\data\report.csv → D:\backup\20260507\report.csv  [BLAKE3: ...]
[2026-05-07 10:30:20] [ACTION]  (2/2) log
[2026-05-07 10:30:20] [OK]      検知: report.csv
```

**① システムログ**（`system_{Date}.log`）

```
2026-05-07 10:30:18 │ INFO  │ cat-watcher 起動  global=... rules=...
2026-05-07 10:30:18 │ INFO  │ 監視ルール [csv-backup] (有効)  パス=C:\data\incoming  イベント=作成, 変更  サブフォルダ=あり
2026-05-07 10:30:25 │ INFO  │ 終了シグナル受信
```

**② 検知ログ**（`detect_csv-backup_{Date}.log`）

```
2026-05-07 10:30:20 │ Create,Modify        │ C:\data\report.csv
```

**③ アクションログ**（`action_csv-backup_{Date}.log`・1検知=1ブロック、`#N` は日次でリセット）

```
═══ #1  2026-05-07 10:30:20  C:\data\report.csv  (Create,Modify)  actions=2 ═══
2026-05-07 10:30:20 │ 1. copy   │ destination=D:\backup\{Date}  overwrite=false
2026-05-07 10:30:20 │ 1. OK     │ コピー完了: C:\data\report.csv → D:\backup\20260507\report.csv  [BLAKE3: ...]
2026-05-07 10:30:20 │ 2. log    │
2026-05-07 10:30:20 │ 2. OK     │ 検知: report.csv
```

アクションが失敗した場合は `1. WARN`（リトライ）→ `1. ERR`（最終失敗）の順に
**アクションログにのみ** 記録されます（システムログには残りません）。
`[system_log]` の `console = false` でターミナル出力を、各ログの `enabled = false`
で個別にファイル出力を無効にできます。

## ダッシュボード（ブラウザでリアルタイム表示）

検知・アクション・システムログを **ブラウザでリアルタイム表示** できます。常駐
（特に Windows サービスのようにコンソールが見えない運用）でも、ブラウザを開けば
今まさに流れているイベントを確認できます。

ターミナルの代替となる読み取り専用のライブビューで、ファイルログ（前述の 3 種）は
従来どおり出力されます（**ファイルログが正・ダッシュボードは補助**）。

### ビルドと有効化

ダッシュボードは**既定ビルドに同梱**されています（`cargo build` でそのまま含まれ、リリースのバイナリにも入ります）。不要なら `--no-default-features` で外せます。

`global.toml` に `[dashboard]` を追加し `enabled = true` にすると、監視の起動と同時に
ローカル HTTP サーバが立ち上がります。

```toml
[dashboard]
enabled = false             # true で起動
bind    = "127.0.0.1:8080"  # 待ち受けアドレス
history = 200               # 接続時にブラウザへ再生する直近イベント件数
```

ブラウザで `http://127.0.0.1:8080/` を開くと、種別（system / detect / action）や
キーワードでの絞り込み、一時停止、自動スクロールができます。接続した瞬間に直近
`history` 件が再生され、以降はリアルタイムに追記されます。

### 仕組みと注意点

- **配信方式**: Server-Sent Events（SSE）。ブラウザの再接続は自動。
- **クロスプラットフォーム**: localhost の TCP ポートを開くだけなので Windows / Linux で
  同じ動作。Windows サービスとして常駐していても利用できます。
- **負荷の扱い**: 配信は取りこぼし許容で、ブラウザ側が詰まっても監視・アクションの
  処理は遅延しません。
- **セキュリティ**: 既定は `127.0.0.1`（ローカルのみ）。ログにはファイルパスが
  含まれるため、外部公開は慎重に。リモートで見たい場合は SSH トンネルや
  リバースプロキシ経由を推奨します（本体に認証・TLS はありません）。
- `--no-default-features` でビルドした場合、`[dashboard]` セクションを書いても無視されます
  （設定エラーにはなりません）。

## 常駐化（サービス登録）

### Windows サービス

`cat-watcher.exe` は Windows サービスとして登録・常駐できます。OS 起動時に自動で監視を開始したい場合に使います。

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

:: 停止・削除
sc stop cat-watcher
sc delete cat-watcher
```

注意事項:

- `binPath=` の後のパスはすべて **絶対パス** で指定してください（`~` や相対パスは SCM が解釈できません）
- サービスモードでは **コンソール出力は自動で無効** になり、ファイルログのみ出力されます
- `global.toml` の `[system_log]`（`enabled` / `dir`）を正しく設定しておく必要があります
- 通常の CLI 起動（`cat-watcher -g ... -r ...`）の動作は変わりません

**UNC（ネットワーク）パスを監視する場合**:

サービスは既定で **LocalSystem アカウント** で実行されます。LocalSystem はネットワーク共有への資格情報を持たないため、`watch.path` や `destination` に UNC パス（`\\server\share\...`）があると **サービス起動時はアクセスできず、監視・コピーが失敗します**（CLI 起動ではログオンユーザーの資格情報で接続されるため動作する、という差が出ます）。

UNC パスを使う場合は、共有にアクセスできるアカウントでサービスを実行するよう変更してください:

```cmd
:: 実行アカウントを変更（services.msc の「ログオン」タブでも変更可）
sc config cat-watcher obj= "DOMAIN\watcher-user" password= "********"
sc stop cat-watcher && sc start cat-watcher
```

- `net use` でマップしたドライブ文字（`Z:\` 等）はユーザーセッション単位のため、サービスからは **アカウントを変更しても見えません**。必ず UNC 形式で指定してください

### Linux (systemd)

Linux には専用のサービスモードはなく、通常のフォアグラウンドプロセスとして動作するため、systemd のユニットファイルで常駐させます。

`/etc/systemd/system/cat-watcher.service`:

```ini
[Unit]
Description=cat-watcher folder watcher
After=network.target

[Service]
ExecStart=/opt/cat-watcher/cat-watcher --global /opt/cat-watcher/global.toml --rules /opt/cat-watcher/rules.toml
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now cat-watcher
systemctl status cat-watcher
```

- パスは絶対パスで指定してください（`~` 展開はユニット内では使えません）
- コンソール出力（`console = true`）は journald に記録されます。ファイルログと二重に残したくない場合は `console = false` にしてください

## バリデーション

`--validate` フラグを付けると、設定ファイルの妥当性チェックのみ実行して終了します。複数の問題があるときはすべて一覧で表示されます。

```
バリデーションエラーが 3 件見つかりました:
  [1] system_log.dir が存在しません: C:\logs\app
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

## 開発

```bash
# テスト
cargo test --locked --manifest-path cat-watcher/Cargo.toml

# リリースビルド
cargo build --release --locked --manifest-path cat-watcher/Cargo.toml
```

CI（`.github/workflows/ci.yml`）は main 以外のブランチへのプッシュと PR で、Ubuntu / Windows 上のテストを実行します。

`main` への push で `.github/workflows/release.yml` が走り、`Cargo.toml` のバージョンを元に `vX.Y.Z` タグを作成し、Windows / Linux のバイナリを GitHub Releases に公開します（同バージョンのリリースが既にある場合はスキップ）。

## ドキュメント

詳細な仕様は [`doc/specification.md`](doc/specification.md)、設計資料は [`doc/`](doc/) 配下を参照してください。
