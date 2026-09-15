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

## WSL で Linux 側を検証するとき

CI は Ubuntu でもテストを回すので、Linux 側の経路は WSL で確認できる。
ただし **そのまま使うと検証にならない落とし穴が 2 つある。**

### 1. WSL の PATH には Windows のパスが入っている

既定で Windows 相互運用が有効なため、`/mnt/c/...` 配下が PATH に大量に入る
（実測で **49 個**）。そのため `notepad.exe` のような Windows 専用の実行ファイルが
**WSL では見つかってしまう**。

実際に「PATH で解決できること」を確かめるテストが WSL では通り、
GitHub の Ubuntu ランナーで落ちた例がある。

CI と同じ条件にするには `/mnt/` 由来を除いた PATH で回す。

```bash
CLEAN=$(echo "$PATH" | tr ':' '\n' | grep -v '^/mnt/' | paste -sd:)
PATH="$CLEAN" cargo test --locked --manifest-path cat-watcher/Cargo.toml
```

### 2. `/mnt/c` 配下では inotify が効かない

Windows ドライブは drvfs 経由でマウントされており、ファイル変更通知が飛んでこない。
監視対象は **WSL ネイティブ側（`~/` 配下など ext4 上）** に置くこと。
Windows 側の「共有フォルダや UNC では監視 API が効かない」と同じ種類の制約。

ビルドも `/mnt/c` 上では I/O が遅いので、ソースを `~/` へコピーしてから行う。

```bash
tar -C /mnt/c/Users/capypara20/Developer/02_App/rust-folder-watcher \
    --exclude=./target --exclude=./.git -cf - . | tar -C ~/cat-watcher-src -xf -
```

### Rust ツールチェイン

Ubuntu-24.04 に `rustup` で導入済み（`~/.cargo/bin`）。`gcc` は最初から入っている。
`sudo` はパスワードを要求するが、`rustup` は root 不要なので問題ない。

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

## ドキュメント

> **`doc/` 配下はプロジェクト立ち上げ時に書いたもので、実装に追従できていない。
> 迷ったら必ずコードを正とすること。** `doc/` は「何を作ろうとしたか」を知るための
> 参考資料であって、現在の仕様書ではない。

- `README.md` — **利用者向けリファレンス。ここは比較的新しく、信頼できる**
- `doc/requirements.md` — 要件定義（立ち上げ時）
- `doc/detailed-design.md` — 詳細設計（立ち上げ時。設計意図を知りたいときに読む）
- `doc/specification.md` — 運用仕様（立ち上げ時。設定例は古い可能性が高い）

実際に見つかっているズレの例:

- `doc/specification.md` の `[global]` に `log_level` / `dry_run` が載っているが、
  **どちらも実装に存在しない**（`log_level` は v1.3.0 のログ設定刷新で廃止）
- 設計書は `csv2toml.exe` という別バイナリを前提に書かれていた（実際は本体の
  サブコマンドだった。CSV 機能自体 v2.1.0 で削除）

**仕様を確かめたいときの優先順位**

1. コード（`cat-watcher/src/`）と `src/tests/` のテスト
2. `--help` の実出力（`main.rs` の `AFTER_LONG_HELP`）
3. `README.md`
4. `doc/`（あくまで参考）

ドキュメントを直したときは、ついでに古い記述がないか周辺も確認すること。
