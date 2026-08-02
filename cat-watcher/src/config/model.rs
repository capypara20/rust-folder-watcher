//! 設定ファイル（global.toml / rules.toml）をマッピングするデータ構造。

use serde::Deserialize;

use super::types::{ActionType, Event, LogLevel, LogRotation, WatchTarget};

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlobalConfig {
    pub retry: RetryConfig,
    pub system_log: SystemLogConfig,
    /// ダッシュボード設定（省略可）。未指定なら無効。
    #[serde(default)]
    pub dashboard: Option<DashboardConfig>,
    /// 起動時スキャン設定（省略可）。未指定なら有効（既定 ON）。
    #[serde(default)]
    pub startup_scan: Option<StartupScanConfig>,
    /// copy / move の宛先フォルダの扱い（省略可）。未指定なら自動作成する。
    #[serde(default)]
    pub destination: Option<DestinationConfig>,
    /// 検知のデバウンス設定（省略可）。未指定なら既定値を使う。
    #[serde(default)]
    pub detect: Option<DetectConfig>,
    /// Windows サービス設定（省略可）。未指定なら既定値を使う。
    // service フィールドは Windows のサービス起動経路でのみ参照するため、
    // 非 Windows ビルドでは未使用になる。
    #[cfg_attr(not(windows), allow(dead_code))]
    #[serde(default)]
    pub service: Option<ServiceConfig>,
}

impl GlobalConfig {
    /// 起動時スキャンを行うか。セクション未指定なら有効（既定 ON）。
    /// イベント監視だけでは取りこぼす「起動前から存在するファイル」や
    /// 「ダウンタイム中に届いたファイル」を回収するための安全網。
    pub fn scan_on_start(&self) -> bool {
        self.startup_scan
            .as_ref()
            .map(|s| s.enabled)
            .unwrap_or(true)
    }

    /// copy / move の宛先フォルダが無いときに自動作成するか（全ルール共通の既定値）。
    /// 各アクションの `auto_create` で個別に上書きできる。セクション未指定なら ON。
    pub fn auto_create_destination(&self) -> bool {
        self.destination
            .as_ref()
            .map(|d| d.auto_create)
            .unwrap_or(true)
    }

    /// 最後のイベントからこの時間だけ静かになったら「確定」とみなす（ミリ秒）。
    pub fn debounce_ms(&self) -> u64 {
        self.detect
            .as_ref()
            .map(|d| d.debounce_ms)
            .unwrap_or(DEFAULT_DEBOUNCE_MS)
    }

    /// デバウンス済みのパスが無いか確認する間隔（ミリ秒）。
    pub fn poll_interval_ms(&self) -> u64 {
        self.detect
            .as_ref()
            .map(|d| d.poll_interval_ms)
            .unwrap_or(DEFAULT_POLL_INTERVAL_MS)
    }

    /// Windows サービス起動時に、外部プロセス（command / execute）を
    /// アクティブなログオンユーザーの権限で実行するか。セクション未指定なら
    /// 有効（既定 ON）。ログオンユーザーがいない場合はサービスアカウント
    /// 権限へフォールバックする。CLI 起動時はこの設定に関係なく、元々
    /// ログオンユーザー権限で動作する。
    // 参照するのは Windows のサービス起動経路のみ。
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn run_as_logged_in_user(&self) -> bool {
        self.service
            .as_ref()
            .map(|s| s.run_as_logged_in_user)
            .unwrap_or(true)
    }
}

/// Windows サービス設定。サービスとして常駐する場合の挙動を制御する。
/// CLI 起動時には参照されない。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceConfig {
    /// サービス起動時に外部プロセスをアクティブなログオンユーザー権限で実行するか。
    /// サービスは既定で SYSTEM 権限で動くため、PowerShell / 7-Zip などの外部
    /// プロセスがログオンユーザーの環境で動かず不便なことへの対策。
    /// ログオンユーザーがいない（誰もログオンせずサービス起動した）場合は
    /// サービスアカウント権限で起動する。
    // 値を読むのは Windows のサービス起動経路のみ。
    #[cfg_attr(not(windows), allow(dead_code))]
    #[serde(default = "default_true")]
    pub run_as_logged_in_user: bool,
}

/// 起動時スキャン設定。起動直後に監視フォルダを 1 度だけ走査し、
/// 既に存在するエントリを Create イベントとして検知パイプラインへ流す。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartupScanConfig {
    /// 起動時スキャンを行うか。
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// copy / move の宛先フォルダの扱い（全ルール共通の既定値）。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DestinationConfig {
    /// 宛先フォルダが存在しないときに自動作成するか。
    ///
    /// - `true`（既定）: 実行時に再帰的に作成する。起動時の存在チェックは
    ///   「ドライブや共有そのものが存在するか」だけに緩める。
    /// - `false`: 自動作成しない。起動時に宛先が無ければバリデーションエラー、
    ///   実行時に無ければアクション失敗にする（typo で予期しない場所に
    ///   書き込むのを防ぎたいとき用）。
    #[serde(default = "default_true")]
    pub auto_create: bool,
}

