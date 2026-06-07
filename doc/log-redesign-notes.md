# ログ構成 再設計メモ

> 2026-06-07 Roze と Claude の設計相談まとめ。実装前の検討段階。

## 背景・目的

- このプログラムは「1つの実行ファイルに複数ルールを定義し、一斉に稼働」させる。
- 一部ルールを有効/無効にして運用するため、ログも用途別・ルール別に分けたい。
- 現状はログが事実上1本（`cat-watcher_{DateTime}.log`）＋任意のルール別ログのみ。

## 決定方針（確定）

ログを **目的ごとに3種類** に分け、種類ごとに列構成を変える。

| 種類 | ファイル名（案） | 単位 | 列構成 | 内容 |
|------|-----------------|------|--------|------|
| ① システムログ | `system_{Date}.log` | 全体で1本 | 3列 | 起動・ルール状態一覧・ERROR/WARN・終了 |
| ② 検知ログ | `detect_{rule}_{Date}.log` | ルール別 | （CSV検討中） | 何がいつ検知されたか（全行MATCH） |
| ③ アクションログ | `action_{rule}_{Date}.log` | ルール別 | ブロック構造 | トリガー単位で処理内容を記録 |

## ① システムログ（ほぼ確定・エラー方針は更新済み）

```
timestamp(19) │ level(5) │ content
```

- 起動時にルール一覧を出す。**有効/無効＋watch_path＋events を書く（詳細表示）**。
- **アクション失敗のエラーは system に出さない**（→ action ログだけに集約・確定）。
  system に残るのは「ライフサイクル」＋「システム階層のエラー」のみ:
  - 起動バナー / ルール一覧 / 監視開始サマリ / 終了
  - 設定読込失敗・ログファイルオープン失敗など**アクション以前の異常**
- 例:
  ```
  2026-06-07 10:00:00 │ INFO  │ cat-watcher 起動  global=global.toml  rules=rules.toml
  2026-06-07 10:00:00 │ INFO  │ [ルール] csv-backup  有効  watch=C:/watch/csv  events=Create,Modify
  2026-06-07 10:00:00 │ INFO  │ [ルール] pdf-move    無効
  2026-06-07 10:00:00 │ INFO  │ 監視開始  有効ルール=2 / 全3件
  2026-06-07 18:00:00 │ INFO  │ 終了シグナル受信
  2026-06-07 18:00:00 │ INFO  │ 正常終了
  ```

## ② 検知ログ（確定）

- 列: `timestamp(19) │ events(22) │ 検知パス`（パイプ区切りの固定幅テーブル形式）
- **CSV 化は却下**。過去に試して失敗済み（events 複数値の扱いが破綻）。
  → 既存ログと同じ「罫線テーブル形式」を踏襲する。
- 例:
  ```
  2026-06-07 10:05:30 │ Create               │ C:/watch/csv/data_20260607.csv
  2026-06-07 10:31:05 │ Create,Rename        │ C:/watch/csv/sales_final.csv
  ```

## ③ アクションログ（確定）

- 「1検知イベント = 1ブロック」。先頭にセパレータ行（案1ベース）。
- **セパレータ行の並び**: `ブロック通し番号 → 時刻 → 検知フルパス → トリガーイベント → アクション数`
  ```
  ═══ #1  2026-06-07 10:05:30  C:/watch/csv/data_20260607.csv  (Create)  actions=2 ═══
  ```
  - ブロック番号 `#N` は検知イベントごとの通し番号（日次ファイル単位でリセット想定）。
- **内容行は3列**（検知パスはセパレータに移動したのでトリガー列を廃止）:
  ```
  timestamp(19) │ index＋処理種別 │ 処理内容
  ```
  - **ツリー記号 `├`/`└` は廃止**。`N. 種別`（番号＋ピリオド）形式にする。
  - **OK / ERROR の結果行も同じ番号を付ける**（`1. copy` ↔ `1. OK` でペア／案A）。
    どのアクションが成功/失敗したか追跡しやすい。
  - 結果行の表記: 成功 `1. OK` / 失敗 `1. ERR`（列幅を揃えるため ERROR は ERR に短縮）。
- 完成イメージ:
  ```
  ═══ #1  2026-06-07 10:05:30  C:/watch/csv/data_20260607.csv  (Create)  actions=2 ═══
  2026-06-07 10:05:30 │ 1. copy │ destination=C:/backup/20260607/  overwrite=true
  2026-06-07 10:05:31 │ 1. OK   │ コピー完了: C:/backup/20260607/data_20260607.csv
  2026-06-07 10:05:31 │ 2. cmd  │ shell=powershell  command=Notify.ps1
  2026-06-07 10:05:32 │ 2. OK   │ 実行完了

  ═══ #2  2026-06-07 10:31:05  C:/watch/csv/sales_final.csv  (Create,Rename)  actions=2 ═══
  2026-06-07 10:31:05 │ 1. copy │ destination=C:/backup/20260607/  overwrite=true
  2026-06-07 10:31:06 │ 1. ERR  │ 書き込み権限なし: C:/backup/20260607/
  ```

## 設定の変更点（案）

- `[rules.log]` の `log_file_name` を廃止し、種類別に分離:
  - `detect_log_file_name`
  - `action_log_file_name`
- **各ログの出力 ON/OFF はルール別に個別設定する（確定）**。例:
  - `[rules.log]` に `detect_enabled` / `action_enabled`
