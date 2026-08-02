// ダッシュボードの入口。状態の保持・SSE 接続・UI の配線を担当する。
//
//   search.js … 検索条件とハイライト
//   render.js … 1 イベント → 表の 1 行
//   app.js    … それらを繋いで画面を組み立てる（このファイル）

import { search, haystack, highlightInto } from "./search.js";
import { buildRow, buildHistoryRow, buildDaySeparator, splitTimestamp, el } from "./render.js";

const MAX_EVENTS = 5000; // メモリに保持する最大イベント数
const MAX_DOM = 2000;    // DOM に残す最大行数（描画負荷対策）
const SEARCH_DEBOUNCE_MS = 120;

const $ = (id) => document.getElementById(id);

const ui = {
  table: $("table"),
  log: $("log"),
  banner: $("banner"),
  dot: $("dot"),
  statusText: $("statusText"),
  search: $("search"),
  searchBox: document.querySelector(".searchbox"),
  matchInfo: $("matchInfo"),
  count: $("count"),
  shown: $("shown"),
  only: $("only"),
  pause: $("pause"),
};

const state = {
  events: [],
  mode: "live",        // "live" | "history"
  autoscroll: true,
  paused: false,
  pendingWhilePaused: 0,
  wrap: false,
  currentMatch: -1,    // ナビゲーション中の <mark> インデックス
  flashArmed: false,   // 接続直後のバックログ再生では光らせない
  lastDate: null,      // 直近に描いた日付（区切り行の挿入判定用）
  shownCount: 0,
};

/* ---- 表示判定 ------------------------------------------- */

function checkedKinds() {
  const set = {};
  document.querySelectorAll("input.kind").forEach((c) => {
    if (c.checked) set[c.value] = true;
  });
  return set;
}

function shouldShow(ev, kinds) {
  if (!kinds[ev.kind]) return false;
  if (ui.only.checked && search.active && !search.test(haystack(ev))) return false;
  return true;
}

/* ---- ライブ表示 ------------------------------------------ */

let emptyShown = true;

function clearEmpty() {
  if (emptyShown) {
    ui.log.innerHTML = "";
    emptyShown = false;
  }
}

function showEmpty(message) {
  ui.log.innerHTML = "";
  ui.log.appendChild(el("div", "empty", message));
  emptyShown = true;
  state.lastDate = null;
  state.shownCount = 0;
  updateCounters();
}

/** 日付が変わっていれば区切り行を差し込む。 */
function appendDaySeparatorIfNeeded(parent, ev) {
  const { date } = splitTimestamp(ev.ts);
  if (!date || date === state.lastDate) return;
  state.lastDate = date;
  parent.appendChild(buildDaySeparator(date));
}

function appendLive(ev, isNew) {
  if (state.mode !== "live") return;
  if (!shouldShow(ev, checkedKinds())) return;

  clearEmpty();
  const nearBottom = ui.log.scrollHeight - ui.log.scrollTop - ui.log.clientHeight < 40;

  appendDaySeparatorIfNeeded(ui.log, ev);
  const row = buildRow(ev);
  if (isNew && state.flashArmed && ev.kind === "detect") row.classList.add("flash");
  ui.log.appendChild(row);
  state.shownCount++;

  while (ui.log.childNodes.length > MAX_DOM) ui.log.removeChild(ui.log.firstChild);
  if (state.autoscroll && nearBottom) ui.log.scrollTop = ui.log.scrollHeight;

  updateCounters();
  updateMatchInfo();
}

