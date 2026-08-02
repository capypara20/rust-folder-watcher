
# 詳細設計書 — ファイル常駐監視アプリケーション

| 項目 | 内容 |
|------|------|
| 文書版数 | 1.1 |
| 作成日 | 2025-07 |
| 関連文書 | [requirements.md](requirements.md)（要件定義書）/ [design.md](design.md)（設計メモ） |

---

## 目次

1. [システム構成](#1-システム構成)
2. [アーキテクチャ](#2-アーキテクチャ)
3. [設定ファイル仕様](#3-設定ファイル仕様)
4. [CLI 仕様](#4-cli-仕様)
5. [起動・終了シーケンス](#5-起動終了シーケンス)
6. [ファイル監視](#6-ファイル監視)
7. [デバウンス](#7-デバウンス)
8. [ルール評価](#8-ルール評価)
9. [プレースホルダ](#9-プレースホルダ)
10. [アクション実行](#10-アクション実行)
11. [エラー処理](#11-エラー処理)
12. [ログ仕様](#12-ログ仕様)
13. [安全性対策](#13-安全性対策)
14. [Windows 固有の考慮事項](#14-windows-固有の考慮事項)
15. [CSV→TOML 変換ツール（csv2toml）](#15-csvtoml-変換ツールcsv2toml)
16. [モジュール構成](#16-モジュール構成)
17. [使用クレート](#17-使用クレート)

---

## 1. システム構成

### 1.1 成果物

| 成果物 | 説明 |
|--------|------|
| `cat-watcher.exe` | ファイル監視アプリケーション本体 |
| `csv2toml.exe` | CSV→TOML 変換ツール |

両ツールは同一 Cargo ワークスペースで管理する。

### 1.2 リポジトリ構成

```
Cargo.toml              # ワークスペース定義
cat-watcher/                # 監視アプリ本体
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs
│       ├── config.rs
│       ├── watcher.rs
│       ├── router.rs
│       ├── actions/
│       │   ├── mod.rs
│       │   ├── copy.rs
│       │   ├── move_file.rs
│       │   ├── command.rs
│       │   └── execute.rs
│       ├── placeholder.rs
│       └── error.rs
csv2toml/               # CSV→TOML 変換ツール
│   ├── Cargo.toml
│   └── src/
config/                 # 設定ファイルサンプル
│   ├── global.toml
│   ├── rules.toml
│   └── rules.csv
doc/                    # ドキュメント
tool/                   # 補助スクリプト
```

---

## 2. アーキテクチャ

### 2.1 処理フロー

```
global.toml + rules.toml
    │
    ▼
Config Loader（バリデーション含む）
    │
    ▼
Watcher（notify クレート / ReadDirectoryChangesW）
    │ ファイルシステムイベント
    ▼
Event Router
    │  デバウンス（500ms）
    │  → target フィルタ
    │  → patterns / regex マッチ
    │  → events フィルタ
    ▼
Action Executor（tokio 非同期）
    │  copy | move | command | execute
    │  アクションチェーン（順次実行）
    ▼
Logger（tracing / JSON 構造化ログ → ファイル出力）
```

### 2.2 非同期モデル

- ランタイム: `tokio`（マルチスレッド）
- 監視スレッドをブロックしないよう、アクション実行は `tokio::spawn` で切り離す
- `copy` / `move` は完了を待つ（await）。`command` / `execute` は fire-and-forget（spawn して即座に返す）

---

## 3. 設定ファイル仕様

### 3.1 構成

| ファイル | 用途 | 管理方法 |
|---------|------|---------|
| `global.toml` | グローバル設定（ログ、リトライ等） | 手動編集 |
| `rules.toml` | 監視ルール定義 | csv2toml 変換ツールで生成（手動編集も可） |

**設計方針**:
- すべての設定項目にデフォルト値を持たない。省略した場合はバリデーションエラー
- ホットリロードは行わない。設定変更時はアプリ再起動
- 相対パスは CWD（カレントワーキングディレクトリ）基準

### 3.2 `global.toml`

```toml
[global]
log_level = "info"
log_file = "./logs/watcher.log"
log_rotation = "daily"
retry_count = 3
retry_interval_ms = 1000
dry_run = false
```

| キー | 型 | 説明 |
|------|-----|------|
| `log_level` | string | `trace` / `debug` / `info` / `warn` / `error` |
| `log_file` | string | ログファイルパス |
| `log_rotation` | string | `daily`（日次ローテーション）/ `never`（ローテーションなし） |
| `retry_count` | u32 | ファイル I/O エラー時のリトライ回数 |
| `retry_interval_ms` | u64 | リトライ間隔（ミリ秒） |
| `dry_run` | bool | `true` でアクションを実行せずログのみ出力 |

### 3.3 `rules.toml`

```toml
[[rules]]
name = "csv-backup"
enabled = true

[rules.watch]
path = "C:/data/incoming"
recursive = true
target = "file"
include_hidden = false
patterns = ["*.csv", "report_*.xlsx"]
events = ["create", "modify"]

[[rules.actions]]
type = "copy"
destination = "C:/data/backup"
overwrite = false
preserve_structure = true
verify_integrity = true
```

#### ルール設定項目

| セクション | キー | 型 | 必須 | 説明 |
|-----------|------|-----|------|------|
| `rules[]` | `name` | string | ○ | ルール名（識別用） |
| `rules[]` | `enabled` | bool | ○ | ルールの有効/無効 |
| `rules[].watch` | `path` | string | ○ | 監視対象ディレクトリパス |
| `rules[].watch` | `recursive` | bool | ○ | サブディレクトリの再帰監視 |
| `rules[].watch` | `target` | string | ○ | 検知対象: `file` / `directory` / `both` |
| `rules[].watch` | `include_hidden` | bool | ○ | 隠しファイル・隠しフォルダを検知対象に含めるか。`true`: 含める / `false`: 除外。Windows は `FILE_ATTRIBUTE_HIDDEN` 属性、Linux ほかはファイル名の `.` 始まりで判定 |
| `rules[].watch` | `patterns` | string[] | ※ | glob パターン（`regex` と排他） |
| `rules[].watch` | `exclude_patterns` | string[] | | 除外 glob パターン |
| `rules[].watch` | `regex` | string | ※ | 正規表現（`patterns` と排他） |
| `rules[].watch` | `events` | string[] | ○ | 検知イベント: `create` / `modify` / `delete` / `rename` |
| `rules[].actions[]` | `type` | string | ○ | `copy` / `move` / `command` / `execute` |
| `rules[].actions[]` | `destination` | string | copy/move 時 ○ | コピー/移動先ディレクトリ |
| `rules[].actions[]` | `overwrite` | bool | copy/move 時 ○ | 同名ファイル存在時に上書きするか |
| `rules[].actions[]` | `preserve_structure` | bool | copy/move 時 ○ | サブディレクトリ構造を維持するか |
| `rules[].actions[]` | `verify_integrity` | bool | copy/move 時 ○ | コピー/移動後に BLAKE3 ハッシュ値比較で完全性を検証するか |
| `rules[].actions[]` | `shell` | string | command 時 ○ | シェル種別: `cmd` / `powershell` / `pwsh` |
| `rules[].actions[]` | `command` | string | command 時 ○ | 実行するコマンド文字列 |
| `rules[].actions[]` | `program` | string | execute 時 ○ | 実行ファイルの絶対パス |
| `rules[].actions[]` | `args` | string[] | execute 時 ○ | 引数リスト |
| `rules[].actions[]` | `working_dir` | string | | command / execute のカレントディレクトリ |
| `rules[].actions[]` | `auto_create` | bool | | copy/move の宛先フォルダを自動作成するか。省略時は `global.toml` の `[destination] auto_create` に従う |
| `rules[].actions[]` | `delay_ms` | 整数 | | このアクションを実行する前に待つミリ秒。省略時 0 |

※ `patterns` と `regex` はいずれか一方を必ず指定する（両方指定・両方省略はエラー）。

#### グローバル設定の追加セクション

| セクション | キー | 型 | 既定 | 説明 |
|-----------|------|-----|------|------|
| `[startup_scan]` | `enabled` | bool | `true` | 起動直後に監視フォルダを 1 度走査し、既存エントリを create として流す |
| `[destination]` | `auto_create` | bool | `true` | copy/move の宛先フォルダを自動作成するか（全ルール共通の既定値） |
| `[detect]` | `debounce_ms` | 整数 | `500` | 最後のイベントからこの時間静かになったら確定とみなす |
| `[detect]` | `poll_interval_ms` | 整数 | `100` | 確定済みを確認する間隔。1 以上必須（0 はバリデーションエラー） |
| `[service]` | `run_as_logged_in_user` | bool | `true` | サービス起動時、外部プロセスをログオンユーザー権限で実行するか |

#### `auto_create` とロード時バリデーションの関係

実行時は宛先フォルダを `create_dir_all` で自動作成するため、ロード時の存在チェックは
`auto_create` に合わせて強さを変える。

| `auto_create` | ロード時のチェック | 実行時の挙動 |
|---|---|---|
| `true`（既定） | 静的部分の先祖をたどって**1 つも実在しない場合のみ**エラー（未接続のドライブレター・綴り違いの共有名を検出） | 無ければ再帰的に作成 |
| `false` | 静的部分が実在するディレクトリであることを必須にする | 無ければアクション失敗 |

`destination` が `{WatchPath}/out` のように先頭からプレースホルダーで始まる場合は、
静的部分が無いためロード時の判定をスキップする。

#### `target` × `recursive` の挙動

| | `recursive = false` | `recursive = true` |
|---|---|---|
| `target = "file"` | 直下のファイルのみ | 全階層のファイル |
| `target = "directory"` | 直下のフォルダのみ | 全階層のフォルダ |
| `target = "both"` | 直下のファイル＋フォルダ | 全階層のファイル＋フォルダ |

`include_hidden` フィルタは `target` フィルタの直後に適用される。`patterns` / `exclude_patterns` / `regex` はその後に適用される。

> フィルタ適用順: **target** → **include_hidden** → **patterns / exclude_patterns / regex**

#### `preserve_structure` の挙動

`recursive = true` かつ `copy` / `move` 時に有効。

- `true`: 監視ルートからの相対パス構造を宛先に再現する。中間ディレクトリは自動作成
- `false`: 検知ファイルを宛先直下にフラット配置する

`recursive = false` の場合、または `command` / `execute` アクションでは無効。

### 3.4 バリデーション

起動時に以下をすべてチェックし、不正な場合はエラー終了（終了コード `1`）する。

| チェック項目 | 説明 |
|-------------|------|
| 必須項目の存在 | 全項目が明示的に指定されていること |
| `patterns` / `regex` 排他 | 両方指定・両方省略はエラー |
| `events` 空チェック | 空配列はエラー |
| `actions` 空チェック | 空配列はエラー |
| `type` ごとの必須フィールド | `copy`/`move` → `destination`, `overwrite`, `preserve_structure`, `verify_integrity`。`command` → `shell`, `command`。`execute` → `program`, `args` |
| `watch.path` 存在確認 | 指定パスが実在すること |
| `destination` 存在確認 | copy/move 時、宛先が実在すること（`preserve_structure = true` の場合はルートのみ） |
| 循環参照チェック | §13.1 参照 |
| 同名ルールの watch 設定一致 | 同一 `name` のルールは watch 設定が完全に一致すること |
| プレースホルダ検証 | 未知のプレースホルダが含まれていないこと |
| glob / regex 構文チェック | コンパイルが成功すること |

---

## 4. CLI 仕様

### 4.1 watcher

```
watcher.exe --global <path> --rules <path> [OPTIONS]
```

| オプション | 短縮 | 必須 | 説明 |
|-----------|------|------|------|
| `--global <path>` | `-g` | ○ | `global.toml` のパス |
| `--rules <path>` | `-r` | ○ | `rules.toml` のパス |
| `--dry-run` | | | ドライランモード（`global.toml` 設定を上書き） |
| `--log-level <level>` | | | ログレベル上書き |
| `--validate` | | | バリデーション専用モード（監視は開始しない） |
| `--version` | `-V` | | バージョン表示 |
| `--help` | `-h` | | ヘルプ表示 |

#### 終了コード

| コード | 意味 |
|--------|------|
| `0` | 正常終了（グレースフルシャットダウン / `--validate` 成功） |
| `1` | 設定ファイルエラー（パース失敗、バリデーションエラー） |
| `2` | 実行時致命的エラー（監視ディレクトリ消失等） |

### 4.2 csv2toml

```
csv2toml.exe --input <path> --output <path> [OPTIONS]
```

| オプション | 短縮 | 必須 | 説明 |
|-----------|------|------|------|
| `--input <path>` | `-i` | ○ | 入力 CSV ファイルパス |
| `--output <path>` | `-o` | ○ | 出力 TOML ファイルパス |
| `--validate` | | | バリデーション専用モード |
| `--dry-run` | | | 変換結果を stdout に出力（ファイル書き出しなし） |
| `--version` | `-V` | | バージョン表示 |
| `--help` | `-h` | | ヘルプ表示 |

#### 終了コード

| コード | 意味 |
|--------|------|
| `0` | 正常終了 |
| `1` | エラー（CSV パース失敗、バリデーションエラー） |

---

## 5. 起動・終了シーケンス

### 5.1 起動シーケンス

```
 1. CLI 引数パース
 2. global.toml 読み込み・デシリアライズ
 3. rules.toml 読み込み・デシリアライズ
 4. バリデーション（§3.4 の全チェック、循環参照含む）
 5. ロガー初期化（JSON ファイル出力）
 6. glob / regex パターンのコンパイル（起動時 1 回のみ）
 7. 監視対象ディレクトリの存在確認
 8. 既存ファイルの初回スキャン・ルール評価（§5.3）
 9. Watcher 起動（notify クレート）
10. イベントループ開始
```

ステップ 1〜7 のいずれかでエラーが発生した場合、ログを出力して終了コード `1` で即座に終了する。

`--validate` が指定されている場合、ステップ 7 の後に「バリデーション成功」をログに出力し、終了コード `0` で終了する。

### 5.2 グレースフルシャットダウン

Ctrl+C（`SIGINT`）を受信した場合の動作:

1. **新規イベント受付停止** — Watcher からの新規イベント処理を停止する
2. **実行中アクションの待機** — `copy` / `move`: 完了を待つ。`command` / `execute`: fire-and-forget のため待機しない（起動済みプロセスは放置）
3. **ログのフラッシュ** — バッファ内のログをすべて書き出す
4. **終了** — 終了コード `0` で終了する

### 5.3 既存ファイルの初回スキャン

起動時（ステップ 8）に、監視対象ディレクトリ内の既存ファイル/フォルダに対してルール評価を実行する。

- 過去の処理済みファイルの記録は保持しない（**ステートレス**）
- 再起動のたびに既存ファイルが再処理される

### 5.4 設定変更の反映

**ホットリロードは実装しない**。設定変更時はアプリケーションを再起動する。

理由:
- 設定変更は頻繁に行う操作ではない
- Watcher の差し替えや実行中アクションとの整合管理が複雑になる
- ステートレス再起動（§5.3）で既存ファイルが再処理されるため実運用上の問題が少ない

---

## 6. ファイル監視

### 6.1 監視メカニズム

- `notify` クレートを使用し、Windows の `ReadDirectoryChangesW` API によりファイルシステムイベントを検知する
- ディレクトリ単位でハンドルを取得する方式のため、再帰監視でもハンドル消費は少ない

### 6.2 Watcher の統合

同一 `watch.path` を監視する複数ルールが存在する場合、OS レベルの Watcher は **1 つにまとめる**。イベント発生後のルール評価で各ルールへ振り分ける。

### 6.3 イベント種別

| イベント | 説明 |
|---------|------|
| `create` | ファイル/フォルダの新規作成 |
| `modify` | ファイルの内容変更（ディレクトリの modify は OS 依存で不安定なため非保証） |
| `delete` | ファイル/フォルダの削除 |
| `rename` | ファイル/フォルダのリネーム/移動 |

### 6.4 OS バッファ溢れ

`notify` は OS バッファが溢れた場合に `Rescan` イベントを発行する。これを受信したら対象ディレクトリをフルスキャンし、差分を検出する。

---

## 7. デバウンス

### 7.1 目的

Windows の `ReadDirectoryChangesW` は 1 つのファイル操作に対して複数のイベント（例: `Create` → `Modify` → `Modify`）を発火することがある。デバウンスにより、これらを 1 回のアクション実行にまとめる。

### 7.2 仕様

| 項目 | 内容 |
|------|------|
| デバウンス窓 | **500ms**（コード内定数、ユーザー設定なし） |
| 集約キー | ファイルパス単位 |
| 集約内容 | デバウンス窓内に発生したイベント種別の**集合** |

### 7.3 判定ロジック

```
時間軸:  |————— 500ms —————|
OS 通知:  Create  Modify  Modify
集約結果: {Create, Modify}
```

集約結果とルールの `events` 設定の**積集合**が空でなければマッチとみなし、**1 回だけ**アクションを実行する。

```
集約結果:      {Create, Modify}
events 設定:   ["create"]
積集合:        {Create}  ≠ ∅  → マッチ → アクション 1 回実行
```

---

## 8. ルール評価

### 8.1 評価順序

TOML ファイル内の定義順（上から順）に評価する。同一ルール名の複数行定義に依存する順序はない（同名ルールは watch 設定が一致している必要がある）。

### 8.2 複数ルールマッチ

1 つのファイルが複数ルールにマッチした場合、**すべてのマッチルール**のアクションを実行する（先勝ち・排他ではない）。

### 8.3 評価パイプライン

```
イベント発生
  → デバウンス集約（§7）
  → target フィルタ（file / directory / both）
  → include_hidden フィルタ（§14.5）
  → patterns / exclude_patterns / regex マッチ
  → events 積集合判定
  → マッチ → アクションチェーン実行
```

### 8.4 rename イベント

rename イベントでは、**リネーム後のファイル名**に対してパターンマッチを行う。

---

## 9. プレースホルダ

### 9.1 一覧

PowerShell / .NET の `FileInfo` プロパティ名に準拠した命名を採用する。

検知ファイル `C:/data/sub/report_2026.csv`（watch_path: `C:/data`）の場合:

#### ファイル情報プレースホルダ

| プレースホルダ | 値の例 | 説明 |
|---------------|--------|------|
| `{FullName}` | `C:/data/sub/report_2026.csv` | 正規化済み絶対パス |
| `{DirectoryName}` | `C:/data/sub` | 親ディレクトリの絶対パス |
| `{Name}` | `report_2026.csv` | ファイル名（拡張子付き） |
| `{BaseName}` | `report_2026` | ファイル名（拡張子なし） |
| `{Extension}` | `csv` | 拡張子（**ドットなし**） |

> 拡張子なしファイルの場合、`{Extension}` は空文字列。ドット付きで使いたい場合は `.{Extension}` と記述する。

#### コンテキストプレースホルダ

| プレースホルダ | 値の例 | 説明 |
|---------------|--------|------|
| `{RelativePath}` | `sub/report_2026.csv` | watch_path からの相対パス |
| `{WatchPath}` | `C:/data` | ルールの監視パス |
| `{Destination}` | `C:/backup/sub/report_2026.csv` | 直前の copy/move 先の絶対パス。直前に copy/move がない場合は空文字列 |

#### 日時プレースホルダ

| プレースホルダ | 値の例 | 説明 |
|---------------|--------|------|
| `{Date}` | `20260328` | アクション実行時の日付（`YYYYMMDD`） |
| `{Time}` | `143052` | アクション実行時の時刻（`HHmmss`） |
| `{DateTime}` | `20260328_143052` | 日時（`YYYYMMDD_HHmmss`） |

### 9.2 エスケープ

リテラルの `{` `}` を出力する場合は `{{` `}}` と記述する。

```
command = "echo {{result}}: {Name}"
→ echo {result}: report_2026.csv
```

### 9.3 未知のプレースホルダ

定義されていないプレースホルダ（例: `{Unknown}`）が含まれる場合、**起動時バリデーションでエラー**とする。

### 9.4 アクションチェーン時の更新ルール

| 前アクション | 後続の `{FullName}` 等 | `{Destination}` |
|-------------|----------------------|-----------------|
| `copy` | **変更なし**（ソースファイルのパス） | コピー先の絶対パス |
| `move` | **移動後のパスに更新** | 移動先の絶対パス |
| `command` / `execute` | 変更なし | 変更なし |

`move` は元ファイルが存在しなくなるため、後続アクションのパス系プレースホルダを移動後のパスに更新する。

---

## 10. アクション実行

### 10.1 copy（コピー）

| 項目 | 仕様 |
|------|------|
| 完了待ち | あり。コピー完了まで待機してから次のアクションチェーンに進む |
| 書き込み方式 | 直接書き込み（一時ファイル経由のアトミック操作は行わない） |
| フォルダコピー | `target = "directory"` の場合、フォルダの中身ごとすべて再帰コピー |
| 異ボリューム | OS がコピーを処理するため問題なし |
| destination 不在 | 起動時バリデーションでエラー。ただし `preserve_structure = true` の場合はルートディレクトリが存在すれば OK（中間ディレクトリは自動作成） |
| リトライ | 「コピー＋完全性検証」を 1 つのリトライ単位とする。I/O エラー・ハッシュ不一致のいずれも同じ `retry_count` を消費する。不一致時は宛先ファイルを削除してからリトライする（`overwrite = false` でもリトライ可能にするため）。リトライごとに WARN ログ、最終失敗時に ERROR ログを出力 |
| 完全性検証 | `verify_integrity = true` の場合、コピー後にソースと宛先の BLAKE3 ハッシュ値を比較する。最終的にハッシュ不一致で失敗した場合は不正な宛先ファイルを削除する |

### 10.2 move（移動）

| 項目 | 仕様 |
|------|------|
| 完了待ち | あり |
| 書き込み方式 | 直接移動 |
| フォルダ移動 | フォルダの中身ごとすべて移動 |
| 異ボリューム | `rename` API がエラーの場合、内部で `copy → 元ファイル削除` にフォールバック |
| destination 不在 | copy と同様 |
| リトライ | 異ボリュームフォールバック時は「コピー＋完全性検証」を 1 つのリトライ単位とする（copy と同様）。ハッシュ不一致時は宛先ファイルを削除してからリトライする |
| 完全性検証 | `verify_integrity = true` の場合、同一ボリューム `rename` ではスキップ（データ転送なし）。異ボリュームフォールバック時は `copy` 後・**元ファイル削除前**に BLAKE3 ハッシュ値を比較する。最終的にハッシュ不一致で失敗した場合は不正な宛先ファイルを削除し、元ファイルは保持する（データ消失防止） |

### 10.3 command（コマンド実行）

| 項目 | 仕様 |
|------|------|
| 完了待ち | **なし**（fire-and-forget）。プロセス起動後に制御を返す |
| シェル | `shell` フィールドに従い実行 |
| タイムアウト | なし |
| stdout / stderr | ログに記録しない |
| 終了コード | 判定しない |
| `working_dir` | 指定された場合はカレントディレクトリとして設定 |

#### `shell` 設定によるコマンド起動方式

| shell | 起動コマンド |
|-------|------------|
| `cmd` | `cmd.exe /C <command>` |
| `powershell` | `powershell.exe -NoProfile -Command <command>` |
| `pwsh` | `pwsh.exe -NoProfile -Command <command>` |

### 10.4 execute（外部プロセス起動）

| 項目 | 仕様 |
|------|------|
| 完了待ち | **なし**（fire-and-forget） |
| 起動方式 | `CreateProcessW` で直接起動（シェルを介さない） |
| タイムアウト | なし |
| stdout / stderr | ログに記録しない |
| 終了コード | 判定しない |

### 10.5 アクションチェーン

- 1 ルールに複数アクションが定義されている場合、**TOML 定義順に順次実行**する
- `copy` / `move` は完了を待ってから次のアクションに進む
- `command` / `execute` は起動後すぐに次のアクションに進む
- チェーン中のアクションが**エラーになった場合、後続アクションを中断**する

### 10.6 リトライ

| 項目 | 仕様 |
|------|------|
| 対象 | ファイル I/O エラーおよびハッシュ不一致（`verify_integrity = true` 時） |
| 設定 | `global.toml` の `retry_count` / `retry_interval_ms` |
| 適用範囲 | `copy` / `move` のファイル操作時。「コピー（または移動）＋完全性検証」を **1 つの操作単位**としてリトライする。I/O エラーとハッシュ不一致は同じカウンタを消費する |
| 適用外 | `command` / `execute`（fire-and-forget）。設定エラー等の致命的エラー |
| 不一致時の処理 | ハッシュ不一致でリトライする前に宛先ファイルを削除する（`overwrite = false` でもリトライ可能にするため） |
| ログレベル | リトライ中は WARN、最終失敗時は ERROR |
| 最終失敗時 | 不正な宛先ファイルが残っている場合は削除する。move の異ボリュームフォールバック時は元ファイルを保持する |

### 10.7 完全性検証（BLAKE3 ハッシュ比較）

`verify_integrity = true` が設定された `copy` / `move` アクションにおいて、ファイル転送後にソースと宛先の BLAKE3 ハッシュ値を比較し、データの完全性を検証する。

#### アルゴリズム選定理由

| 項目 | BLAKE3 | MD5 | SHA-256 |
|------|--------|-----|---------|
| 速度 | ★★★（SIMD/マルチスレッド対応。数 GB/s） | ★★ | ★ |
| 安全性 | 暗号学的ハッシュ | 衝突脆弱性あり | 安全 |
| 出力サイズ | 256bit (32 bytes) | 128bit | 256bit |
| 大容量ファイル | 内部 Merkle tree 構造で高速 | 線形 | 線形 |

大容量ファイル（数 GB 以上）でも高速にハッシュ計算が可能な BLAKE3 を採用する。

#### 検証フロー

**copy の場合（verify_integrity = true）:**
```
retry_remaining = retry_count
loop:
  1. ソースファイルを宛先にコピー
     → I/O エラー発生時は手順 6 へ
  2. ソースファイルの BLAKE3 ハッシュを計算
  3. 宛先ファイルの BLAKE3 ハッシュを計算
  4. 一致 → integrity_verified ログ（INFO）→ 成功
  5. 不一致 → integrity_failed ログ → 宛先ファイルを削除
  6. retry_remaining > 0 → WARN ログ（action_retry）
     → retry_interval_ms 待機 → retry_remaining -= 1 → loop へ
  7. retry_remaining == 0 → ERROR ログ（action_failed）
     → 不正な宛先ファイルが残っていれば削除 → アクションエラー
```

**move（異ボリュームフォールバック・verify_integrity = true）の場合:**
```
1. rename 失敗 → copy → delete フォールバック開始
retry_remaining = retry_count
loop:
  2. ソースファイルを宛先にコピー
     → I/O エラー発生時は手順 7 へ
  3. ソースファイルの BLAKE3 ハッシュを計算
  4. 宛先ファイルの BLAKE3 ハッシュを計算
  5. 一致 → integrity_verified ログ（INFO）→ 元ファイルを削除 → 成功
  6. 不一致 → integrity_failed ログ → 宛先ファイルを削除
  7. retry_remaining > 0 → WARN ログ（action_retry）
     → retry_interval_ms 待機 → retry_remaining -= 1 → loop へ
  8. retry_remaining == 0 → ERROR ログ（action_failed）
     → 不正な宛先ファイルが残っていれば削除
     → 元ファイルは保持（データ消失防止）→ アクションエラー
```

**move（同一ボリューム rename）の場合:**
- `rename` はメタデータ変更のみでデータ転送が発生しないため、**ハッシュ検証をスキップ**する

#### フォルダの検証

`target = "directory"` の場合、フォルダ内の各ファイルを個別に BLAKE3 ハッシュ検証する。1 ファイルでも不一致があればアクションエラーとする。

#### ハッシュ計算の実装方針

- `blake3` クレートの `Hasher` を使用し、ストリーミング（チャンク単位）でハッシュを計算する
- バッファサイズはデフォルト 64 KiB（`blake3` クレートの推奨値に準拠）
- `dry_run = true` の場合はハッシュ検証をスキップする

---

## 11. エラー処理

### 11.1 エラー分類

| 分類 | 例 | 動作 |
|------|-----|------|
| **致命的エラー** | 設定パース失敗、バリデーションエラー | ログ出力 → 終了コード `1` で終了 |
| **致命的エラー（実行時）** | watch_path 消失 | ログ出力 → 終了コード `2` で終了 |
| **回復可能エラー** | ファイルロック、一時的 I/O エラー、ハッシュ不一致 | リトライ → 全リトライ失敗時はログ出力してスキップ |
| **アクションエラー** | destination 書き込み失敗、プロセス起動失敗 | ログ出力 → アクションチェーン中断 → 次のイベント処理へ |

### 11.2 watch_path 消失

監視中に `watch_path` が削除された場合:
- エラーログを記録
- 終了コード `2` でアプリケーションを終了

### 11.3 エラー実装方針

- `anyhow` または独自 `error.rs` でエラー型を統一
- `unwrap()` は原則禁止（テストコードを除く）

---

## 12. ログ仕様

### 12.1 フォーマット

JSON 構造化ログ。1 行 1 JSON オブジェクト。

```json
{"timestamp":"2026-03-28T14:30:52.123+09:00","level":"INFO","event":"file_detected","rule":"csv-backup","event_type":"create","file_path":"C:/data/incoming/report.csv","target":"file"}
{"timestamp":"2026-03-28T14:30:52.456+09:00","level":"INFO","event":"action_started","rule":"csv-backup","action_type":"copy","source":"C:/data/incoming/report.csv","destination":"C:/data/backup/report.csv"}
{"timestamp":"2026-03-28T14:30:52.789+09:00","level":"INFO","event":"action_completed","rule":"csv-backup","action_type":"copy","source":"C:/data/incoming/report.csv","destination":"C:/data/backup/report.csv","duration_ms":333}
```

### 12.2 フィールド一覧

| フィールド | 型 | 常に出力 | 説明 |
|-----------|-----|---------|------|
| `timestamp` | string | ○ | ISO 8601（タイムゾーン付き） |
| `level` | string | ○ | `TRACE` / `DEBUG` / `INFO` / `WARN` / `ERROR` |
| `event` | string | ○ | イベント識別子（下表参照） |
| `rule` | string | | ルール名 |
| `event_type` | string | | `create` / `modify` / `delete` / `rename` |
| `file_path` | string | | 検知ファイルパス |
| `target` | string | | `file` / `directory` |
| `action_type` | string | | `copy` / `move` / `command` / `execute` |
| `source` | string | | コピー/移動元パス |
| `destination` | string | | コピー/移動先パス |
| `error` | string | | エラーメッセージ |
| `retry` | u32 | | リトライ回数 |
| `duration_ms` | u64 | | 処理時間（ミリ秒） |
| `source_hash` | string | | ソースファイルの BLAKE3 ハッシュ値（完全性検証時） |
| `dest_hash` | string | | 宛先ファイルの BLAKE3 ハッシュ値（完全性検証時） |

### 12.3 イベント識別子

| event 値 | 説明 |
|----------|------|
| `app_started` | アプリケーション起動 |
| `app_shutdown` | アプリケーション終了 |
| `file_detected` | ファイル/フォルダ検知 |
| `action_started` | アクション実行開始 |
| `action_completed` | アクション正常完了 |
| `action_failed` | アクション失敗 |
| `action_retry` | アクションリトライ |
| `integrity_verified` | ハッシュ値比較による完全性検証成功 |
| `integrity_failed` | ハッシュ値比較による完全性検証失敗 |
| `chain_aborted` | アクションチェーン中断 |
| `validation_error` | 設定バリデーションエラー |
| `watch_path_lost` | 監視ディレクトリ消失 |

### 12.4 ローテーション

| `log_rotation` | 挙動 | ファイル名例 |
|----------------|------|-------------|
| `daily` | 日次ローテーション | `watcher.2026-03-28.log` |
| `never` | ローテーションなし | `watcher.log` |

### 12.5 出力先

- **ファイル出力のみ**（`global.toml` の `log_file` パス）
- コンソール出力は行わない

---

## 13. 安全性対策

### 13.1 循環参照検知

起動時のバリデーションで以下の 4 パターンをすべてチェックする。いずれかに該当する場合はエラー終了。

| パターン | 内容 |
|---------|------|
| 1 | `destination` == `watch_path`（同一ディレクトリ） |
| 2 | `destination` が `watch_path` の配下 かつ `recursive = true` |
| 3 | `watch_path` が `destination` の配下 |
| 4 | ルール間の相互参照（全ルールの watch_path → destination ペアでグラフを構築し、循環を検出） |

**例（パターン 4）:**
```
ルール A: watch C:/data/incoming → copy to C:/data/processed
ルール B: watch C:/data/processed → copy to C:/data/incoming
→ A → B → A → ... の無限ループ → エラー
```

### 13.2 シンボリックリンク

**追跡しない**。`notify` クレートのデフォルト動作に従い、シンボリックリンクの先は監視対象外。

### 13.3 パストラバーサル防止

コピー/移動先のパスを `canonicalize` で正規化し、意図しないディレクトリへのアクセスを防止する。

### 13.4 コマンドインジェクション

**サニタイズは実装しない**。

- `command` タイプはシェル経由で実行するため、ファイル名中の特殊文字（`&`, `;`, `$()` 等）がシェルコマンドとして解釈されるリスクがある
- 設定ファイル自体に任意コマンドを記述できるため、セキュリティ境界は設定ファイルの権限管理にある
- **運用上の対策**: 設定ファイルの NTFS 権限を適切に管理する。外部ユーザーがファイル名を制御できる環境では `execute` タイプ（シェル非経由）の使用を推奨する

---

## 14. Windows 固有の考慮事項

### 14.1 パス

- TOML 内のパスは `/` 区切りを推奨（Rust の `std::path::Path` は `/` を受け付ける）
- 長いパス（260 文字超）は `\\?\` プレフィクスで対処
- UNC パス（`\\server\share`）対応。SMB で OS 通知が届かない環境は動作保証外

### 14.2 ファイルロック

- 他プロセスが書き込み中のファイルは開けない（`SHARING_VIOLATION`）
- リトライ機構（`retry_count` / `retry_interval_ms`）で対処

### 14.3 文字コード

- ファイルシステム（UTF-16）↔ Rust（UTF-8）間は `OsString` で適切に変換
- CSV: BOM 付き UTF-8 を想定
- ログ出力: UTF-8

### 14.4 プロセス起動

- `command` タイプ: `shell` 設定に応じて `cmd.exe`、`powershell.exe`、`pwsh.exe` 経由
- `execute` タイプ: `CreateProcessW` 直接起動
- 環境変数展開（`%USERPROFILE%` 等）はシェル経由時のみ有効

### 14.5 隠しファイル・システムファイル

- `ReadDirectoryChangesW` は隠しファイル（Hidden 属性）やシステムファイル（System 属性）を区別せず、すべてのファイル変更を通知する。API レベルでの除外機能はない
- `include_hidden = false` の場合、イベント受信後に `GetFileAttributesW` でファイル属性を確認し、`FILE_ATTRIBUTE_HIDDEN` が付与されたファイル・フォルダを除外する
- `include_hidden = true` の場合はフィルタリングせず、OS から通知された全イベントを処理する
- Windows がエクスプローラー操作時に自動生成する `desktop.ini`、`Thumbs.db` 等は隠し属性を持つため、`include_hidden = false` で自動的に除外される
- **Linux ほか**では隠し属性という概念が無いため、ファイル名が `.` で始まるか（ドットファイル慣習）で判定する
- Windows では `.env` のようなドット始まりのファイルでも隠し属性が無ければ除外されない。名前で除外したい場合は `exclude_regex = "^\\."` を使う

#### 属性判定の詳細ルール

| 項目 | 仕様 |
|------|------|
| **判定対象属性** | `FILE_ATTRIBUTE_HIDDEN` のみ。`FILE_ATTRIBUTE_SYSTEM` は判定しない |
| **属性取得失敗時** | delete 等でファイルが既に存在せず `GetFileAttributesW` が失敗した場合、属性判定をスキップし**処理対象とする**（安全側に倒す） |
| **隠しフォルダ配下のファイル** | 個別ファイルの属性のみ判定する。親フォルダの Hidden 属性は遡ってチェックしない（`recursive = true` で隠しフォルダ配下に Hidden 属性を持たない通常ファイルがある場合、そのファイルは処理対象となる） |
| **rename 時** | リネーム後のパスに対して属性判定を行う |

---

## 15. CSV→TOML 変換ツール（csv2toml）

### 15.1 変換対象

| 入力 | 出力 |
|------|------|
| `rules.csv` | `rules.toml` |

`global.toml` は手動編集のため変換対象外。

### 15.2 CSV フォーマット

#### 列定義

| 列名 | 型 | 必須 | 説明 |
|------|----|------|------|
| `name` | string | ○ | ルール名 |
| `enabled` | bool | ○ | 有効/無効 |
| `watch_path` | string | ○ | 監視対象ディレクトリ |
| `recursive` | bool | ○ | サブディレクトリ監視 |
| `target` | string | ○ | `file` / `directory` / `both` |
| `include_hidden` | bool | ○ | 隠しファイル・隠しフォルダを含めるか: `true` / `false` |
| `patterns` | string | ※ | glob パターン（`\|` 区切りで複数） |
| `exclude_patterns` | string | | 除外パターン（`\|` 区切り） |
| `regex` | string | ※ | 正規表現（`patterns` と排他） |
| `events` | string | ○ | イベント種別（`\|` 区切り） |
| `action_type` | string | ○ | `copy` / `move` / `command` / `execute` |
| `action_destination` | string | copy/move 時 ○ | コピー/移動先 |
| `action_overwrite` | bool | copy/move 時 ○ | 上書き許可 |
| `action_preserve_structure` | bool | copy/move 時 ○ | 構造維持 |
| `action_verify_integrity` | bool | copy/move 時 ○ | BLAKE3 ハッシュ値比較による完全性検証 |
| `action_shell` | string | command 時 ○ | `cmd` / `powershell` / `pwsh` |
| `action_command` | string | command 時 ○ | 実行コマンド |
| `action_program` | string | execute 時 ○ | 実行ファイル |
| `action_args` | string | execute 時 ○ | 引数（`\|` 区切り） |
| `action_working_dir` | string | | 作業ディレクトリ |

※ `patterns` と `regex` はいずれか一方を必ず指定。

#### CSV ルール

- **列識別**: ヘッダ名で識別（列順序に依存しない）
- **空白処理**: 値の前後をトリム。`""` 内の空白は保持
- **BOM**: BOM 付き UTF-8 を想定
- **アクションチェーン**: 同一 `name` の複数行で表現。上から順に実行

### 15.3 バリデーション

| チェック項目 | 説明 |
|-------------|------|
| 必須項目の存在 | 列ヘッダと値の両方をチェック |
| `patterns` / `regex` 排他 | 両方指定・両方省略はエラー |
| `action_type` ごとの必須フィールド | 型に応じた必須チェック |
| `name` 重複時の watch 設定不一致 | 同名ルールの watch 設定が異なる場合はエラー |
| パスの正規化 | `\` → `/` に統一 |

---

## 16. モジュール構成

### 16.1 watcher クレート

| モジュール | 責務 |
|-----------|------|
| `main.rs` | エントリポイント、CLI パース、シグナルハンドリング、起動シーケンス制御 |
| `config.rs` | `global.toml` / `rules.toml` の読み込み・デシリアライズ・バリデーション |
| `watcher.rs` | `notify` クレートによるファイル監視のセットアップと制御 |
| `router.rs` | デバウンス、target フィルタ、パターンマッチ、events 判定、ルール振り分け |
| `actions/mod.rs` | Action トレイト定義、アクションチェーン実行制御 |
| `actions/copy.rs` | コピーアクション |
| `actions/move_file.rs` | 移動アクション |
| `actions/command.rs` | コマンド実行アクション |
| `actions/execute.rs` | 外部プロセス起動アクション |
| `placeholder.rs` | プレースホルダの解析・展開 |
| `error.rs` | エラー型定義 |

### 16.2 csv2toml クレート

| モジュール | 責務 |
|-----------|------|
| `main.rs` | エントリポイント、CLI パース、変換・バリデーション制御 |

---

## 17. 使用クレート

| 用途 | クレート |
|------|---------|
| ファイル監視 | `notify` |
| 非同期ランタイム | `tokio` |
| 設定ファイル | `toml` + `serde` |
| glob マッチ | `globset` |
| 正規表現 | `regex` |
| ロギング | `tracing` + `tracing-subscriber` + `tracing-appender` |
| CLI 引数 | `clap` |
| シグナルハンドリング | `tokio::signal` |
| CSV パース | `csv` + `serde` |
| ハッシュ（完全性検証） | `blake3` |
