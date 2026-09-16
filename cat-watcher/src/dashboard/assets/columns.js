// ヘッダーの境界をドラッグして列幅を変える。
//
// 「対象」と「内容」のどちらを広く取りたいかは見ている作業によって変わるので、
// 既定の配分を押し付けずに手で寄せられるようにする。結果は localStorage に
// 残して次に開いたときも引き継ぐ。
//
// 幅は CSS 変数 `--cols-user` として `.table` に載せる。`--cols` を直に
// 上書きしないのは、過去ログ表示（`.table.history`）が別の列構成を持っていて、
// そちらまで巻き込まないようにするため。style.css 側で
// `--cols: var(--cols-user, <既定>)` と受けている。

const KEY = "catwatcher.cols";

/** 最後の列は残り幅を取らせたいので、px 指定するのは最後の 1 つ手前まで。 */
const MIN_PX = 40;

/**
 * ドラッグを配線する。
 * @param {HTMLElement} table  `.table`（CSS 変数を載せる先）
 * @param {HTMLElement} thead  `.thead`（`.th` が並んでいる）
 */
export function initColumns(table, thead) {
  const ths = [...thead.querySelectorAll(".th")];
  // 最後の列は「残り全部」なので、境界は列数 - 1 本
  ths.slice(0, -1).forEach((th, index) => {
    const grip = document.createElement("span");
    grip.className = "grip";
    grip.title = "ドラッグで列幅を変更（ダブルクリックで既定に戻す）";
    th.appendChild(grip);
    grip.addEventListener("pointerdown", (e) => startDrag(e, table, thead, index));
    grip.addEventListener("dblclick", (e) => {
      e.preventDefault();
      reset(table);
    });
  });

  restore(table);
}

/** 現在の実測幅を px の配列にする。CSS 側が fr や minmax でも数値になる。 */
function measure(thead) {
  return [...thead.querySelectorAll(".th")].map((th) => Math.round(th.getBoundingClientRect().width));
}

function apply(table, widths) {
  // 最後の列だけ 1fr にして、ウィンドウを広げたときに余りを吸わせる
  const cols = widths.slice(0, -1).map((w) => w + "px").concat("minmax(120px, 1fr)");
  table.style.setProperty("--cols-user", cols.join(" "));
}

function startDrag(e, table, thead, index) {
  e.preventDefault();
  const widths = measure(thead);
  const startX = e.clientX;
  const startW = widths[index];
  document.body.classList.add("resizing");
  e.target.setPointerCapture(e.pointerId);

  const onMove = (m) => {
    widths[index] = Math.max(MIN_PX, startW + (m.clientX - startX));
    apply(table, widths);
  };
  const onUp = () => {
    document.body.classList.remove("resizing");
    e.target.removeEventListener("pointermove", onMove);
    e.target.removeEventListener("pointerup", onUp);
    save(widths);
  };
  e.target.addEventListener("pointermove", onMove);
  e.target.addEventListener("pointerup", onUp);
}

function reset(table) {
  table.style.removeProperty("--cols-user");
  try {
    localStorage.removeItem(KEY);
  } catch (_) {
    /* プライベートウィンドウなどでは保存できない。表示は効いているので無視する */
  }
}

function save(widths) {
  try {
    localStorage.setItem(KEY, JSON.stringify(widths));
  } catch (_) {
    /* 同上 */
  }
}

function restore(table) {
  let saved;
  try {
    saved = JSON.parse(localStorage.getItem(KEY) || "null");
  } catch (_) {
    return;
  }
  // 列を増減させたあとに古い設定が残っていると列がずれる。長さで弾く。
  const expected = table.querySelectorAll(".thead .th").length;
  if (!Array.isArray(saved) || saved.length !== expected) return;
  if (!saved.every((w) => typeof w === "number" && w >= MIN_PX)) return;
  apply(table, saved);
}