function rerenderLive() {
  state.mode = "live";
  ui.banner.hidden = true;
  ui.table.classList.remove("history");
  search.rebuild(ui.search.value);
  ui.searchBox.classList.toggle("invalid", search.invalid);

  ui.log.innerHTML = "";
  emptyShown = false;
  state.lastDate = null;
  state.shownCount = 0;

  const kinds = checkedKinds();
  const frag = document.createDocumentFragment();
  for (const ev of state.events.slice(-MAX_DOM)) {
    if (!shouldShow(ev, kinds)) continue;
    appendDaySeparatorIfNeeded(frag, ev);
    frag.appendChild(buildRow(ev));
    state.shownCount++;
  }

  if (state.shownCount === 0) {
    showEmpty("表示できるイベントがありません");
  } else {
    ui.log.appendChild(frag);
    if (state.autoscroll) ui.log.scrollTop = ui.log.scrollHeight;
  }

  state.currentMatch = -1;
  updateCounters();
  updateMatchInfo();
}

function onEvent(ev) {
  state.events.push(ev);
  if (state.events.length > MAX_EVENTS) {
    state.events.splice(0, state.events.length - MAX_EVENTS);
  }
  ui.count.textContent = state.events.length;

  if (state.paused) {
    state.pendingWhilePaused++;
    ui.pause.textContent = "再開 (" + state.pendingWhilePaused + ")";
    return;
  }
  appendLive(ev, true);
}

function updateCounters() {
  ui.shown.textContent = state.shownCount;
}

/* ---- 検索ナビゲーション（ハイライト間の移動）-------------- */

function marks() {
  return ui.log.querySelectorAll("mark");
}

function updateMatchInfo() {
  const ms = marks();
  if (state.currentMatch >= ms.length) state.currentMatch = ms.length - 1;
  ui.matchInfo.textContent = (ms.length ? state.currentMatch + 1 : 0) + "/" + ms.length;
}

function gotoMatch(delta) {
  const ms = marks();
  if (!ms.length) return;
  if (state.currentMatch >= 0 && ms[state.currentMatch]) {
    ms[state.currentMatch].classList.remove("current");
  }
  state.currentMatch = (state.currentMatch + delta + ms.length) % ms.length;
  const cur = ms[state.currentMatch];
  cur.classList.add("current");
  cur.scrollIntoView({ block: "center", behavior: "smooth" });
  ui.matchInfo.textContent = state.currentMatch + 1 + "/" + ms.length;
}

/* ---- 過去ログ検索（サーバ側でログファイルを走査）---------- */

function addBackButton() {
  const back = el("button", null, "✕ ライブに戻る");
  back.addEventListener("click", rerenderLive);
  ui.banner.appendChild(back);
}

function runHistory() {
  const term = ui.search.value.trim();
  if (!term) {
    ui.search.focus();
    return;
  }
  const url =
    "/search?q=" + encodeURIComponent(term) +
    "&regex=" + (search.useRegex ? 1 : 0) +
    "&ci=" + (search.caseSensitive ? 0 : 1) +
    "&limit=1000";

  ui.banner.hidden = false;
  ui.banner.innerHTML = "";
  ui.banner.appendChild(el("span", "ttl", "過去ログ検索中… 「" + term + "」"));

  fetch(url)
    .then((r) => r.json())
    .then(renderHistory)
    .catch((e) => {
      ui.banner.innerHTML = "";
      ui.banner.appendChild(el("span", "ttl", "検索に失敗しました: " + e));
      addBackButton();
    });
}

function renderHistory(res) {
  state.mode = "history";
  ui.table.classList.add("history");
  search.rebuild(ui.search.value);

  ui.banner.hidden = false;
  ui.banner.innerHTML = "";
  const ttl =
    "過去ログ検索: 「" + (res.query || "") + "」 — " + res.hits.length + " 件" +
    (res.truncated ? "（上限で打ち切り）" : "") +
    "  / 走査 " + res.scanned_files + " ファイル";
  ui.banner.appendChild(el("span", "ttl", ttl));
  addBackButton();

  ui.log.innerHTML = "";
  emptyShown = false;
  state.lastDate = null;

  if (!res.hits.length) {
    showEmpty("一致するログが見つかりませんでした");
    state.currentMatch = -1;
    updateMatchInfo();
    return;
  }

  const frag = document.createDocumentFragment();
  res.hits.forEach((h) => frag.appendChild(buildHistoryRow(h)));
  ui.log.appendChild(frag);
  ui.log.scrollTop = 0;

  state.shownCount = res.hits.length;
  state.currentMatch = -1;
  updateCounters();
  updateMatchInfo();
}

