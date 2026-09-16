// 1 件のイベントを表の 1 行（DOM）に変換する。
//
// 列の並びは style.css の `--cols` と対応している。片方だけ変えると
// ヘッダーと本文がずれるので、増減させるときは両方を直すこと。

import { highlightInto } from "./search.js";

export function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text != null) e.textContent = text;
  return e;
}

/** "2026-08-02 10:30:20" → { date: "2026-08-02", time: "10:30:20" } */
export function splitTimestamp(ts) {
  const s = ts || "";
  const sp = s.indexOf(" ");
  return sp < 0 ? { date: "", time: s } : { date: s.slice(0, sp), time: s.slice(sp + 1) };
}

/** 日付が変わったことを示す区切り行。 */
export function buildDaySeparator(date) {
  return el("div", "daysep", date);
}

/**
 * パスを「先頭・中ほど・ファイル名」の 3 つに割る。
 *
 *   C:\data\backup\2026\report.txt
 *   └ head ┘└─ mid ─┘ └─ base ─┘
 *
 * 幅が足りないときに削るのは mid だけにして、
 * 「どのドライブか」と「どのファイルか」は必ず残す。
 * CSS の text-overflow は末尾にしか `…` を置けないので、
 * 中略に見せるには要素を分けるしかない。
 */
export function splitPath(path) {
  const sepAt = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  if (sepAt < 0) return { head: "", mid: "", sep: "", base: path };

  const sep = path[sepAt];
  const dir = path.slice(0, sepAt); // 末尾の区切りは sep として別に持つ
  const base = path.slice(sepAt + 1);

  // 先頭の 1 区切りぶん（"C:\" や "/" など）は常に見せる。
  // 両方の区切り文字が混ざっていても、より手前にある方を境目にする。
  const cands = [dir.indexOf("/"), dir.indexOf("\\")].filter((i) => i >= 0);
  if (!cands.length) return { head: dir, mid: "", sep, base };
  const firstAt = Math.min(...cands);
  return { head: dir.slice(0, firstAt + 1), mid: dir.slice(firstAt + 1), sep, base };
}

function appendPath(cell, path) {
  const { head, mid, sep, base } = splitPath(path);
  const wrap = el("span", "path");

  for (const [cls, text] of [["p-head", head], ["p-mid", mid], ["p-sep", sep]]) {
    if (!text) continue;
    const span = el("span", cls);
    highlightInto(span, text);
    wrap.appendChild(span);
  }

  const baseEl = el("span", "p-base");
  highlightInto(baseEl, base);
  wrap.appendChild(baseEl);
  cell.appendChild(wrap);
}

/** ライブ表示の 1 行。 */
export function buildRow(ev) {
  const row = el("div", "row " + ev.kind);
  // 詳細ドロワーが行から元のイベントを引けるようにしておく。
  // 表示用に加工した文字列からは復元できないため。
  row.__ev = ev;
  const { time } = splitTimestamp(ev.ts);

  row.appendChild(el("span", "c-time", time));
  row.appendChild(el("span", "c-kind badge k-" + ev.kind, ev.kind));
  row.appendChild(el("span", "c-level lv lv-" + ev.level, ev.level));

  const rule = el("span", "c-rule");
  if (ev.rule) {
    highlightInto(rule, ev.rule);
    rule.title = ev.rule;
  }
  row.appendChild(rule);

  const pathCell = el("span", "c-path");
  if (ev.path) {
    appendPath(pathCell, ev.path);
    pathCell.title = ev.path;
  }
  row.appendChild(pathCell);

  const body = el("span", "c-body");
  if (ev.message) highlightInto(body, ev.message);
  if (ev.events) {
    if (ev.message) body.appendChild(document.createTextNode("  "));
    const evs = el("span", "events");
    evs.appendChild(document.createTextNode("("));
    highlightInto(evs, ev.events);
    evs.appendChild(document.createTextNode(")"));
    body.appendChild(evs);
  }
  if (ev.message || ev.events) {
    body.title = [ev.message, ev.events && "(" + ev.events + ")"].filter(Boolean).join("  ");
  }
  row.appendChild(body);

  // 詳細を開くボタン。position:absolute なのでグリッドの列数には影響しない。
  // グループ見出しはクリックが開閉に取られるので、そこではこれが唯一の入口になる。
  const more = el("button", "rowmore", "⋯");
  more.title = "詳細を開く";
  more.tabIndex = -1;
  row.appendChild(more);

  return row;
}

/**
 * 検知 1 件ぶんのグループを作る。
 *
 * detect 行を見出しにして、それに続く action 行を折りたたみできる子要素へ入れる。
 * 「まず一致したファイルだけを一覧したい。中身は必要なときだけ開く」という
 * 読み方ができるようにするための構造。
 *
 * 戻り値の `children` に action 行を追加し、結果が出るたびに
 * [`updateGroupSummary`] で見出しの集計を書き換える。
 */
export function buildGroupBlock(detectEv) {
  const block = el("div", "grp");
  const head = buildRow(detectEv);
  head.classList.add("grp-head");

  // 開閉の目印。position:absolute なのでグリッドの列数には影響しない。
  const twisty = el("span", "twisty", "▶");
  head.insertBefore(twisty, head.firstChild);

  // 集計は「内容」列の中に置く。列を増やすとヘッダーとずれるため。
  const summary = el("span", "grp-summary");
  head.querySelector(".c-body").appendChild(summary);

  const children = el("div", "grp-children");

  head.addEventListener("click", (e) => {
    // 本文中のテキスト選択を邪魔しない
    if (window.getSelection().toString()) return;
    // 「⋯」は詳細を開くためのボタンなので、開閉には使わせない
    if (e.target.closest(".rowmore")) return;
    setGroupOpen(block, !block.classList.contains("open"));
    e.preventDefault();
  });

  block.appendChild(head);
  block.appendChild(children);
  return { block, head, children, summary };
}

/** グループの開閉を切り替える。 */
export function setGroupOpen(block, open) {
  block.classList.toggle("open", open);
  const twisty = block.querySelector(".twisty");
  if (twisty) twisty.textContent = open ? "▼" : "▶";
}

/**
 * 見出しに「アクション何件で、どういう結果だったか」を書く。
 * 畳んだままでも失敗の有無が分かるようにするのが目的。
 */
export function updateGroupSummary(summary, counts) {
  summary.innerHTML = "";
  if (counts.total === 0) {
    summary.appendChild(el("span", "grp-none", "アクションなし"));
    return;
  }
  summary.appendChild(el("span", "grp-count", `アクション ${counts.total} 件`));
  for (const [key, label, cls] of [
    ["ok", "OK", "lv-ok"],
    ["warn", "WARN", "lv-warn"],
    ["error", "ERROR", "lv-error"],
  ]) {
    if (counts[key] > 0) {
      summary.appendChild(el("span", "grp-tally " + cls, `${label} ${counts[key]}`));
    }
  }
}

/** 過去ログ検索の 1 行（列構成がライブとは別）。 */
export function buildHistoryRow(hit) {
  const row = el("div", "hrow");
  row.appendChild(el("span", "badge k-" + hit.kind, hit.kind));

  const file = el("span", "hfile", hit.file);
  file.title = hit.file;
  row.appendChild(file);

  const line = el("span", "hline");
  highlightInto(line, hit.line);
  row.appendChild(line);
  return row;
}
