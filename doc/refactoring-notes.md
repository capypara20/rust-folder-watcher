# リファクタリング作業メモ（v2.0.0 以降）

> セッションが圧縮されても文脈を失わないための作業ログ。進行に合わせて更新する。

## 現在地（2026-09-12）

- ベースライン: `main` / `cat-watcher` v2.0.0 / **テスト 114 件すべてパス**
- フェーズ: **第1段リファクタ 完了**（テスト 117 件パス）
- GitHub Issue は未取得（`gh` 未認証）

## 決定事項

| 項目 | 決定 | 理由 |
|---|---|---|
| 進め方 | CLAUDE.md → エージェント定義 → 実作業 | 共通の前提を先に明文化すると各エージェントの指示が短く済む |
| 並列度 | **調査・レビューのみ並列。コード編集は逐次** | 同一ファイルの同時編集はコンフリクトを生み、逐次より遅くなる |
| `.claude/` | **コミットしない**（従来通り） | `.gitignore` の既存方針を維持 |
| 検証スキル | ネイティブ Win11 前提に全面書き換え済み | 旧環境（Linux ホスト + VirtualBox + Z:）の記述が実態と乖離していた |

## 作成物

| パス | 内容 | git |
|---|---|---|
| `CLAUDE.md` | プロジェクト要約・ビルド手順・落とし穴（91行） | コミット対象 |
| `.claude/agents/rust-explorer.md` | 調査担当・読み取り専用・haiku | ローカルのみ |
| `.claude/agents/rust-reviewer.md` | レビュー担当・読み取り専用・opus | ローカルのみ |
| `.claude/agents/doc-syncer.md` | 文書追従チェック・読み取り専用・haiku | ローカルのみ |
| `.claude/agents/test-runner.md` | ビルド/テスト実行・編集不可・haiku | ローカルのみ |
| `.claude/skills/verify-and-push/SKILL.md` | push 前検証手順（書き換え済み） | ローカルのみ |

> エージェント定義は **セッション起動時に読み込まれる**。作成直後のセッションでは
> `subagent_type` に指定できない（次回起動から有効）。当面は組み込みの `Explore` に
> 定義の中身を埋め込んで代用する。

## 環境の落とし穴（毎回踏むので必読）

- **scoop の shim が全滅**。`cargo` / `gh` / `pwsh` が名前で起動できない

      export PATH="/c/Users/capypara20/scoop/persist/rustup/.cargo/bin:/c/Users/capypara20/scoop/apps/gcc/15.2.0/bin:$PATH"

  - PATH を付けないと `windres` が無く `build.rs` が panic する
- **PowerShell ツールは使用不可**。Bash ツール + フルパスで `pwsh.exe` を呼ぶ
  - `/c/Users/capypara20/scoop/apps/pwsh/7.6.3/pwsh.exe`（動作確認済み）
- `gh` は `/c/Users/capypara20/scoop/apps/gh/2.97.0/bin/gh.exe`（**未認証・HTTP 401**）
- 既定ツールチェインは `windows-gnu`、CI とリリースは `x86_64-pc-windows-msvc`
- `.claude/settings.json` に旧環境の残骸あり（`additionalDirectories` が `/home/roze123/...`）— 未整理
- **ソースのインデントはタブ**。編集時に空白へ変えないこと

## 調査結果サマリ（実測で裏取り済み）

### 病巣: 「1種類足すと、あちこち直す」構造

**フィルタを1種増やすと約12箇所の修正が必要**

| 場所 | 重複の中身 | 確度 |
|---|---|---|
| `router.rs:86-150` | GlobSet 構築 x4 + Regex 構築 x4（完全同一コード） | 実測確認済み |
| `config/validate.rs:127-186` | glob 構文チェック x4 + regex 構文チェック x4 | 調査報告 |
| `config/validate.rs:113-174` | patterns と regex の排他チェック x4 | 調査報告 |

対象フィールドは8種:
`patterns` / `exclude_patterns` / `dir_patterns` / `exclude_dir_patterns` /
`regex` / `exclude_regex` / `dir_regex` / `exclude_dir_regex`

**アクションを1種増やすと5ファイルの修正が必要**（実測で確認）

| ファイル | 修正内容 |
|---|---|
| `config/types.rs:92-104` | `ActionType` enum + `impl_case_insensitive_deserialize!` |
| `config/model.rs:259-293` | `ActionConfig` に専用フィールド追加 |
| `config/validate.rs:295,320,348` | `collect_action_errors` の match アーム |
| `actions/mod.rs:1-6,101-122` | `pub mod` 宣言 + `execute_chain` の match アーム |
| `actions/(新規).rs` | 実装本体 |

`ActionType` は enum + match のままで trait 化されていない。

### 巨大関数 TOP5（実測）

| 関数 | 行数 | 場所 |
|---|---|---|
| `convert()` | **232行** | `csv_import.rs:89-320`（ファイル442行の過半数） |
| `start_watching()` | 132行 | `watcher.rs:29-161`（責務10以上） |
| `compile_rules()` | 121行 | `router.rs:82-203` |
| `run_router()` | 96行 | `router.rs:400-496`（**テスト無し**） |
| `search_sources()` | 91行 | `dashboard/search.rs:182-272` |