/// 検知のデバウンス設定。エディタや同期ソフトが出す連続イベントを束ねる。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetectConfig {
    /// 最後のイベントからこの時間だけ静かになったら確定とみなす（ミリ秒）。
    /// 大きいファイルの書き込み完了を待ちたい場合は長めにする。
    #[serde(default = "default_debounce_ms")]
    pub debounce_ms: u64,
    /// 確定済みのパスが無いか確認する間隔（ミリ秒）。短いほど反応が速く、
    /// その分 CPU を使う。通常は既定値のままでよい。
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,
}

pub const DEFAULT_DEBOUNCE_MS: u64 = 500;
pub const DEFAULT_POLL_INTERVAL_MS: u64 = 100;

fn default_debounce_ms() -> u64 {
    DEFAULT_DEBOUNCE_MS
}

fn default_poll_interval_ms() -> u64 {
    DEFAULT_POLL_INTERVAL_MS
}

/// ダッシュボード（ブラウザでログをリアルタイム表示する localhost HTTP サーバ）設定。
/// 実際に機能するのは `dashboard` feature を有効にしてビルドした場合のみ。
/// feature 無しビルドでも設定の読み込み・検証は行う（セクションを書いてもエラーにしない）。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DashboardConfig {
    /// ダッシュボードを起動するか。
    #[serde(default)]
    pub enabled: bool,
    /// 待ち受けアドレス。ログにパスが出るためローカル限定を推奨。
    #[serde(default = "default_dashboard_bind")]
    pub bind: String,
    /// 接続時にブラウザへ再生する直近イベント件数（メモリ保持）。
    // history はサーバ起動時（dashboard feature 有効時）のみ参照するため、
    // feature 無しビルドでは未使用になる。
    #[cfg_attr(not(feature = "dashboard"), allow(dead_code))]
    #[serde(default = "default_dashboard_history")]
    pub history: usize,
}

fn default_dashboard_bind() -> String {
    "127.0.0.1:8080".to_string()
}

fn default_dashboard_history() -> usize {
    200
}

#[derive(Debug, Clone, Deserialize)]
pub struct RulesConfig {
    pub rules: Vec<Rule>,
}

/// リトライ設定（copy / move アクション用）。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetryConfig {
    pub count: u32,
    pub interval_ms: u64,
}

/// システムログ設定（プログラム全体の起動日誌）。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemLogConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub dir: String,
    pub file_name: String,
    pub rotation: LogRotation,
    pub level: LogLevel,
    /// コンソール出力 ON/OFF（ターミナル再設計までの暫定置き場）。
    #[serde(default = "default_true")]
    pub console: bool,
}

/// ルール別ログ設定。検知ログ・アクションログをそれぞれ個別指定する。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleLog {
    pub detect: Option<RuleLogTarget>,
    pub action: Option<RuleLogTarget>,
}

/// 検知ログ / アクションログ共通の出力先設定。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleLogTarget {
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub dir: String,
    pub file_name: String,
    pub rotation: LogRotation,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Rule {
    pub enabled: bool,
    pub name: String,
    pub watch: Watch,
    pub actions: Vec<ActionConfig>,
    pub log: Option<RuleLog>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Watch {
    pub path: String,
    pub recursive: bool,
    pub target: WatchTarget,
    pub include_hidden: bool,
    pub patterns: Option<Vec<String>>,
    pub regex: Option<String>,
    pub exclude_patterns: Vec<String>,
    #[serde(default)]
    pub exclude_regex: Option<String>,
    #[serde(default)]
    pub exclude_dir_patterns: Vec<String>,
    #[serde(default)]
    pub exclude_dir_regex: Option<String>,
    #[serde(default)]
    pub dir_patterns: Vec<String>,
    #[serde(default)]
    pub dir_regex: Option<String>,
    pub events: Vec<Event>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ActionConfig {
    #[serde(rename = "type")]
    pub type_: ActionType,

    // typeがCopy / Move のとき
    pub destination: Option<String>,
    pub overwrite: Option<bool>,
    pub verify_integrity: Option<bool>,
    pub preserve_structure: Option<bool>,
    /// 宛先フォルダを自動作成するか。未指定なら global.toml の
    /// `[destination] auto_create` に従う（設定読み込み後に解決され、
    /// バリデーションと実行時はどちらもここに入った値だけを見る）。
    #[serde(default)]
    pub auto_create: Option<bool>,

    // typeがCommand / Executeのとき
    pub working_dir: Option<String>,

    // typeがCommandのとき
    pub shell: Option<String>,
    pub command: Option<String>,

    // typeがExecuteのとき
    pub program: Option<String>,
    pub args: Option<Vec<String>>,

    // typeがLogのとき
    pub message: Option<String>,

    /// このアクションを実行する前に待つ時間（ミリ秒）。省略時は 0（待たない）。
    ///
    /// デバウンスだけでは足りないケース向けの逃げ道。たとえば
    /// 「巨大ファイルの書き込みが完全に終わるまで待ってからコピーしたい」
    /// 「前のアクションで起動した外部スクリプトの出力が出揃うまで待ちたい」など。
    #[serde(default)]
    pub delay_ms: Option<u64>,
}
