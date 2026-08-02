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
 * パスを「親フォルダ」と「ファイル名」に分けて描く。
 * 幅が足りないときに削るのは親フォルダ側だけなので、
 * 一番知りたいファイル名は必ず残る。
 */
function appendPath(cell, path) {
  const cut = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  const wrap = el("span", "path");
  if (cut >= 0) {
    const dir = el("span", "dir");
    highlightInto(dir, path.slice(0, cut + 1));
    wrap.appendChild(dir);
  }
  const base = el("span", "base");
  highlightInto(base, cut >= 0 ? path.slice(cut + 1) : path);
  wrap.appendChild(base);
  cell.appendChild(wrap);
}

/** ライブ表示の 1 行。 */
export function buildRow(ev) {
  const row = el("div", "row " + ev.kind);
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

  return row;
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
