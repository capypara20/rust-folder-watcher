# cat-watcher

ファイル/フォルダの作成・変更・削除・リネームを検知し、コピー・移動・コマンド実行を
自動で行う Rust 製の常駐監視ツール。Windows / Linux 向けにバイナリを配布している。

## リポジトリ構成

```
cat-watcher/          本体クレート（ここが実装のほぼ全部）
  src/
    main.rs           CLI エントリ。mod 宣言と --help 本文
    config/           TOML 読み込み・型・バリデーション
    router.rs         イベント → ルールのマッチング（中核）
    watcher.rs        OS のファイル監視イベント受信・デバウンス
    actions/          copy / move / command / execute / spawn
    logger/           システム・検知・アクションの 3 種ログ
    dashboard/        localhost HTTP + SSE のリアルタイム表示
    platform/         Windows サービス登録・ユーザー権限での起動
    tests/            テスト本体を一元集約（後述）
notify-fork/          notify クレートの fork（Windows 監視の挙動修正用）
config/               サンプル TOML
doc/                  要件定義・詳細設計・運用仕様（実装前に必ず参照）
tool/                 ローカル検証用 PowerShell スクリプト（gitignore 対象）
```

## ビルドとテスト

**この PC では scoop の shim が壊れており、`cargo` / `gh` / `pwsh` が名前で起動できない。**
以下の PATH 指定を毎回付けること。付け忘れると `windres` が見つからず build.rs が panic する。

```bash
export PATH="/c/Users/capypara20/scoop/persist/rustup/.cargo/bin:/c/Users/capypara20/scoop/apps/gcc/15.2.0/bin:$PATH"

# テスト（114 件、数秒で終わる）
cargo test --locked --manifest-path cat-watcher/Cargo.toml

# 最小ビルド確認（CI がこれも検証する。dashboard feature を外して壊れていないか）
cargo build --locked --no-default-features --manifest-path cat-watcher/Cargo.toml

# リリースビルド
cargo build --release --locked --manifest-path cat-watcher/Cargo.toml
```

- **PowerShell ツールは使えない**（pwsh 起動が shim を経由して失敗する）。Bash ツールを使う。
- `gh` はフルパスで呼ぶ: `C:/Users/capypara20/scoop/apps/gh/2.97.0/bin/gh.exe`
- 既定ツールチェインは `windows-gnu`。CI/リリースは `x86_64-pc-windows-msvc`。**ローカルで通っても
  MSVC で壊れることがある**ので、Windows 固有コードを触ったら CI の結果を必ず確認する。
- `.cargo/config.toml` が MSVC ビルドに `crt-static` を付けている（VC++ 再頒布パッケージ不要にするため）。

## テストの置き場所

テスト本体は **`cat-watcher/src/tests/` に集約**し、各モジュール末尾から `#[path]` で読み込む。
モジュール内に直接テストを書かない。

```rust
// router.rs の末尾
#[cfg(test)]
#[path = "tests/router.rs"]
mod tests;
```

共通ヘルパーは `src/tests/support.rs`（`main.rs` から `test_support` として読み込み済み）。

## 開発ルール

- **main に直接 push しない。** 必ずブランチを切って PR を出す。
- **push 前に `/verify-and-push` スキルを実行する。** cargo test と実機スモークが必須。
- コミットメッセージは日本語 + Conventional Commits。
  例: `feat(dashboard): 検索フィルタを追加`, `fix: destination の存在チェックを修正`
- PR マージ時のコミットには `(#PR番号)` が付く（squash merge 運用）。
- `Cargo.toml` の `version` を上げて main に push すると、`release.yml` が自動でタグを打ち
  GitHub Releases にバイナリを公開する。**バージョン変更は意図したときだけ。**

## 落とし穴

- `.gitignore` が **`/.claude/` と `/tool/*` を丸ごと除外**している。
  `.claude/agents/` などを作ってもリポジトリには入らない（ローカル専用）。
- CI は PR と main への push のみで走る。`**.md` と `doc/**` のみの変更はスキップされる。
- ネットワークパス（UNC）配下や共有フォルダでは Windows の監視 API がイベントを拾わない。
  **動作検証は必ずローカルの `C:` ドライブで行う。**
- `dashboard` は default feature。依存を追加するときは `--no-default-features` でも
  ビルドが通るか確認する。

## 詳細仕様

実装前に該当箇所を読むこと。`doc/detailed-design.md` が最も詳しい。

- `doc/requirements.md` — 要件定義
- `doc/detailed-design.md` — 詳細設計（設定仕様・ルール評価・アクション・ログ・Windows 固有事項）
- `doc/specification.md` — 運用仕様（設定例集・トラブルシューティング）
- `README.md` — 利用者向けリファレンス
