// 検索条件の保持と、テキストへのハイライト適用。
//
// 「検索語 → 正規表現」の変換を 1 か所に閉じ込めておくことで、
// ライブ表示・過去ログ表示・一致件数カウントが同じ判定を共有できる。

function escapeRe(s) {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

export const search = {
  term: "",
  caseSensitive: false,
  useRegex: false,
  /** 現在の検索用 RegExp。検索なし／正規表現が不正なときは null。 */
  re: null,
  /** 直近の rebuild で正規表現の構文エラーが出たか。 */
  invalid: false,

  /** 入力値と各トグルから RegExp を組み直す。 */
  rebuild(term) {
    this.term = term;
    this.invalid = false;
    if (!term) {
      this.re = null;
      return;
    }
    try {
      const src = this.useRegex ? term : escapeRe(term);
      this.re = new RegExp(src, this.caseSensitive ? "g" : "gi");
    } catch (_) {
      this.re = null;
      this.invalid = true;
    }
  },

  get active() {
    return this.re !== null;
  },

  /** テキストが検索にヒットするか。 */
  test(text) {
    if (!this.re || !text) return false;
    this.re.lastIndex = 0;
    return this.re.test(text);
  },
};

/** イベント 1 件のうち、検索対象にする文字列をまとめる。 */
export function haystack(ev) {
  return [ev.rule, ev.path, ev.message, ev.events].filter(Boolean).join(" ");
}

/**
 * `text` を検索語で分割し、一致部分を `<mark>` で包んで `parent` に追加する。
 * 検索していないときはただのテキストノードとして追加する。
 */
export function highlightInto(parent, text) {
  text = text || "";
  if (!text) return;
  if (!search.re) {
    parent.appendChild(document.createTextNode(text));
    return;
  }

  search.re.lastIndex = 0;
  let last = 0;
  let m;
  while ((m = search.re.exec(text)) !== null) {
    if (m.index > last) {
      parent.appendChild(document.createTextNode(text.slice(last, m.index)));
    }
    const mk = document.createElement("mark");
    mk.textContent = m[0];
    parent.appendChild(mk);
    last = m.index + m[0].length;
    // 空文字にマッチする正規表現（例: `a*`）で無限ループしないよう進める
    if (m[0].length === 0) search.re.lastIndex++;
  }
  if (last < text.length) {
    parent.appendChild(document.createTextNode(text.slice(last)));
  }
}
