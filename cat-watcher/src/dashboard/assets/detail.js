// 選んだ 1 件の全項目を、画面下のドロワーへ折り返しありで出す。
//
// 表は 1 行に収める作りなので、長いパスや長いエラーメッセージは必ず見切れる。
// ホバーで出る OS のツールチップ（title 属性）では選択もコピーもできないため、
// 「全部読む・コピーする」ための逃げ道としてここを用意している。
//
// 右ではなく下に置いているのは、横幅をフルに使えるから。
// 長いパスが折り返さずに 1 行で収まる可能性が高い。

import { el } from "./render.js";

const $ = (id) => document.getElementById(id);

/** ドロワーに並べる項目。値が空のものは行ごと省く。 */
const FIELDS = [
  ["時刻", (ev) => ev.ts],
  ["種別", (ev) => [ev.kind, ev.level].filter(Boolean).join("  /  ")],
  ["ルール", (ev) => ev.rule],
  ["対象", (ev) => ev.path],
  ["イベント", (ev) => ev.events],
  ["内容", (ev) => ev.message],
];

const state = {
  ev: null,    // 表示中のイベント
  row: null,   // 選択中の行（ハイライト用）
};

/* ---- 表示 ------------------------------------------------ */

function fillBody(ev) {
  const body = $("dtBody");
  body.innerHTML = "";
  for (const [label, get] of FIELDS) {
    const value = get(ev);
    if (!value) continue;
    body.appendChild(el("dt", null, label));
    const dd = el("dd", null, value);
    // パスやコマンドは 1 語として長くなるので、どこでも折り返せるようにする
    if (label === "対象" || label === "内容") dd.classList.add("wrapany");
    body.appendChild(dd);
  }
}

/** イベント全体をテキスト 1 枚にする（コピー用）。 */
function asText(ev) {
  return FIELDS
    .map(([label, get]) => [label, get(ev)])
    .filter(([, v]) => v)
    .map(([label, v]) => label + "\t" + v)
    .join("\n");
}

function selectRow(row) {
  if (state.row) state.row.classList.remove("sel");
  state.row = row || null;
  if (row) row.classList.add("sel");
}

/**
 * 行に紐づくイベントをドロワーへ出す。
 * 行の DOM には `buildRow` が `__ev` としてイベントを持たせてある。
 */
export function showDetail(row) {
  const ev = row && row.__ev;
  if (!ev) return;
  state.ev = ev;
  selectRow(row);
  fillBody(ev);
  $("detail").hidden = false;
}

export function closeDetail() {
  state.ev = null;
  selectRow(null);
  $("detail").hidden = true;
}

export function isDetailOpen() {
  return !$("detail").hidden;
}

/**
 * 選択を 1 つ上／下へ動かす。畳まれたグループの中にある行は飛ばす
 * （`offsetParent` が null になるので、それで見えているかを判定する）。
 */
export function moveSelection(delta) {
  if (!isDetailOpen()) return;
  const rows = [...$("log").querySelectorAll(".row")].filter((r) => r.offsetParent !== null);
  if (!rows.length) return;
  const at = state.row ? rows.indexOf(state.row) : -1;
  const next = rows[Math.min(Math.max(at + delta, 0), rows.length - 1)];
  if (!next || next === state.row) return;
  showDetail(next);
  next.scrollIntoView({ block: "nearest" });
}

/**
 * 再描画で行の DOM が作り直されると選択が迷子になる。
 * ドロワーの中身はイベントのコピーなので出したままにし、
 * ハイライトだけ落とす。
 */
export function forgetSelectedRow() {
  state.row = null;
}

/* ---- コピー ---------------------------------------------- */

/**
 * クリップボードへ書く。`navigator.clipboard` は安全なコンテキスト
 * （localhost は該当する）でしか使えないので、古い経路も残す。
 */
function copyText(text, button) {
  const done = () => {
    const before = button.textContent;
    button.textContent = "コピーしました";
    button.disabled = true;
    setTimeout(() => {
      button.textContent = before;
      button.disabled = false;
    }, 1200);
  };

  if (navigator.clipboard && navigator.clipboard.writeText) {
    navigator.clipboard.writeText(text).then(done, () => fallbackCopy(text, done));
  } else {
    fallbackCopy(text, done);
  }
}

function fallbackCopy(text, done) {
  const ta = el("textarea");
  ta.value = text;
  ta.style.position = "fixed";
  ta.style.opacity = "0";
  document.body.appendChild(ta);
  ta.select();
  try {
    document.execCommand("copy");
    done();
  } catch (_) {
    /* コピーできない環境では何もしない（テキストは選択して手で取れる） */
  }
  document.body.removeChild(ta);
}

/* ---- 配線 ------------------------------------------------ */

export function initDetail() {
  $("dtClose").addEventListener("click", closeDetail);
  $("dtCopy").addEventListener("click", (e) => {
    if (state.ev) copyText(asText(state.ev), e.currentTarget);
  });
  $("dtCopyPath").addEventListener("click", (e) => {
    if (state.ev && state.ev.path) copyText(state.ev.path, e.currentTarget);
  });

  // 行のクリックでドロワーを開く。グループの見出しだけは従来どおり開閉に使うので、
  // そちらは行の右端に出る「⋯」ボタン経由にする（`.rowmore`）。
  $("log").addEventListener("click", (e) => {
    const more = e.target.closest(".rowmore");
    if (more) {
      e.stopPropagation();
      showDetail(more.closest(".row"));
      return;
    }
    const row = e.target.closest(".row");
    if (!row || row.classList.contains("grp-head")) return;
    if (window.getSelection().toString()) return; // 文字を選んでいるときは邪魔しない
    showDetail(row);
  });

  document.addEventListener("keydown", (e) => {
    if (!isDetailOpen()) return;
    // 検索ボックスに入力中は横取りしない
    if (e.target.tagName === "INPUT" || e.target.tagName === "TEXTAREA") return;
    if (e.key === "Escape") { closeDetail(); e.preventDefault(); }
    else if (e.key === "ArrowDown") { moveSelection(1); e.preventDefault(); }
    else if (e.key === "ArrowUp") { moveSelection(-1); e.preventDefault(); }
  });
}
