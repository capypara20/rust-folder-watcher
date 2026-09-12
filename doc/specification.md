# 運用仕様書 — ファイル常駐監視アプリケーション

| 項目 | 内容 |
|------|------|
| 文書版数 | 1.1 |
| 作成日 | 2025-07 |
| 対象読者 | システム管理者・運用担当者 |

---

## 目次

1. [概要](#1-概要)
2. [動作環境](#2-動作環境)
3. [ファイル構成](#3-ファイル構成)
4. [設定ファイルの書き方](#4-設定ファイルの書き方)
5. [アプリケーションの起動と停止](#5-アプリケーションの起動と停止)
6. [プレースホルダリファレンス](#6-プレースホルダリファレンス)
7. [ログの見方](#7-ログの見方)
8. [設定例集](#8-設定例集)
9. [トラブルシューティング](#9-トラブルシューティング)

---

## 1. 概要

本アプリケーション（`watcher.exe`）は、指定したフォルダをリアルタイムに監視し、ファイルやフォルダの作成・変更・削除・リネームを検知して、あらかじめ定義されたアクション（コピー・移動・コマンド実行・外部プロセス起動）を自動実行するツールです。

### 運用フロー

```
[rules.toml を編集]  ───┐
                        ├──→  cat-watcher で監視開始
[global.toml を編集] ───┘

雛形は `cat-watcher --init global rules` で生成できます。
```

---

## 2. 動作環境

| 項目 | 要件 |
|------|------|
| OS | Windows 10 以降（64bit） |
| ファイルシステム | NTFS（ローカルドライブ）。UNC パス（SMB）にも対応（※1） |
| ランタイム | 不要（単一の `.exe` で動作） |

> ※1 SMB 環境で OS レベルのファイル変更通知が届かない場合は動作保証外です。

---

## 3. ファイル構成

```
watcher.exe          # 監視アプリケーション本体
config/
  global.toml        # グローバル設定
  rules.toml         # 監視ルール定義
logs/
  watcher.log        # ログ出力先（自動生成）
```

---

## 4. 設定ファイルの書き方

設定ファイルは **2 ファイル構成**です。すべての項目に省略やデフォルト値はありません。必ず全項目を明示的に記述してください。

### 4.1 global.toml（グローバル設定）

手動で編集します。

```toml
[global]
log_level = "info"              # ログレベル: trace / debug / info / warn / error
log_file = "./logs/watcher.log" # ログ出力先パス
log_rotation = "daily"          # ログローテーション: daily（日次） / never（なし）
retry_count = 3                 # アクション失敗時のリトライ回数
retry_interval_ms = 1000        # リトライ間隔（ミリ秒）
dry_run = false                 # true にするとアクションを実行せずログのみ出力
```

| 項目 | 説明 |
|------|------|
| `log_level` | 出力するログの最低レベル。`trace` が最も詳細、`error` が最も簡潔 |
| `log_file` | ログファイルのパス。相対パスの場合は watcher.exe の実行ディレクトリ基準 |
| `log_rotation` | `daily` を推奨。日付ごとにファイルが分割されます |
| `retry_count` | ファイルが他プロセスにロックされている場合等のリトライ回数 |
| `retry_interval_ms` | リトライの待機間隔。1000 = 1秒 |
| `dry_run` | テスト時に `true` に設定すると、実際のコピー・移動・コマンド実行を行いません |

### 4.2 rules.toml（監視ルール定義）

`cat-watcher --init rules` で雛形を生成し、手動で編集します。

```toml
[[rules]]
name = "csv-backup"               # ルール名（任意の識別名）
enabled = true                     # ルールの有効/無効

[rules.watch]
path = "C:/data/incoming"          # 監視対象ディレクトリ
recursive = true                   # true: サブディレクトリも監視 / false: 直下のみ
target = "file"                    # 検知対象: file / directory / both
include_hidden = false             # true: 隠しファイルも検知 / false: 除外
patterns = ["*.csv", "report_*"]   # glob パターン
events = ["create", "modify"]      # 検知するイベント: create / modify / delete / rename

[[rules.actions]]
type = "copy"                      # アクション種別: copy / move / command / execute
destination = "C:/data/backup"     # コピー先ディレクトリ
overwrite = false                  # 同名ファイルが存在する場合: true=上書き / false=スキップ
preserve_structure = true          # true: サブディレクトリ構造を維持 / false: フラット配置
verify_integrity = true            # true: BLAKE3 ハッシュ値比較で完全性検証 / false: 検証しない
```

#### 検知対象（target）と再帰監視（recursive）の組み合わせ

| | `recursive = false` | `recursive = true` |
|---|---|---|
| `target = "file"` | 直下のファイルのみ | 全階層のファイル |
| `target = "directory"` | 直下のフォルダのみ | 全階層のフォルダ |
| `target = "both"` | 直下のファイル＋フォルダ | 全階層のファイル＋フォルダ |

#### パターン指定

ファイル名のフィルタリングには **glob パターン**または**正規表現**のいずれか一方を指定します（両方指定・両方省略はエラー）。

```toml
# glob パターン（複数指定可）
patterns = ["*.csv", "report_*.xlsx"]

# または正規表現（一つだけ）
# regex = "^\\d{8}_report\\.csv$"
```

除外パターンも指定できます:
```toml
patterns = ["*"]
exclude_patterns = ["*.tmp", "*.bak"]
```

#### アクション種別ごとの設定

**copy / move:**
```toml
[[rules.actions]]
type = "copy"                     # または "move"
destination = "C:/data/backup"
overwrite = false
preserve_structure = true
verify_integrity = true            # BLAKE3 ハッシュ値比較でコピー/移動の完全性を検証
```

**command（シェルコマンド実行）:**
```toml
[[rules.actions]]
type = "command"
shell = "powershell"              # cmd / powershell / pwsh
command = "Write-Host 'Detected: {Name}'"
working_dir = "C:/scripts"       # 省略可
```

**execute（外部プロセス直接起動）:**
```toml
[[rules.actions]]
type = "execute"
program = "C:/tools/processor.exe"
args = ["--input", "{FullName}", "--mode", "auto"]
working_dir = "C:/tools"         # 省略可
```

#### アクションチェーン

1 つのルールに複数のアクションを定義すると、上から順に実行されます。

```toml
# 1. まずコピー
[[rules.actions]]
type = "copy"
destination = "C:/data/backup"
overwrite = false
preserve_structure = true
verify_integrity = true

# 2. コピー完了後にコマンド実行
[[rules.actions]]
type = "command"
shell = "cmd"
command = "echo Backed up: {Name} >> C:/data/backup.log"
```

> **注意**: チェーン中にエラーが発生すると、後続のアクションは中断されます。

---

## 5. アプリケーションの起動と停止

### 6.1 起動

```powershell
watcher.exe --global config/global.toml --rules config/rules.toml
```

#### オプション

| オプション | 説明 |
|-----------|------|
| `--global <path>` / `-g` | global.toml のパス（必須） |
| `--rules <path>` / `-r` | rules.toml のパス（必須） |
| `--dry-run` | ドライランモード（global.toml の設定を上書き） |
| `--log-level <level>` | ログレベルの上書き |
| `--validate` | 設定ファイルのバリデーションのみ実行（監視は開始しない） |
| `--version` / `-V` | バージョン表示 |
| `--help` / `-h` | ヘルプ表示 |

#### 起動時の動作

1. 設定ファイルを読み込み、バリデーションを実行します
2. バリデーションに問題がなければ、監視対象ディレクトリ内の**既存ファイル**に対してルール評価を実行します
3. その後、リアルタイム監視を開始します

> **注意**: 起動のたびに既存ファイルが再処理されます（過去の処理履歴は保持しません）。

#### 設定のテスト

実際に監視を開始せずに設定ファイルの妥当性だけを確認できます:

```powershell
watcher.exe --global config/global.toml --rules config/rules.toml --validate
```

### 6.2 停止

`Ctrl+C` を押すと安全に停止します。

- 実行中のコピー/移動は完了を待ってから終了します
- 起動済みのコマンド/外部プロセスはそのまま動作を続けます（管理対象外）

### 6.3 設定変更の反映

設定を変更した場合は、**アプリケーションを再起動**してください。実行中の設定変更の自動反映（ホットリロード）は行いません。

### 6.4 終了コード

| コード | 意味 |
|--------|------|
| `0` | 正常終了 |
| `1` | 設定ファイルエラー |
| `2` | 実行時エラー（監視ディレクトリの消失等） |

---

## 6. プレースホルダリファレンス

`command` や `execute` のコマンド文字列、引数にプレースホルダを使用できます。

### 7.1 一覧

検知ファイル `C:/data/sub/report_2026.csv`（watch_path: `C:/data`）の場合:

| プレースホルダ | 値の例 | 説明 |
|---------------|--------|------|
| `{FullName}` | `C:/data/sub/report_2026.csv` | 絶対パス |
| `{DirectoryName}` | `C:/data/sub` | 親ディレクトリ |
| `{Name}` | `report_2026.csv` | ファイル名（拡張子付き） |
| `{BaseName}` | `report_2026` | ファイル名（拡張子なし） |
| `{Extension}` | `csv` | 拡張子（ドットなし） |
| `{RelativePath}` | `sub/report_2026.csv` | 監視パスからの相対パス |
| `{WatchPath}` | `C:/data` | 監視パス |
| `{Destination}` | `C:/backup/sub/report_2026.csv` | 直前の copy/move 先パス |
| `{Date}` | `20260328` | 実行日（YYYYMMDD） |
| `{Time}` | `143052` | 実行時刻（HHmmss） |
| `{DateTime}` | `20260328_143052` | 実行日時 |

### 7.2 使用例

```toml
# コマンドでプレースホルダを使用
command = "magick convert {FullName} -resize 50% {DirectoryName}/thumb_{Name}"

# 外部プロセスの引数で使用
args = ["--input", "{FullName}", "--output", "C:/out/{BaseName}_{DateTime}.{Extension}"]
```

### 7.3 エスケープ

リテラルの `{` `}` を出力する場合は `{{` `}}` と記述します。

```toml
command = "echo {{result}}: {Name}"
# → echo {result}: report_2026.csv
```

### 7.4 {Extension} の補足

- `{Extension}` はドットを含みません（例: `csv`）
- ドット付きで使いたい場合: `.{Extension}` と記述します
- 拡張子がないファイルの場合、空文字列になります

### 7.5 アクションチェーン時の動作

| 前のアクション | 後続の {FullName} 等 | {Destination} |
|---------------|---------------------|--------------| 
| copy | 変更なし（元ファイルのパス） | コピー先パス |
| move | **移動後のパスに更新** | 移動先パス |
| command / execute | 変更なし | 変更なし |

> `move` の後は元ファイルが存在しなくなるため、パス系プレースホルダが自動的に移動先に更新されます。

---

## 7. ログの見方

### 8.1 ログ形式

ログは JSON 形式で 1 行 = 1 レコードです。

```json
{"timestamp":"2026-03-28T14:30:52.123+09:00","level":"INFO","event":"file_detected","rule":"csv-backup","event_type":"create","file_path":"C:/data/incoming/report.csv","target":"file"}
```

### 8.2 主要フィールド

| フィールド | 説明 |
|-----------|------|
| `timestamp` | 発生日時（ISO 8601、タイムゾーン付き） |
| `level` | ログレベル（TRACE / DEBUG / INFO / WARN / ERROR） |
| `event` | 何が起きたか（下表参照） |
| `rule` | 対象ルール名 |
| `file_path` | 検知したファイルパス |
| `error` | エラーメッセージ（エラー時のみ） |

### 8.3 イベント一覧

| event | 説明 |
|-------|------|
| `app_started` | アプリケーションが起動した |
| `app_shutdown` | アプリケーションが終了した |
| `file_detected` | ファイル/フォルダを検知した |
| `action_started` | アクション実行を開始した |
| `action_completed` | アクションが正常完了した |
| `action_failed` | アクションが失敗した |
| `action_retry` | アクションをリトライした |
| `chain_aborted` | エラーによりアクションチェーンが中断された |
| `validation_error` | 設定バリデーションエラー |
| `watch_path_lost` | 監視ディレクトリが消失した |

### 8.4 ログローテーション

| 設定値 | ファイル名例 | 説明 |
|--------|-------------|------|
| `daily` | `watcher.2026-03-28.log` | 日付ごとにファイルが切り替わります |
| `never` | `watcher.log` | 単一ファイルに書き続けます |

---

## 8. 設定例集

### 9.1 CSV ファイルを検知してバックアップコピー

```toml
[[rules]]
name = "csv-backup"
enabled = true

[rules.watch]
path = "C:/data/incoming"
recursive = true
target = "file"
include_hidden = false
patterns = ["*.csv"]
events = ["create"]

[[rules.actions]]
type = "copy"
destination = "C:/data/backup"
overwrite = false
preserve_structure = true
verify_integrity = true
```

### 9.2 画像ファイルを検知してサムネイル生成 + アップロード

```toml
[[rules]]
name = "image-process"
enabled = true

[rules.watch]
path = "C:/data/images"
recursive = false
target = "file"
include_hidden = false
patterns = ["*.png", "*.jpg"]
events = ["create"]

[[rules.actions]]
type = "command"
shell = "powershell"
command = "magick convert {FullName} -resize 50% {DirectoryName}/thumb_{Name}"
working_dir = "C:/data/images"

[[rules.actions]]
type = "execute"
program = "C:/tools/uploader.exe"
args = ["--input", "{FullName}", "--mode", "auto"]
```

### 9.3 フォルダが作成されたらまるごとコピー

```toml
[[rules]]
name = "folder-intake"
enabled = true

[rules.watch]
path = "C:/data/incoming"
recursive = false
target = "directory"
include_hidden = false
patterns = ["batch_*"]
events = ["create"]

[[rules.actions]]
type = "copy"
destination = "C:/data/processed"
overwrite = false
preserve_structure = false
verify_integrity = true
```

### 9.4 ログファイルをアーカイブに移動してログ記録

```toml
[[rules]]
name = "log-archive"
enabled = true

[rules.watch]
path = "C:/app/logs"
recursive = false
target = "file"
include_hidden = false
patterns = ["*.log"]
events = ["create"]

[[rules.actions]]
type = "move"
destination = "C:/archive/logs"
overwrite = false
preserve_structure = false
verify_integrity = true

[[rules.actions]]
type = "command"
shell = "cmd"
command = "echo Archived: {Name} at {DateTime} >> C:/archive/archive.log"
```

### 9.5 パターン不一致ファイルを隔離

```toml
[[rules]]
name = "unmatched-quarantine"
enabled = true

[rules.watch]
path = "C:/data/incoming"
recursive = false
target = "file"
include_hidden = false
patterns = ["*"]
exclude_patterns = ["*.csv", "*.xlsx"]
events = ["create"]

[[rules.actions]]
type = "move"
destination = "C:/data/quarantine"
overwrite = false
preserve_structure = false
verify_integrity = false
```

---

## 9. トラブルシューティング

### 10.1 起動時にエラーが出る

| エラー内容 | 対処 |
|-----------|------|
| 設定項目が不足 | 全項目を明示的に記述してください。省略やデフォルト値はありません |
| `patterns` と `regex` の同時指定 | いずれか一方にしてください |
| `watch_path` が存在しない | 監視対象ディレクトリを事前に作成してください |
| `destination` が存在しない | コピー/移動先ディレクトリを事前に作成してください |
| 循環参照が検出された | コピー先が監視対象内にないか確認してください |
| 未知のプレースホルダ | `{FullName}` 等の正しいプレースホルダ名を使用してください |

### 10.2 ファイルが検知されない

| 考えられる原因 | 対処 |
|--------------|------|
| `enabled = false` | `true` に設定してください |
| `target` が不一致 | ファイルを検知したい場合は `file` または `both`、フォルダは `directory` または `both` |
| `recursive = false` | サブディレクトリ内のファイルを検知するには `true` にしてください |
| パターンが不一致 | glob パターンや正規表現がファイル名にマッチしているか確認してください |
| `events` が不一致 | 例えば新規作成を検知するには `create` が events に含まれている必要があります |
| `include_hidden = false` | 対象が隠しファイル（Hidden 属性）の場合は除外されます。`desktop.ini` や `Thumbs.db` 等は隠し属性を持ちます |
| SMB 環境 | ネットワークドライブの場合、OS レベルで変更通知が届かない場合があります |

### 10.3 コピー/移動が失敗する

| 考えられる原因 | 対処 |
|--------------|------|
| ファイルが他プロセスにロックされている | `retry_count` と `retry_interval_ms` を増やしてください |
| ディスク容量不足 | 宛先ドライブの空き容量を確認してください |
| 権限不足 | watcher.exe の実行ユーザーに宛先への書き込み権限があるか確認してください |
| ハッシュ値が不一致（`integrity_failed`） | ディスクの健全性を確認してください。`verify_integrity = true` の場合、コピー/移動後に BLAKE3 ハッシュ値を比較します。不一致時は宛先ファイルを削除して自動リトライされます（`retry_count` 回まで）。全リトライ失敗後は不正な宛先ファイルが削除されます。move の異ボリュームフォールバック時は元ファイルは保持されます。頻発する場合はストレージ障害の可能性があります |

### 10.4 アプリケーションが突然終了する（終了コード 2）

監視対象ディレクトリが削除された場合、アプリケーションはエラー終了します。ログファイルで `watch_path_lost` イベントを確認してください。

### 10.5 設定を変更したが反映されない

設定の自動反映（ホットリロード）は行いません。設定を変更した後は **アプリケーションを再起動** してください。

### 10.6 パスの指定方法

- パス区切りは `/` を推奨します（例: `C:/data/incoming`）
- `\` も使用可能ですが、TOML ではエスケープが必要です（例: `"C:\\data\\incoming"`）
- 260 文字を超える長いパスは `\\?\C:\...` プレフィクスを使用してください
- パスにスペースを含む場合、CSV では `""` で囲んでください