その他: `matches_pattern()` 75行（ネスト5段）、`main.rs` の `AFTER_LONG_HELP` は43行。

### clippy（`--all-targets`）

- `cat-watcher` 本体 **3件**のみ（very complex type x3）。テスト込みで6件
- `notify-fork` 6件（offset の isize キャスト、nul 終端文字列の手組みなど）
- **警告は少なく、コード自体は健全**。問題は構造であって細部ではない

## 着手候補（優先度順）

### 第1段 — 完了（+129 / -152 行、テスト 117 件パス）

1. ~~`router.rs` の GlobSet/Regex 構築をヘルパー関数化~~ **完了**
2. ~~`validate.rs` の glob/regex チェックを統一関数化~~ **完了**
3. ~~`copy.rs` の 2 関数を `common.rs` へ移動~~ **完了**

→ フィルタ追加時の修正が **12箇所 → 2箇所** になった。

**実施時に判明した不変条件（今後も壊さないこと）**

- `watch.patterns` だけが `Option<Vec<String>>`。`patterns = []` は
  「Some(空 GlobSet) = 何にもマッチしない」で、キー自体が無い `None`
  （フィルタなしで全通過）とは意味が違う。他の 3 種は `Vec<String>` なので
  「空 = 未指定」でよい。**4 種を機械的に揃えると全ファイルがマッチする回帰になる。**
  番人テスト: `tests/router.rs` の `test_compile_rules_keeps_empty_patterns_distinct_from_none`
  など 3 件（意図的に壊すと落ちることを確認済み）
- `validate.rs` のエラーは **積む順番がそのまま表示順**。元コードは
  `exclude_patterns` だけ「glob 検査 → 排他検査」、`exclude_dir_*` と `dir_*` は
  「排他検査 → glob 検査」という不揃いがある。揃えると表示順が変わるため据え置き

### 第2段 — 中リスク（テストの追加が要る）

4. `matches_pattern()` をファイル判定とディレクトリ判定に分割（ネスト5段→2段）
5. `csv_import.rs` の `convert()` を パース / 検証 / TOML生成 の3つに分割
6. `move_one_file()` を rename 試行 と copy+delete フォールバックに分割

### 第3段 — 高リスク・要相談

7. `Action` trait 導入で `execute_chain` の match を多態化（アクション追加が1ファイルで済む）
8. `start_watching()` と `run_router()` の分割（tokio::select! 絡みで所有権の調整が重い）

## 決定済みの次の作業

Roze との合意順: **第1段コミット → CSV 削除 → 設定の見やすさ改善**

### 1. CSV 機能の削除（v2.1.0）

- `--from-csv`（`csv_import.rs` 442行）と `--init csv`（`templates.rs` の `RULES_CSV`）を**両方**削除
- rules.toml → CSV への逆変換は元々存在しない
- 影響: `main.rs` / `csv_import.rs` / `templates.rs` / `tests/csv_import.rs` /
  `tests/templates.rs` / `README.md` / `doc/` 4ファイル
- 破壊的変更だが **v2.1.0** で出す（Roze の判断）
- これにより第2段候補の「`convert()` 232行を分割」は**不要になる**

### 2. 設定の「必須/任意が分からない」問題（案 B）

**原因**: `ActionConfig` が copy/move/command/execute の 4 種を 1 つの構造体に
詰め込んでいるため、アクション固有のフィールドを全部 `Option` にするしかない
（`model.rs` に 24 個の `Option`）。型が持つべき「必須」情報が失われ、
`validate.rs:293-371` の `collect_action_errors` が手書きで再実装している。

**対応**: `serde` のタグ付き enum に分割する。

```rust
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ActionConfig {
    Copy    { destination: String, ... },
    Move    { destination: String, ... },
    Command { shell: String, command: String, ... },
    Execute { program: String, args: Vec<String>, ... },
}
```

- **利用者の TOML の書き方は変わらない**（`type = "copy"` のまま）
- 必須項目の書き忘れを serde が自動で検出する
- `validate.rs` の手書き必須チェック約 80 行が不要になる
- 第3段の `Action` trait 化の土台になる

### 3. 検討中（未決）

- **`command` と `execute` の統合** — Roze の案。アクション種別を減らして
  設定を単純にする。破壊的変更なので CSV 削除と合わせて検討する
- **ブラウザ GUI での設定編集** — 保留。cat-watcher は `command` / `execute` で
  任意コマンドを実行でき、Windows サービスとしても動く。書き込み経路を足すと
  「画面に到達できる人が任意コマンドを実行できる」ことになる。
  現在のダッシュボードは `GET` のみ・`127.0.0.1:8080` 既定で書き込み経路が無い

## 作業の進め方

- 着手時はブランチを切る（**main に直接コミットしない**）
- 変更後は `/verify-and-push` で検証してから push