/* ---- SSE 接続 -------------------------------------------- */

function connect() {
  const es = new EventSource("/events");
  es.onopen = () => {
    ui.dot.className = "dot live";
    ui.statusText.textContent = "接続中（ライブ）";
    state.flashArmed = false;
    // 接続直後はバックログが一気に流れるので、再生が落ち着いてから点滅を有効化する
    setTimeout(() => { state.flashArmed = true; }, 1000);
  };
  es.onerror = () => {
    ui.dot.className = "dot down";
    ui.statusText.textContent = "切断（再接続中…）";
  };
  es.onmessage = (e) => {
    try {
      onEvent(JSON.parse(e.data));
    } catch (_) {
      /* 壊れた行は無視する */
    }
  };
}

/* ---- UI の配線 ------------------------------------------- */

let debounceTimer;
function onSearchChanged() {
  clearTimeout(debounceTimer);
  debounceTimer = setTimeout(() => {
    if (state.mode === "live") {
      rerenderLive();
    } else {
      search.rebuild(ui.search.value);
      ui.searchBox.classList.toggle("invalid", search.invalid);
    }
  }, SEARCH_DEBOUNCE_MS);
}

function toggleButton(id, get, set) {
  const btn = $(id);
  btn.addEventListener("click", () => {
    set(!get());
    btn.classList.toggle("on", get());
  });
}

document.querySelectorAll("input.kind").forEach((c) =>
  c.addEventListener("change", () => { if (state.mode === "live") rerenderLive(); })
);
ui.only.addEventListener("change", () => { if (state.mode === "live") rerenderLive(); });
ui.search.addEventListener("input", onSearchChanged);
ui.search.addEventListener("keydown", (e) => {
  if (e.key !== "Enter") return;
  e.preventDefault();
  if (e.ctrlKey || e.metaKey) runHistory();
  else if (e.shiftKey) gotoMatch(-1);
  else gotoMatch(1);
});

$("next").addEventListener("click", () => gotoMatch(1));
$("prev").addEventListener("click", () => gotoMatch(-1));
$("histBtn").addEventListener("click", runHistory);

toggleButton("tCase", () => search.caseSensitive, (v) => { search.caseSensitive = v; onSearchChanged(); });
toggleButton("tRegex", () => search.useRegex, (v) => { search.useRegex = v; onSearchChanged(); });
toggleButton("wrap", () => state.wrap, (v) => { state.wrap = v; ui.log.classList.toggle("wrap", v); });

$("autoscroll").addEventListener("click", () => {
  state.autoscroll = !state.autoscroll;
  $("autoscroll").classList.toggle("on", state.autoscroll);
  if (state.autoscroll && state.mode === "live") ui.log.scrollTop = ui.log.scrollHeight;
});

ui.pause.addEventListener("click", () => {
  state.paused = !state.paused;
  ui.pause.classList.toggle("on", state.paused);
  if (state.paused) {
    ui.pause.textContent = "再開";
  } else {
    ui.pause.textContent = "一時停止";
    state.pendingWhilePaused = 0;
    if (state.mode === "live") rerenderLive();
  }
});

$("clear").addEventListener("click", () => {
  state.events = [];
  state.pendingWhilePaused = 0;
  ui.count.textContent = "0";
  if (state.paused) ui.pause.textContent = "再開";
  state.mode = "live";
  ui.banner.hidden = true;
  ui.table.classList.remove("history");
  showEmpty("クリアしました。新しいイベントを待機中…");
  state.currentMatch = -1;
  updateMatchInfo();
});

connect();