- global の `log_file_name` / `log_to_file` をシステムログに再利用するかは
  全体方針が固まってから決める（後方互換 or 新項目追加）。

## ログ詳細仕様（追加確定）

- **ブロック番号 `#N`**: 左詰め、**ファイルごと（＝日次ローテーション単位）に #1 から振り直す**。
- **ターミナル出力はログファイルと完全に切り離して設計する**。
  3ファイルの設計とは別問題として後で扱う（コンソールに何を流すかは別途）。
- **ログファイル名にプレフィックス規約（detect_/action_/system_）は設けない**。
  → 各ログのファイル名はユーザーが設定で自由に指定する。
- **各ログは「ON/OFF」＋「出力先ディレクトリ」＋「ファイル名」を設定で個別指定**できる。
  - システムログ: global 側で指定
  - 検知ログ / アクションログ: rules.log 側でルール別に指定

## ログレベルフィルタ（確定）

- **level 指定はシステムログのみ**に効かせる（INFO/WARN/ERROR…）。
- 検知ログ・アクションログは level なし、**ON/OFF のみ**。
  - detect は全行 MATCH なので絞る対象がない。
  - action の「ERRだけ表示」は将来 failure 系ログを足す方が筋が良い（今はやらない）。

## 設定キー名（確定）

セクションを切り、キー名を全ログで統一する。セクション名が「ログ設定」を語るので
キーの `log_` プレフィックスは外して短くする（`dir` `file_name` など）。

### global.toml — システムログは独立トップレベルセクション `[system_log]`

```toml
[global]
retry_count       = 3
retry_interval_ms = 1000

[system_log]
enabled   = true
dir       = "./log"
file_name = "system_{Date}.log"
rotation  = "daily"
level     = "info"          # level はシステムログだけ
```

### rules.toml — `[rules.log.detect]` / `[rules.log.action]`（global と同じキー構成）

```toml
[rules.log.detect]
enabled   = true
dir       = "./log"
file_name = "detect_csv-backup_{Date}.log"
rotation  = "daily"

[rules.log.action]
enabled   = true
dir       = "./log"
file_name = "action_csv-backup_{Date}.log"
rotation  = "daily"
```

### キー一覧（統一）

| キー | system_log | detect | action |
|------|:---:|:---:|:---:|
| enabled   | ✓ | ✓ | ✓ |
| dir       | ✓ | ✓ | ✓ |
| file_name | ✓ | ✓ | ✓ |
| rotation  | ✓ | ✓ | ✓ |
| level     | ✓ | ✗ | ✗ |

- ファイル名はユーザー自由指定（プレフィックス規約なし）。例の `detect_`/`action_` は単なる例。
- **注意**: これは設定フォーマットの破壊的変更。既存 global.toml/rules.toml の移行が必要
  （旧 `log_*` キー・旧 `[rules.log]` フラット構成からの移行）。テンプレートも更新対象。

## 未決事項（次回相談）

- 実装の段取り（config 構造体の変更、Logger の3系統化、テンプレート/CSV更新、移行）。
- ターミナル出力の設計（別トピック）。

### 設定ファイル構成について（決着）

- **global.toml / rules.toml の2ファイル構成は維持で確定**。
- Roze が一時「1ファイルに統合したい」と考えた理由＝2ファイル編集が面倒、というだけ。
- 2ファイルの利点（rules 単独差し替え／CSVインポートが rules 単独生成前提／
  後方互換／設定ミスの影響分離）を確認し、現状維持で納得。

## 確定事項の追記

- 検知ログ CSV 化は **却下**（過去に失敗）。罫線テーブル形式で確定。
- ログ ON/OFF は **ルール別個別設定** で確定。
- アクションログは **案1（枠線＋ブロック通し番号）** で確定。内容行3列・`N.`番号・`OK`/`ERR`。
- **アクション失敗エラーは action ログだけに記録**（system に二重出力しない）で確定。
- システムログはライフサイクル＋システム階層エラーのみ（アクションエラーは載らない）。

---

## 実装ステータス（2026-06-08 完了）

3分割ログを実装し、`cargo test`（159件）緑・Linux 実機 end-to-end 検証済み。

- `config.rs`: `[retry]` / `[system_log]` / `[rules.log.detect]` / `[rules.log.action]`。
  全構造体に `#[serde(deny_unknown_fields)]`（旧 `[global]` は起動時エラーで停止）。
- `logger.rs`: 単一 `Logger` + `LogKind`(System/Detect/Action)。
  `new_system` / `for_detect` / `for_action` の3コンストラクタ。
  ブロック連番 `block_seq` は日次ローテで #1 リセット。
- `actions/mod.rs`: `ActionSink`（system＋action 2系統ファンアウト）。
  アクション失敗は `execute_chain` が `sink.err` で集約（action ログ＋ターミナルのみ）。
- `router.rs`: `CompiledRule` に `detect_logger` / `action_logger`。
  検知→detect、ブロック開始→action、結果→ActionSink。
- ターミナル表示は無改変（System ロガーが全種を受けるがファイルには
  Info/Warn/Error のみ書く）。

検証結果（Linux）:
- system ログ＝ライフサイクルのみ／detect ログ＝3列／action ログ＝ブロック構造。
- コピー失敗時 `1. WARN`×N→`1. ERR` が **action ログにのみ** 記録され、
  system ログには出ず、ターミナルには ERROR が表示されることを確認。

**残: Win11 実機検証（パス区切り `\`、サービスモード console=false、PowerShell/cmd 失敗）。**
