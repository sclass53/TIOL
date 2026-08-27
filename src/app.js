// Tauri v2 global API (withGlobalTauri) — static frontend, no bundler
const { invoke, convertFileSrc } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { open: openDialog } = window.__TAURI__.dialog;

// Frontend instrumentation: every JS error / rejected promise / thumbnail
// failure is reported to the backend log buffer (visible in the debug-mode
// panel and stderr) — the only way to debug UI failures without a console.
function reportJs(kind, message) {
  try {
    invoke("report_js_event", { kind, message: String(message).slice(0, 500) });
  } catch (e) {
    /* never block the UI on reporting */
  }
}
window.addEventListener("error", (e) => {
  reportJs("error", `${e.message || e.error} @ ${e.filename || "?"}:${e.lineno || "?"}`);
});
window.addEventListener("unhandledrejection", (e) => {
  const r = e.reason;
  reportJs("rejection", (r && (r.stack || r.message || r)) || String(r));
});

import {
  t,
  setLanguage,
  initI18n,
  currentLang,
  applyStaticI18n,
  onLanguageChange,
} from "./i18n.js";

const els = {
  navPhotos: document.getElementById("nav-photos"),
  navFolders: document.getElementById("nav-folders"),
  navTags: document.getElementById("nav-tags"),
  navSettings: document.getElementById("nav-settings"),
  viewPhotos: document.getElementById("view-photos"),
  viewFolders: document.getElementById("view-folders"),
  viewTags: document.getElementById("view-tags"),
  viewSettings: document.getElementById("view-settings"),
  langOptions: document.getElementById("lang-options"),
  toggleHwDecode: document.getElementById("toggle-hw-decode"),
  hwDecodeHint: document.getElementById("hw-decode-hint"),
  btnRestart: document.getElementById("btn-restart"),
  gpuStatus: document.getElementById("gpu-status"),
  btnClearCache: document.getElementById("btn-clear-cache"),
  btnClearTags: document.getElementById("btn-clear-tags"),
  btnRunTagging: document.getElementById("btn-run-tagging"),
  taggingStatus: document.getElementById("tagging-status"),
  cacheHint: document.getElementById("cache-hint"),
  confirmOverlay: document.getElementById("confirm-overlay"),
  confirmText: document.getElementById("confirm-text"),
  confirmOk: document.getElementById("confirm-ok"),
  confirmCancel: document.getElementById("confirm-cancel"),
  taggingBadge: document.getElementById("tagging-badge"),
  taggingTitle: document.querySelector(".tagging-badge__title"),
  taggingFill: document.getElementById("tagging-fill"),
  taggingCount: document.getElementById("tagging-count"),
  editOverlay: document.getElementById("edit-overlay"),
  editInput: document.getElementById("edit-input"),
  editSave: document.getElementById("edit-save"),
  editCancel: document.getElementById("edit-cancel"),
  searchInput: document.getElementById("search-input"),
  searchMode: document.getElementById("search-mode"),
  semanticSearchInput: document.getElementById("semantic-search-input"),
  btnSelectMode: document.getElementById("btn-select-mode"),
  selectionBar: document.getElementById("selection-bar"),
  selectionCount: document.getElementById("selection-count"),
  selectionHint: document.getElementById("selection-hint"),
  btnSelectionTag: document.getElementById("btn-selection-tag"),
  btnSelectionClearTags: document.getElementById("btn-selection-clear-tags"),
  btnSelectionCancel: document.getElementById("btn-selection-cancel"),
  tagpickOverlay: document.getElementById("tagpick-overlay"),
  tagpickList: document.getElementById("tagpick-list"),
  tagpickCancel: document.getElementById("tagpick-cancel"),
  editChips: document.getElementById("edit-chips"),
  editSuggest: document.getElementById("edit-suggest"),
  photoGrid: document.getElementById("photo-grid"),
  photoStatus: document.getElementById("photo-status"),
  folderList: document.getElementById("folder-list"),
  folderStatus: document.getElementById("folder-status"),
  btnAdd: document.getElementById("btn-add-folder"),
  btnRefresh: document.getElementById("btn-refresh"),
};

// ---------------------------------------------------------------------------
// Photo grid: chunked rendering + lazy thumbnails (LIMITS.md §5.5)
// ---------------------------------------------------------------------------
let currentPhotos = [];
let renderedCount = 0;
const CHUNK_APPEND = 100; // cards appended per scroll fill

function switchView(name) {
  const isPhotos = name === "photos";
  const isFolders = name === "folders";
  const isTags = name === "tags";
  els.viewPhotos.classList.toggle("view--hidden", !isPhotos);
  els.viewFolders.classList.toggle("view--hidden", !isFolders);
  els.viewTags.classList.toggle("view--hidden", !isTags);
  els.viewSettings.classList.toggle("view--hidden", name !== "settings");
  els.navPhotos.classList.toggle("sidebar__btn--active", isPhotos);
  els.navFolders.classList.toggle("sidebar__btn--active", isFolders);
  els.navTags.classList.toggle("sidebar__btn--active", isTags);
  els.navSettings.classList.toggle("sidebar__btn--active", name === "settings");
  // Leaving the photos view exits multi-select mode (C-13).
  if (!isPhotos && selectMode) setSelectMode(false);
  // Defer to next frame so the unhidden view has settled before measuring.
  if (isPhotos) requestAnimationFrame(fillGridIfNeeded);
}

els.navPhotos.addEventListener("click", () => {
  switchView("photos");
  // Re-fetch so cards show freshly computed tags (stale-tag fix).
  if (!els.searchInput.value.trim() && !els.semanticSearchInput.value.trim()) {
    loadPhotos();
  }
});
els.navFolders.addEventListener("click", () => { switchView("folders"); loadFolders(); });
els.navTags.addEventListener("click", () => { switchView("tags"); renderTags(); });
els.navSettings.addEventListener("click", () => { switchView("settings"); renderSettings(); });

// --- settings view ---
let hwDecodeValue = null; // "1" | "0"
let hwDecodePending = false;

async function renderSettings() {
  if (hwDecodeValue === null) {
    try {
      hwDecodeValue = (await invoke("get_setting", { key: "hw_decode" })) || "0";
    } catch (e) {
      hwDecodeValue = "0";
    }
  }
  const cur = currentLang();
  els.langOptions.querySelectorAll("[data-lang]").forEach((btn) => {
    btn.classList.toggle("btn--active", btn.dataset.lang === cur);
  });
  renderHwDecode();
  detectAndReportRenderer();
  refreshModelStatus();
  renderDebug();
  if (aiProvider === null) {
    try {
      aiProvider = (await invoke("get_setting", { key: "ai_provider" })) || "auto";
    } catch (e) {
      aiProvider = "auto";
    }
  }
  renderAiProvider();
}

function renderHwDecode() {
  const on = hwDecodeValue === "1";
  els.toggleHwDecode.textContent = t(on ? "settings.on" : "settings.off");
  els.toggleHwDecode.classList.toggle("btn--active", on);
  els.hwDecodeHint.hidden = !hwDecodePending;
  els.btnRestart.disabled = !hwDecodePending;
}

els.langOptions.addEventListener("click", (ev) => {
  const btn = ev.target.closest("[data-lang]");
  if (btn) setLanguage(btn.dataset.lang);
});

els.toggleHwDecode.addEventListener("click", async () => {
  const next = hwDecodeValue === "1" ? "0" : "1";
  try {
    await invoke("set_setting", { key: "hw_decode", value: next });
    hwDecodeValue = next;
    hwDecodePending = true;
    renderHwDecode();
  } catch (e) {
    alert(String(e));
  }
});

els.btnRestart.addEventListener("click", () => {
  invoke("restart_app").catch((e) => console.error(e));
});

els.btnClearCache.addEventListener("click", async () => {
  try {
    await invoke("clear_cache");
    // Drop the in-memory thumbnail map and re-render so cards re-request.
    thumbSrcCache.clear();
    renderPhotos(currentPhotos);
    els.cacheHint.textContent = t("settings.cacheCleared");
    els.cacheHint.hidden = false;
    setTimeout(() => {
      els.cacheHint.hidden = true;
    }, 3000);
  } catch (e) {
    alert(String(e));
  }
});

// --- generic confirm dialog (warning before destructive actions) ---
let confirmCallback = null;

function confirmDialog(message, onOk) {
  els.confirmText.textContent = message;
  confirmCallback = onOk;
  els.confirmOverlay.hidden = false;
}
function closeConfirmDialog() {
  confirmCallback = null;
  els.confirmOverlay.hidden = true;
}
els.confirmOk.addEventListener("click", () => {
  const cb = confirmCallback;
  closeConfirmDialog();
  if (cb) cb();
});
els.confirmCancel.addEventListener("click", closeConfirmDialog);
els.confirmOverlay.addEventListener("click", (e) => {
  if (e.target === els.confirmOverlay) closeConfirmDialog();
});

els.btnClearTags.addEventListener("click", () => {
  confirmDialog(t("tags.clearAllConfirm"), async () => {
    try {
      await invoke("clear_all_tags");
      renderTags();
      if (!els.viewPhotos.classList.contains("view--hidden")) loadPhotos();
    } catch (e) {
      alert(String(e));
    }
  });
});

// --- AI progress: settings status line + floating tagging badge ---
// Event: "ai-queue-status" { done, remaining } — remaining = tasks still in
// the queue (grows when new tasks are added, so the badge count follows).
const modelStatusEl = document.getElementById("model-status");
const aiProviderOptions = document.getElementById("ai-provider-options");
let aiProvider = null; // "auto" | "gpu" | "cpu" | "coreml"
let modelBaseText = "…"; // status without the inference-progress suffix
let aiProgress = null; // { done, remaining } | null

function setModelStatus(text) {
  modelBaseText = text;
  renderModelStatus();
}
function renderModelStatus() {
  let text = modelBaseText;
  if (aiProgress && aiProgress.remaining > 0) {
    text += ` · ${t("settings.aiProgress", {
      done: aiProgress.done,
      remaining: aiProgress.remaining,
    })}`;
  }
  modelStatusEl.textContent = text;
}
listen("ai-queue-status", (ev) => {
  const d = ev.payload || {};
  const remaining = d.remaining || 0;
  const done = d.done || 0;
  aiProgress = d;
  renderModelStatus();
  // Floating badge (top-right): visible while background work remains.
  // "Tagging" vs "Indexing" depends on whether user tags exist (the engine
  // also embeds photos — that is indexing, not tagging).
  els.taggingBadge.hidden = remaining <= 0;
  if (remaining > 0) {
    const tagging = !!d.tagging;
    els.taggingTitle.textContent = t(tagging ? "tagging.badge" : "tagging.indexing");
    els.taggingCount.textContent = t(tagging ? "tagging.remaining" : "tagging.indexingRemaining", {
      count: remaining,
    });
    const total = done + remaining;
    els.taggingFill.style.width =
      total > 0 ? `${Math.round((done / total) * 100)}%` : "0%";
  }
  // Queue drained: refresh card tags (photos view) AND the settings tag
  // match counts, so they never stay stale after a tagging batch.
  if (remaining <= 0) {
    renderTags();
    if (!els.viewPhotos.classList.contains("view--hidden")) {
      if (!els.searchInput.value.trim() && !els.semanticSearchInput.value.trim()) {
        loadPhotos();
      }
    }
  }
});

function backendLabel(status) {
  // status: "locked:cuda" | "locked:directml" | "locked:coreml" | "locked:cpu"
  const b = status.split(":")[1];
  if (!b) return t("settings.modelLocked");
  return `${t("settings.modelLocked")} (${b.toUpperCase()})`;
}
function modelStatusText(status) {
  if (status.startsWith("locked")) return backendLabel(status);
  if (status.startsWith("degraded")) return status.replace("degraded: ", t("settings.modelError") + " — ");
  switch (status) {
    case "downloading":
      return t("settings.modelDownloading");
    case "error":
      return t("settings.modelError");
    default:
      return status;
  }
}
listen("model-download", (ev) => {
  const d = ev.payload || {};
  if (d.status === "locked") {
    setModelStatus(t("settings.modelLocked"));
  } else if (d.status === "downloading") {
    const pct = d.progress !== undefined ? ` ${Math.round(d.progress * 100)}%` : "";
    setModelStatus(`${t("settings.modelDownloading")} ${d.file_name || ""}${pct}`.trim());
  } else {
    setModelStatus(d.message || d.status || "");
  }
});
async function refreshModelStatus() {
  try {
    const s = await invoke("get_ai_status");
    setModelStatus(modelStatusText(s));
  } catch (e) {
    /* keep default */
  }
}

function renderAiProvider() {
  aiProviderOptions.querySelectorAll("[data-provider]").forEach((btn) => {
    btn.classList.toggle("btn--active", btn.dataset.provider === aiProvider);
  });
}
aiProviderOptions.addEventListener("click", async (ev) => {
  const btn = ev.target.closest("[data-provider]");
  if (!btn || btn.dataset.provider === aiProvider) return;
  try {
    await invoke("set_ai_provider", { provider: btn.dataset.provider });
    aiProvider = btn.dataset.provider;
    renderAiProvider();
    refreshModelStatus();
  } catch (e) {
    alert(String(e));
  }
});

// --- Debug mode: in-app log panel (get_logs / set_debug_mode) ---
const debugEls = {
  toggle: document.getElementById("toggle-debug"),
  log: document.getElementById("debug-log"),
};
let debugValue = null; // "1" | "0"
let debugMode = false; // live flag: gates AI-confidence badges on cards
let logPollTimer = null;

async function renderDebug() {
  if (debugValue === null) {
    try {
      debugValue = (await invoke("get_setting", { key: "debug" })) || "0";
    } catch (e) {
      debugValue = "0";
    }
  }
  const on = debugValue === "1";
  debugMode = on;
  debugEls.toggle.textContent = t(on ? "settings.on" : "settings.off");
  debugEls.toggle.classList.toggle("btn--active", on);
  debugEls.log.hidden = !on;
  if (on) {
    startLogPolling();
  } else {
    stopLogPolling();
  }
}

debugEls.toggle.addEventListener("click", async () => {
  const next = debugValue === "1" ? "0" : "1";
  try {
    await invoke("set_debug_mode", { enabled: next === "1" });
    debugValue = next;
    await renderDebug();
    // Re-render cards so AI-confidence badges appear/disappear right away.
    renderPhotos(currentPhotos);
  } catch (e) {
    alert(String(e));
  }
});

async function pollLogs() {
  if (debugValue !== "1" || debugEls.log.hidden) return;
  try {
    const lines = await invoke("get_logs", { limit: 300 });
    const pre = debugEls.log;
    const stick =
      pre.scrollTop + pre.clientHeight >= pre.scrollHeight - 24;
    pre.textContent = lines.join("\n");
    if (stick) pre.scrollTop = pre.scrollHeight;
  } catch (e) {
    /* keep last snapshot */
  }
}
function startLogPolling() {
  stopLogPolling();
  pollLogs();
  logPollTimer = setInterval(pollLogs, 1000);
}
function stopLogPolling() {
  if (logPollTimer) {
    clearInterval(logPollTimer);
    logPollTimer = null;
  }
}

// --- Custom tag management (Tags tab, C-12) ---
const tagEls = {
  input: document.getElementById("tag-input"),
  threshold: document.getElementById("tag-threshold"),
  add: document.getElementById("btn-add-tag"),
  list: document.getElementById("tag-list"),
};

async function renderTags() {
  let tags = [];
  try {
    tags = await invoke("get_custom_tags");
  } catch (e) {
    reportJs("get-tags", String(e));
  }
  tagEls.list.textContent = "";
  if (!tags.length) {
    const li = document.createElement("li");
    li.className = "tags__empty";
    li.textContent = t("tags.empty");
    tagEls.list.appendChild(li);
    return;
  }
  for (const tg of tags) {
    const li = document.createElement("li");
    li.className = "tags__item";
    const name = document.createElement("span");
    name.className = "tags__name";
    name.textContent = tg.name;
    const meta = document.createElement("span");
    meta.className = "tags__meta";
    meta.textContent = `${t("tags.tagThreshold")}: ${Number(tg.threshold).toFixed(2)} · ${t("tags.tagCount", { count: tg.photo_count })}`;
    const del = document.createElement("button");
    del.className = "btn btn--ghost";
    del.textContent = t("tags.removeTag");
    del.addEventListener("click", async () => {
      try {
        await invoke("delete_custom_tag", { id: tg.id });
        renderTags();
      } catch (e) {
        alert(String(e));
      }
    });
    li.appendChild(name);
    li.appendChild(meta);
    li.appendChild(del);
    tagEls.list.appendChild(li);
  }
}

tagEls.add.addEventListener("click", async () => {
  const name = tagEls.input.value.trim();
  const threshold = parseFloat(tagEls.threshold.value) || 0.06;
  if (!name) {
    alert(t("tags.nameRequired"));
    return;
  }
  try {
    await invoke("add_custom_tag", { name, threshold });
    tagEls.input.value = "";
    renderTags();
  } catch (e) {
    alert(String(e));
  }
});
tagEls.input.addEventListener("keydown", (e) => {
  if (e.key === "Enter") tagEls.add.click();
});

// "AI Tagging" (C-12): the ONLY way tagging starts. Queues a full pass over
// every photo missing any current tag (new tags & new files included).
let taggingStatusTimer = null;
els.btnRunTagging.addEventListener("click", async () => {
  try {
    const n = await invoke("run_ai_tagging");
    els.taggingStatus.textContent = t("tags.runStarted", { count: n });
    els.taggingStatus.hidden = false;
    clearTimeout(taggingStatusTimer);
    taggingStatusTimer = setTimeout(() => {
      els.taggingStatus.hidden = true;
    }, 6000);
  } catch (e) {
    const msg = String(e);
    if (msg.includes("no tags defined")) {
      els.taggingStatus.textContent = t("tags.runNoTags");
      els.taggingStatus.hidden = false;
      clearTimeout(taggingStatusTimer);
      taggingStatusTimer = setTimeout(() => {
        els.taggingStatus.hidden = true;
      }, 6000);
    } else {
      alert(msg);
    }
  }
});

// --- GPU renderer status (verifies hardware decoding took effect) ---
async function detectAndReportRenderer() {
  let renderer = null;
  try {
    const canvas = document.createElement("canvas");
    const gl =
      canvas.getContext("webgl") || canvas.getContext("experimental-webgl");
    if (gl) {
      const ext = gl.getExtension("WEBGL_debug_renderer_info");
      if (ext) {
        const r = gl.getParameter(ext.UNMASKED_RENDERER_WEBGL);
        const v = gl.getParameter(ext.UNMASKED_VENDOR_WEBGL);
        renderer = `${v} / ${r}`;
      } else {
        renderer = "WebGL available";
      }
    }
  } catch (e) {
    renderer = null;
  }
  const isSoftware = !!renderer && /swiftshader/i.test(renderer);
  els.gpuStatus.textContent = renderer
    ? t("settings.gpu", { renderer }) + (isSoftware ? ` ${t("settings.gpuSoftware")}` : "")
    : t("settings.gpuUnknown");
  try {
    await invoke("report_renderer", { renderer: renderer || "unknown" });
  } catch (e) {
    console.error(e);
  }
}

// --- chunked rendering ---
function cardsPerRow() {
  const g = els.photoGrid;
  const gap = 12;
  const cardW = 180;
  const contentW = (g.clientWidth || 1200) - 32; // 16px padding each side
  return Math.max(1, Math.floor((contentW + gap) / (cardW + gap)));
}

function renderPhotos(photos) {
  currentPhotos = photos;
  renderedCount = 0;
  els.photoGrid.innerHTML = "";
  thumbObserver.disconnect();
  // Queued entries reference cards from the previous render — drop them.
  thumbQueue.length = 0;
  if (!photos.length) {
    els.photoGrid.innerHTML = `<div class="empty">${t(hasActiveFilters() ? "photos.filterEmpty" : "photos.empty")}</div>`;
    els.photoStatus.textContent = t("photos.status.count", { count: 0 });
    return;
  }
  els.photoGrid.scrollTop = 0;
  // Initial render: exactly the top 5 rows (in order). Further rows are
  // rendered on scroll / viewport fill.
  renderChunk(cardsPerRow() * 5);
  // Deterministic initial thumbnail load: explicitly enqueue the first
  // screenful top-down (the observer's initial callback proved unreliable
  // for cards already in the DOM — it skipped the first rows).
  // Iterate BOTTOM-UP: enqueueThumb unshifts to the queue head, so the last
  // processed card would win the front; reversed order keeps card 0 (top
  // row) first. _initial is set AFTER enqueueing so the enqueue is not
  // blocked, then the observer/click can no longer re-prioritize these.
  // One bad card must never kill the whole screenful — per-card try/catch,
  // and a card that failed to enqueue stays unmarked so click/observer can
  // retry it.
  for (let i = renderedCount - 1; i >= 0; i--) {
    const card = els.photoGrid.children[i];
    const img = card && card._img;
    if (!img || !card._photo) continue;
    try {
      enqueueThumb(img, card._photo);
      img._initial = true;
    } catch (e) {
      reportJs("enqueue", String(e));
    }
  }
  // Pump unconditionally (no-op on an empty queue) so a zero-return from
  // enqueueThumb can never leave the screenful stuck unserved.
  pumpThumbs();
}

function renderChunk(limit = CHUNK_APPEND) {
  const total = currentPhotos.length;
  if (renderedCount >= total) return;
  const end = Math.min(renderedCount + limit, total);
  for (let i = renderedCount; i < end; i++) {
    els.photoGrid.appendChild(buildCard(currentPhotos[i]));
  }
  renderedCount = end;
  els.photoStatus.textContent =
    renderedCount < total
      ? t("photos.status.partial", { shown: renderedCount, total })
      : t("photos.status.count", { count: total });
}

// ---------------------------------------------------------------------------
// Multi-select mode (phone-gallery style, C-13): toolbar button toggles it;
// click toggles one card, drag over the grid rubber-bands a range. Selected
// photos can get ONE existing tag appended via the bottom bar -> picker.
// ---------------------------------------------------------------------------
let selectMode = false;
const selectedIds = new Set();

function setSelectMode(on) {
  if (selectMode === on) return;
  selectMode = on;
  els.btnSelectMode.textContent = t(on ? "photos.selectDone" : "photos.selectMode");
  els.btnSelectMode.classList.toggle("searchbar__select--active", on);
  els.photoGrid.classList.toggle("selecting", on);
  els.selectionBar.hidden = !on;
  if (!on) {
    selectedIds.clear();
    hideSelectionHint();
  }
  // Update already-rendered cards in place (no re-render: keeps scroll pos).
  for (const card of els.photoGrid.children) {
    if (!card._photo) continue;
    card.classList.toggle("card--selected", selectedIds.has(card._photo.id));
    const cb = card.querySelector(".card__check");
    if (cb) cb.hidden = !on;
  }
  updateSelectionBar();
}

function updateSelectionBar() {
  if (!selectMode) return;
  const n = selectedIds.size;
  els.selectionCount.textContent = t("photos.selectedCount", { count: n });
  els.btnSelectionTag.disabled = n === 0;
  els.btnSelectionClearTags.disabled = n === 0;
}

function toggleSelect(photo) {
  if (selectedIds.has(photo.id)) selectedIds.delete(photo.id);
  else selectedIds.add(photo.id);
  if (photo._card) {
    photo._card.classList.toggle("card--selected", selectedIds.has(photo.id));
  }
  updateSelectionBar();
}

let selectionHintTimer = null;
function showSelectionHint(text) {
  els.selectionHint.textContent = text;
  els.selectionHint.hidden = false;
  clearTimeout(selectionHintTimer);
  selectionHintTimer = setTimeout(hideSelectionHint, 2500);
}
function hideSelectionHint() {
  els.selectionHint.hidden = true;
}

els.btnSelectMode.addEventListener("click", () => setSelectMode(!selectMode));
els.btnSelectionCancel.addEventListener("click", () => setSelectMode(false));

// "Delete tags" (red): strip ALL tags (text + colors) from the selection.
els.btnSelectionClearTags.addEventListener("click", () => {
  const n = selectedIds.size;
  confirmDialog(t("photos.deleteTagsConfirm", { count: n }), async () => {
    const ids = [...selectedIds];
    try {
      await invoke("clear_tags_from_files", { fileIds: ids });
      showSelectionHint(t("photos.tagsDeleted", { count: ids.length }));
      // Update the visible cards in place (keeps scroll position).
      for (const p of currentPhotos) {
        if (!selectedIds.has(p.id)) continue;
        p.tags = [];
        p.colors = [];
        if (p._card) renderCardMeta(p._card, p);
      }
    } catch (e) {
      alert(String(e));
    }
  });
});

// Esc leaves select mode (after closing any open overlay first).
document.addEventListener("keydown", (e) => {
  if (e.key !== "Escape" || !selectMode) return;
  if (!els.tagpickOverlay.hidden) {
    els.tagpickOverlay.hidden = true;
    return;
  }
  if (!els.editOverlay.hidden) {
    closeEditDialog(false);
    return;
  }
  if (!preview.els.overlay.hidden) {
    preview.close();
    return;
  }
  setSelectMode(false);
});

// Drag-to-select: press anywhere on the grid and drag — cards intersecting
// the rubber band are selected on release. A plain click (no drag) falls
// through to the card click handler which toggles that one card.
let dragSel = null;
els.photoGrid.addEventListener("mousedown", (e) => {
  if (!selectMode || e.button !== 0) return;
  if (e.target.closest("button, input, select, .card__edit")) return;
  const gridRect = els.photoGrid.getBoundingClientRect();
  // Don't hijack the vertical scrollbar.
  if (e.clientX > gridRect.left + gridRect.width - 16) return;
  e.preventDefault(); // no text selection / native image drag
  const box = document.createElement("div");
  box.className = "selection-box";
  els.photoGrid.appendChild(box);
  const startX = e.clientX - gridRect.left;
  const startY = e.clientY - gridRect.top;
  dragSel = { startX, startY, box, gridRect, moved: false, last: null };
  const onMove = (ev) => {
    if (!dragSel) return;
    const x = ev.clientX - dragSel.gridRect.left;
    const y = ev.clientY - dragSel.gridRect.top;
    const w = x - dragSel.startX;
    const h = y - dragSel.startY;
    if (Math.abs(w) > 4 || Math.abs(h) > 4) dragSel.moved = true;
    if (!dragSel.moved) return; // keep the box hidden until a real drag
    const L = Math.min(dragSel.startX, x);
    const T = Math.min(dragSel.startY, y);
    const R = Math.max(dragSel.startX, x);
    const B = Math.max(dragSel.startY, y);
    dragSel.last = { L, T, R, B };
    dragSel.box.style.left = L + "px";
    dragSel.box.style.top = T + "px";
    dragSel.box.style.width = R - L + "px";
    dragSel.box.style.height = B - T + "px";
    // Live highlight of intersecting cards (grid-relative coords).
    for (const card of els.photoGrid.children) {
      if (!card._photo) continue;
      const r = card.getBoundingClientRect();
      const hit =
        r.left - dragSel.gridRect.left < R &&
        r.right - dragSel.gridRect.left > L &&
        r.top - dragSel.gridRect.top < B &&
        r.bottom - dragSel.gridRect.top > T;
      card.classList.toggle("card--sel-hover", hit);
    }
  };
  const onUp = () => {
    window.removeEventListener("mousemove", onMove);
    window.removeEventListener("mouseup", onUp);
    const d = dragSel;
    dragSel = null;
    box.remove();
    for (const card of els.photoGrid.children) {
      card.classList.remove("card--sel-hover");
    }
    if (!d || !d.moved) return; // plain click → card click toggles it
    const { L, T, R, B } = d.last;
    for (const card of els.photoGrid.children) {
      if (!card._photo) continue;
      const r = card.getBoundingClientRect();
      const hit =
        r.left - d.gridRect.left < R &&
        r.right - d.gridRect.left > L &&
        r.top - d.gridRect.top < B &&
        r.bottom - d.gridRect.top > T;
      if (hit && !selectedIds.has(card._photo.id)) {
        selectedIds.add(card._photo.id);
        card.classList.add("card--selected");
      }
    }
    updateSelectionBar();
  };
  window.addEventListener("mousemove", onMove);
  window.addEventListener("mouseup", onUp);
});

// "Add tag" for the selection: pick ONE existing tag, appended to all
// selected photos (manual tags, existing tags untouched). The panel has a
// live search box for large tag sets (C-13.3).
let tagpickAll = [];
const tagpickSearch = document.getElementById("tagpick-search");

function renderTagPickList() {
  els.tagpickList.textContent = "";
  const q = tagpickSearch.value.trim().toLowerCase();
  const shown = tagpickAll.filter((n) => !q || n.toLowerCase().includes(q));
  if (!tagpickAll.length) {
    const d = document.createElement("div");
    d.className = "tagpick-empty";
    d.textContent = t("tags.pickEmpty");
    els.tagpickList.appendChild(d);
    return;
  }
  if (!shown.length) {
    const d = document.createElement("div");
    d.className = "tagpick-empty";
    d.textContent = t("tags.pickNoMatch");
    els.tagpickList.appendChild(d);
    return;
  }
  for (const n of shown) {
    const btn = document.createElement("button");
    btn.className = "btn btn--ghost tagpick-item";
    btn.textContent = n;
    btn.addEventListener("click", async () => {
      const ids = [...selectedIds];
      els.tagpickOverlay.hidden = true;
      try {
        await invoke("add_tags_to_files", { fileIds: ids, tags: [n] });
        showSelectionHint(t("photos.tagsAdded", { count: ids.length, tag: n }));
        // Update card tag lines in place (no re-render: keeps scroll pos).
        for (const p of currentPhotos) {
          if (selectedIds.has(p.id) && !p.tags.includes(n)) {
            p.tags.push(n);
            if (p._card) renderCardMeta(p._card, p);
          }
        }
      } catch (e) {
        alert(String(e));
      }
    });
    els.tagpickList.appendChild(btn);
  }
}
tagpickSearch.addEventListener("input", renderTagPickList);

els.btnSelectionTag.addEventListener("click", async () => {
  if (!selectedIds.size) return;
  try {
    tagpickAll = await invoke("get_all_tags");
  } catch (e) {
    alert(String(e));
    return;
  }
  tagpickSearch.value = "";
  renderTagPickList();
  els.tagpickOverlay.hidden = false;
});
els.tagpickCancel.addEventListener("click", () => {
  els.tagpickOverlay.hidden = true;
});
els.tagpickOverlay.addEventListener("click", (e) => {
  if (e.target === els.tagpickOverlay) els.tagpickOverlay.hidden = true;
});

// ---------------------------------------------------------------------------
// Color labels (C-14): applied from the selection-bar dots, shown as dots on
// the cards (right of the filename), filterable from the search bar (union).
// ---------------------------------------------------------------------------
const COLOR_ORDER = ["red", "orange", "yellow", "green", "blue", "purple"];
const COLOR_HEX = {
  red: "#ff3b30",
  orange: "#ff9500",
  yellow: "#ffcc00",
  green: "#34c759",
  blue: "#0a84ff",
  purple: "#af52de",
};

// The unfiltered result of the current query — filters re-apply to it.
let allPhotos = [];

function hasActiveFilters() {
  return (
    activeColorFilters.size > 0 ||
    activeLensFilters.size > 0 ||
    focalMin != null ||
    focalMax != null
  );
}

/// Combined filter (C-14/C-15): color labels ∩ lens ∩ focal range — all
/// intersections. Per the C-15 rule, a photo WITHOUT the required EXIF data
/// (lens or focal) fails that filter.
function applyFilters(photos) {
  return photos.filter((p) => {
    if (
      activeColorFilters.size &&
      ![...activeColorFilters].some((c) => (p.colors || []).includes(c))
    ) {
      return false;
    }
    if (activeLensFilters.size) {
      const pLens = (p.lens || "").trim();
      if (!(pLens && activeLensFilters.has(pLens))) {
        return false;
      }
    }
    if (focalMin != null || focalMax != null) {
      if (p.focal_length == null) return false;
      if (focalMin != null && p.focal_length < focalMin) return false;
      if (focalMax != null && p.focal_length > focalMax) return false;
    }
    return true;
  });
}

function updateFilterButton() {
  btnColorFilter.classList.toggle("searchbar__filter--active", hasActiveFilters());
}

// Selection-bar dots: one per color; clicking applies/toggles it on ALL
// selected photos (phone-gallery semantics, handled by toggle_color_tag).
const selectionColorDots = document.getElementById("selection-color-dots");
for (const c of COLOR_ORDER) {
  const dot = document.createElement("button");
  dot.className = "color-dot color-dot--sel";
  dot.style.background = COLOR_HEX[c];
  dot.title = t(`colors.${c}`);
  dot.addEventListener("click", async () => {
    if (!selectedIds.size) return;
    const ids = [...selectedIds];
    try {
      const all = await invoke("toggle_color_tag", { fileIds: ids, color: c });
      // Update the visible cards in place (keeps scroll position).
      for (const p of currentPhotos) {
        if (!selectedIds.has(p.id)) continue;
        const cs = p.colors || [];
        if (all && !cs.includes(c)) p.colors = [...cs, c];
        else if (!all && cs.includes(c)) p.colors = cs.filter((x) => x !== c);
        if (p._card) renderCardMeta(p._card, p);
      }
      dot.classList.add("color-dot--pulse");
      setTimeout(() => dot.classList.remove("color-dot--pulse"), 350);
    } catch (e) {
      alert(String(e));
    }
  });
  selectionColorDots.appendChild(dot);
}

// Search-bar filter panel (C-14/C-15): color dots (union) + lens list
// (union) + focal range — all three intersect each other.
const activeColorFilters = new Set();
const activeLensFilters = new Set();
let focalMin = null; // mm, null = inactive
let focalMax = null;
const btnColorFilter = document.getElementById("btn-color-filter");
const colorFilterPanel = document.getElementById("color-filter");
const colorFilterDots = document.getElementById("color-filter-dots");
const colorFilterLens = document.getElementById("color-filter-lens");
const filterFocalMin = document.getElementById("filter-focal-min");
const filterFocalMax = document.getElementById("filter-focal-max");

function renderFilterDots() {
  colorFilterDots.textContent = "";
  for (const c of COLOR_ORDER) {
    const dot = document.createElement("button");
    dot.className =
      "color-dot color-dot--filter" +
      (activeColorFilters.has(c) ? " color-dot--on" : "");
    dot.style.background = COLOR_HEX[c];
    dot.title = t(`colors.${c}`);
    dot.addEventListener("click", () => {
      if (activeColorFilters.has(c)) activeColorFilters.delete(c);
      else activeColorFilters.add(c);
      renderFilterDots();
      updateFilterButton();
      renderPhotos(applyFilters(allPhotos));
    });
    colorFilterDots.appendChild(dot);
  }
}
renderFilterDots();

// Lens list (cached per session; lens set only changes with new photos).
let lensCache = null;
async function ensureLensList() {
  if (lensCache) return lensCache;
  try {
    lensCache = await invoke("get_lens_list");
  } catch (e) {
    reportJs("get-lenses", String(e));
    lensCache = [];
  }
  return lensCache;
}

async function renderFilterLens() {
  const lenses = await ensureLensList();
  colorFilterLens.textContent = "";
  if (!lenses.length) {
    const d = document.createElement("div");
    d.className = "tagpick-empty";
    d.textContent = t("photos.filterLensEmpty");
    colorFilterLens.appendChild(d);
    return;
  }
  for (const l of lenses) {
    const btn = document.createElement("button");
    btn.className =
      "filter-lens__item" + (activeLensFilters.has(l) ? " filter-lens__item--on" : "");
    btn.textContent = l;
    btn.title = l;
    btn.addEventListener("click", () => {
      if (activeLensFilters.has(l)) activeLensFilters.delete(l);
      else activeLensFilters.add(l);
      renderFilterLens();
      updateFilterButton();
      const filtered = applyFilters(allPhotos);
      renderPhotos(filtered);
      // Diagnostics (dev): if a lens filter kills everything while photos
      // DO carry lens data, report what the frontend actually sees.
      if (
        activeLensFilters.size &&
        !filtered.length &&
        allPhotos.length &&
        allPhotos.some((p) => p.lens)
      ) {
        reportJs(
          "lens-filter",
          JSON.stringify({
            active: [...activeLensFilters],
            sample: allPhotos
              .slice(0, 5)
              .map((p) => ({ id: p.id, lens: p.lens })),
          })
        );
      }
    });
    colorFilterLens.appendChild(btn);
  }
}

function readFocalFilters() {
  const mn = filterFocalMin.value.trim();
  const mx = filterFocalMax.value.trim();
  focalMin = mn === "" ? null : Math.max(0, parseFloat(mn) || 0);
  focalMax = mx === "" ? null : parseFloat(mx) || 0;
}
filterFocalMin.addEventListener("input", () => {
  readFocalFilters();
  updateFilterButton();
  renderPhotos(applyFilters(allPhotos));
});
filterFocalMax.addEventListener("input", () => {
  readFocalFilters();
  updateFilterButton();
  renderPhotos(applyFilters(allPhotos));
});

btnColorFilter.addEventListener("click", async (e) => {
  e.stopPropagation();
  colorFilterPanel.hidden = !colorFilterPanel.hidden;
  if (!colorFilterPanel.hidden) {
    // Anchor the panel under the filter button (it sits between the search
    // boxes, not at the left edge anymore).
    const r = btnColorFilter.getBoundingClientRect();
    colorFilterPanel.style.left = `${Math.max(8, r.left)}px`;
    colorFilterPanel.style.top = `${r.bottom + 6}px`;
    await renderFilterLens();
  }
});
document.getElementById("btn-color-filter-clear").addEventListener("click", () => {
  activeColorFilters.clear();
  activeLensFilters.clear();
  focalMin = null;
  focalMax = null;
  filterFocalMin.value = "";
  filterFocalMax.value = "";
  renderFilterDots();
  renderFilterLens();
  updateFilterButton();
  renderPhotos(applyFilters(allPhotos));
});
document.addEventListener("click", (e) => {
  if (
    !colorFilterPanel.hidden &&
    !e.target.closest("#color-filter, #btn-color-filter")
  ) {
    colorFilterPanel.hidden = true;
  }
});

// Keep filling until the viewport is covered (only while photos view visible).
// Bounded per frame: at most 3 chunks, continue on the next frame, so a large
// library can never block the UI with a synchronous render burst.
function fillGridIfNeeded() {
  if (els.viewPhotos.classList.contains("view--hidden")) return;
  const g = els.photoGrid;
  let passes = 0;
  while (renderedCount < currentPhotos.length && g.scrollHeight <= g.clientHeight + 300) {
    renderChunk();
    if (++passes >= 3) {
      requestAnimationFrame(fillGridIfNeeded);
      return;
    }
  }
}

// When the user scrolls or jumps past the rendered region, keep filling until
// the area they are looking at (plus a safety margin) is covered. Bounded per
// frame so a long scrollbar drag can never block the UI.
let fillScheduled = false;
function scheduleScrollFill() {
  if (fillScheduled) return;
  fillScheduled = true;
  requestAnimationFrame(() => {
    fillScheduled = false;
    scrollToViewport();
  });
}

function scrollToViewport() {
  if (renderedCount >= currentPhotos.length) return;
  const g = els.photoGrid;
  let passes = 0;
  while (
    renderedCount < currentPhotos.length &&
    g.scrollHeight < g.scrollTop + g.clientHeight * 2 + 600
  ) {
    renderChunk();
    if (++passes >= 2) {
      scheduleScrollFill();
      return;
    }
  }
}

els.photoGrid.addEventListener("scroll", () => {
  const g = els.photoGrid;
  if (renderedCount >= currentPhotos.length) return;
  // Viewport bottom is beyond the rendered region -> render the viewed area.
  if (g.scrollTop + g.clientHeight * 2 + 600 > g.scrollHeight) {
    scheduleScrollFill();
  }
});

// --- lazy thumbnails via IntersectionObserver (only near-viewport cards) ---
const THUMB_MAX_INFLIGHT = 4;
let thumbInFlight = 0;
const thumbQueue = []; // { img, photo }
const thumbSrcCache = new Map(); // path -> resolved asset src (thumb or original)

const thumbObserver = new IntersectionObserver(
  (entries) => {
    // Iterate bottom-up: setThumb unshifts to the queue head, so the LAST
    // processed entry would win the front. Reversed order keeps the TOP of
    // the viewport first (natural top-down fill).
    for (let i = entries.length - 1; i >= 0; i--) {
      const entry = entries[i];
      if (!entry.isIntersecting) continue;
      // Keep observing: scrolling back to a card whose thumbnail wasn't
      // served yet re-triggers setThumb, which re-prioritizes it.
      const t = entry.target;
      if (t._img && t._photo) setThumb(t._img, t._photo);
    }
  },
  { root: els.photoGrid, rootMargin: "300px" }
);

function showPlaceholder(img, photo) {
  if (!img.isConnected) return;
  const thumb = img.parentElement;
  if (!thumb) return;
  thumb.textContent = photo.filename;
  thumb.classList.add("card__thumb--placeholder");
  img.remove();
}

/// Enqueue (or serve from cache / re-prioritize) one card's thumbnail.
/// Returns true when a NEW entry was queued (caller decides when to pump).
function enqueueThumb(img, photo) {
  const cached = thumbSrcCache.get(photo.path);
  if (cached !== undefined) {
    if (cached) img.src = cached;
    else showPlaceholder(img, photo);
    return false;
  }
  const idx = thumbQueue.findIndex((q) => q.photo.path === photo.path);
  if (idx >= 0) {
    // Scroll-time cards move to the front (viewport-first). Initial-screenful
    // cards keep their top-down serve order — reprioritizing them reorders
    // the rows (the observer's initial callback did exactly that).
    if (!img._initial) {
      const item = thumbQueue.splice(idx, 1)[0];
      thumbQueue.unshift(item);
    }
    return false;
  }
  // Already handled by the explicit initial load (queued, in flight or
  // served) — never enqueue a duplicate that would jump the queue.
  if (img._initial) return false;
  // New requests go to the front (viewport-first instead of FIFO).
  thumbQueue.unshift({ img, photo });
  return true;
}

function setThumb(img, photo) {
  if (enqueueThumb(img, photo)) pumpThumbs();
}

function pumpThumbs() {
  while (thumbInFlight < THUMB_MAX_INFLIGHT && thumbQueue.length) {
    const { img, photo } = thumbQueue.shift();
    thumbInFlight++;
    invoke("get_thumbnail", { path: photo.path })
      .then((thumbPath) => {
        // Empty result = generation failed (corrupt/unsupported): show the
        // placeholder immediately — never load the original full-size file.
        if (!thumbPath) {
          thumbSrcCache.set(photo.path, "");
          showPlaceholder(img, photo);
          return;
        }
        let src = null;
        try {
          src = convertFileSrc(thumbPath);
        } catch {
          /* keep placeholder */
        }
        thumbSrcCache.set(photo.path, src);
        if (src && img.isConnected) img.src = src;
      })
      .catch((err) => {
        thumbSrcCache.set(photo.path, "");
        showPlaceholder(img, photo);
        reportJs("thumb-fail", `${photo.path}: ${err}`);
      })
      .finally(() => {
        thumbInFlight--;
        pumpThumbs();
      });
  }
}

// --- tag edit dialog (C-13): current tags as chips + picker of existing
// tags to add (one click per tag); a free-text input creates new tags.
let editPhoto = null;
let editManual = []; // manual (source=0) tag names — what Save applies
let editAiTags = []; // AI (source=1) tag names — shown read-only
let editSuggestAll = []; // every existing tag name (get_all_tags)

async function openEditDialog(photo) {
  editPhoto = photo;
  editManual = [];
  editAiTags = [];
  let tags = [];
  try {
    tags = await invoke("get_file_tags", { fileId: photo.id });
  } catch (e) {
    reportJs("get-tags", String(e));
  }
  editManual = (tags || [])
    .filter((tg) => tg.source === 0)
    .map((tg) => tg.name);
  editAiTags = (tags || [])
    .filter((tg) => tg.source === 1)
    .map((tg) => tg.name);
  try {
    editSuggestAll = await invoke("get_all_tags");
  } catch (e) {
    editSuggestAll = [];
  }
  renderEditChips();
  renderEditSuggest();
  els.editInput.value = "";
  els.editOverlay.hidden = false;
  els.editInput.focus();
}

function renderEditChips() {
  els.editChips.textContent = "";
  for (const n of editManual) {
    const chip = document.createElement("span");
    chip.className = "edit-chip";
    const txt = document.createElement("span");
    txt.textContent = n;
    const del = document.createElement("button");
    del.className = "edit-chip__del";
    del.textContent = "×";
    del.title = t("card.edit.remove");
    del.addEventListener("click", () => {
      editManual = editManual.filter((x) => x !== n);
      renderEditChips();
      renderEditSuggest();
    });
    chip.appendChild(txt);
    chip.appendChild(del);
    els.editChips.appendChild(chip);
  }
  for (const n of editAiTags) {
    const chip = document.createElement("span");
    chip.className = "edit-chip edit-chip--ai";
    chip.textContent = n;
    els.editChips.appendChild(chip);
  }
  if (!editManual.length && !editAiTags.length) {
    const d = document.createElement("span");
    d.className = "tagpick-empty";
    d.textContent = t("card.edit.noTags");
    els.editChips.appendChild(d);
  }
}

function renderEditSuggest() {
  els.editSuggest.textContent = "";
  const candidates = editSuggestAll.filter(
    (n) => !editManual.includes(n) && !editAiTags.includes(n)
  );
  if (!candidates.length) {
    const d = document.createElement("div");
    d.className = "tagpick-empty";
    d.textContent = t("card.edit.noSuggest");
    els.editSuggest.appendChild(d);
    return;
  }
  for (const n of candidates) {
    const btn = document.createElement("button");
    btn.className = "edit-suggest__item";
    btn.textContent = `+ ${n}`;
    btn.addEventListener("click", () => {
      if (!editManual.includes(n)) editManual.push(n);
      renderEditChips();
      renderEditSuggest();
    });
    els.editSuggest.appendChild(btn);
  }
}

function addEditTag(name) {
  const n = name.trim();
  if (!n) return;
  if (!editManual.includes(n)) editManual.push(n);
  renderEditChips();
  renderEditSuggest();
}

async function closeEditDialog(save) {
  els.editOverlay.hidden = true;
  if (!save || !editPhoto) {
    editPhoto = null;
    return;
  }
  const p = editPhoto;
  editPhoto = null;
  try {
    const updated = await invoke("update_tags", { fileId: p.id, tags: editManual });
    p.tags = updated.tags || [];
    if (p._card) renderCardMeta(p._card, p);
    runSearch();
  } catch (e) {
    alert(String(e));
  }
}

/// Re-render the meta row of one card (name + color dots + tag list) after a
/// tag/color edit.
function renderCardColors(el, colors) {
  el.textContent = "";
  for (const c of colors || []) {
    const dot = document.createElement("span");
    dot.className = "card__color";
    dot.style.background = COLOR_HEX[c] || "#888";
    dot.title = t(`colors.${c}`);
    el.appendChild(dot);
  }
}

function renderCardMeta(card, p) {
  const meta = card.querySelector(".card__meta");
  if (!meta) return;
  meta.querySelectorAll(".card__desc").forEach((el) => el.remove());
  const colorsEl = card.querySelector(".card__colors");
  if (colorsEl) renderCardColors(colorsEl, p.colors);
  if (p.tags && p.tags.length) {
    const tagsText = p.tags.join(", ");
    const descEl = document.createElement("div");
    descEl.className = "card__desc";
    descEl.textContent = tagsText;
    descEl.title = tagsText;
    meta.appendChild(descEl);
  }
}

els.editSave.addEventListener("click", () => closeEditDialog(true));
els.editCancel.addEventListener("click", () => closeEditDialog(false));
els.editOverlay.addEventListener("click", (e) => {
  if (e.target === els.editOverlay) closeEditDialog(false);
});
// Enter adds the typed tag to the current list (Save applies everything);
// Escape closes without saving.
els.editInput.addEventListener("keydown", (e) => {
  if (e.key === "Enter") {
    addEditTag(els.editInput.value);
    els.editInput.value = "";
  } else if (e.key === "Escape") {
    closeEditDialog(false);
  }
});

function buildCard(p) {
  const card = document.createElement("div");
  card.className = "card";
  const thumb = document.createElement("div");
  thumb.className = "card__thumb";
  const img = document.createElement("img");
  img.alt = p.filename;
  img.loading = "lazy";
  img.draggable = false;
  img.onerror = () => {
    thumb.textContent = p.filename;
    thumb.classList.add("card__thumb--placeholder");
    img.remove();
  };
  thumb.appendChild(img);
  thumb._img = img;
  thumb._photo = p;
  // Multi-select checkbox (top-right, visible in select mode only).
  const check = document.createElement("span");
  check.className = "card__check";
  check.hidden = !selectMode;
  thumb.appendChild(check);
  // Debug-mode AI confidence badge (semantic search fills FileRecord.score).
  if (debugMode && p.score != null) {
    const badge = document.createElement("span");
    badge.className = "card__score";
    badge.textContent = `AI ${p.score.toFixed(3)}`;
    thumb.appendChild(badge);
  }
  thumbObserver.observe(thumb);
  card._photo = p;
  card._img = img;
  card.classList.toggle("card--selected", selectedIds.has(p.id));

  const meta = document.createElement("div");
  meta.className = "card__meta";
  const metaRow = document.createElement("div");
  metaRow.className = "card__meta-row";
  const nameEl = document.createElement("span");
  nameEl.className = "card__meta-name";
  nameEl.textContent = p.filename;
  nameEl.title = p.path;
  // Color-label dots (C-14), right of the filename like Apple Photos.
  const colorsEl = document.createElement("span");
  colorsEl.className = "card__colors";
  renderCardColors(colorsEl, p.colors);
  const editBtn = document.createElement("button");
  editBtn.className = "card__edit";
  editBtn.textContent = "✎";
  editBtn.title = t("card.edit.title");
  editBtn.addEventListener("click", async (ev) => {
    ev.stopPropagation();
    openEditDialog(p);
  });
  metaRow.appendChild(nameEl);
  metaRow.appendChild(colorsEl);
  metaRow.appendChild(editBtn);
  meta.appendChild(metaRow);
  if (p.tags && p.tags.length) {
    const tagsText = p.tags.join(", ");
    const descEl = document.createElement("div");
    descEl.className = "card__desc";
    descEl.textContent = tagsText;
    descEl.title = tagsText;
    meta.appendChild(descEl);
  }
  card.appendChild(thumb);
  card.appendChild(meta);
  p._card = card;
  // click: prioritize this card's thumbnail (queue head), then open preview.
  // setThumb must never prevent the preview from opening.
  card.style.cursor = "pointer";
  card.addEventListener("click", () => {
    // Select mode: clicking toggles selection instead of opening the preview.
    if (selectMode) {
      toggleSelect(p);
      return;
    }
    try {
      setThumb(img, p);
    } catch (e) {
      reportJs("click", String(e));
    }
    preview.open(p);
  });
  return card;
}

async function loadPhotos(folderId = null) {
  try {
    const photos = await invoke("get_photos", { folderId });
    allPhotos = photos;
    renderPhotos(applyFilters(photos));
  } catch (e) {
    console.error(e);
  }
}

// ---------------------------------------------------------------------------
// Image preview (modal, right panel) + custom context menu
// ---------------------------------------------------------------------------
function formatSize(bytes) {
  if (bytes >= 1024 * 1024) return (bytes / 1024 / 1024).toFixed(1) + " MB";
  if (bytes >= 1024) return (bytes / 1024).toFixed(0) + " KB";
  return bytes + " B";
}

/// Focal length display: whole mm stays an integer, otherwise 1 decimal.
function formatFocal(f) {
  return Number.isInteger(f) ? String(f) : f.toFixed(1);
}

const preview = {
  els: {
    overlay: document.getElementById("preview-overlay"),
    name: document.getElementById("preview-name"),
    meta: document.getElementById("preview-meta"),
    img: document.getElementById("preview-img"),
    error: document.getElementById("preview-error"),
    close: document.getElementById("preview-close"),
  },
  async open(photo) {
    this.els.name.textContent = photo.filename;
    this.els.name.title = photo.path;
    // Meta lines: name·size, then EXIF lens/focal when present (C-15).
    this.els.meta.textContent = "";
    const l1 = document.createElement("div");
    l1.textContent = `${photo.filename} · ${formatSize(photo.size)}`;
    this.els.meta.appendChild(l1);
    if (photo.lens && photo.lens !== "----") {
      const d = document.createElement("div");
      d.textContent = t("preview.lens", { lens: photo.lens });
      this.els.meta.appendChild(d);
    }
    if (photo.focal_length != null) {
      const d = document.createElement("div");
      d.textContent = t("preview.focal", { focal: formatFocal(photo.focal_length) });
      this.els.meta.appendChild(d);
    }
    this.els.error.hidden = true;
    this.els.error.textContent = t("preview.error");
    this.els.img.hidden = false;
    this._thumbOk = false;
    this.els.img.onerror = () => {
      // only surface an error when no usable thumbnail is on screen
      if (!this._thumbOk) {
        this.els.img.hidden = true;
        this.els.error.hidden = false;
      }
    };
    this.els.overlay.hidden = false;

    // 1) thumbnail first: cached, or a quick backend hit — instant feedback.
    const thumbSrc = thumbSrcCache.get(photo.path);
    if (thumbSrc) {
      this.els.img.src = thumbSrc;
      this._thumbOk = true;
    } else {
      try {
        const tp = await invoke("get_thumbnail", { path: photo.path });
        if (tp) {
          const src = convertFileSrc(tp);
          thumbSrcCache.set(photo.path, src);
          this.els.img.src = src;
          this._thumbOk = true;
        }
      } catch (e) {
        /* no thumbnail — full image below will decide */
      }
    }
    // 2) full image replaces the thumbnail once decoded (progressive preview).
    let fullSrc = null;
    try {
      fullSrc = convertFileSrc(photo.path);
    } catch (e) {
      /* keep placeholder */
    }
    if (fullSrc) {
      const fullImg = new Image();
      fullImg.onload = () => {
        if (!this.els.overlay.hidden) this.els.img.src = fullSrc;
      };
      fullImg.src = fullSrc;
    }
  },
  close() {
    this.els.overlay.hidden = true;
    this._thumbOk = false;
    this.els.img.onerror = null;
    this.els.img.src = "";
  },
};

preview.els.close.addEventListener("click", () => preview.close());
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && !preview.els.overlay.hidden) preview.close();
});

// --- custom context menu on photo cards (replaces the browser menu) ---
let contextMenu = null;
function hideContextMenu() {
  if (contextMenu) {
    contextMenu.remove();
    contextMenu = null;
  }
}

/// Small transient toast (bottom-center) for actions without a natural place
/// to report success (e.g. "wallpaper set").
function toast(message) {
  const el = document.createElement("div");
  el.className = "toast";
  el.textContent = message;
  document.body.appendChild(el);
  setTimeout(() => el.remove(), 2200);
}

function showContextMenu(x, y, photo) {
  hideContextMenu();
  const menu = document.createElement("div");
  menu.className = "ctx-menu";
  const reveal = document.createElement("button");
  reveal.className = "ctx-menu__item";
  reveal.textContent = t("menu.reveal");
  reveal.addEventListener("click", () => {
    hideContextMenu();
    invoke("reveal_in_folder", { path: photo.path }).catch((e) => alert(String(e)));
  });
  const wallpaper = document.createElement("button");
  wallpaper.className = "ctx-menu__item";
  wallpaper.textContent = t("menu.wallpaper");
  wallpaper.addEventListener("click", () => {
    hideContextMenu();
    invoke("set_wallpaper", { path: photo.path })
      .then(() => toast(t("menu.wallpaperSet")))
      .catch((e) => alert(String(e)));
  });
  menu.appendChild(reveal);
  menu.appendChild(wallpaper);
  document.body.appendChild(menu);
  const w = menu.offsetWidth;
  const h = menu.offsetHeight;
  menu.style.left = `${Math.max(4, Math.min(x, window.innerWidth - w - 8))}px`;
  menu.style.top = `${Math.max(4, Math.min(y, window.innerHeight - h - 8))}px`;
  contextMenu = menu;
}

window.addEventListener("contextmenu", (e) => {
  // Keep the native menu on text fields (copy/paste), replace it elsewhere.
  const tag = e.target.tagName;
  if (tag === "INPUT" || tag === "TEXTAREA" || e.target.isContentEditable) return;
  e.preventDefault();
  const card = e.target.closest(".card");
  if (card && card._photo) {
    showContextMenu(e.clientX, e.clientY, card._photo);
  } else {
    hideContextMenu();
  }
});
window.addEventListener("click", hideContextMenu);
window.addEventListener("scroll", hideContextMenu, true);
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") hideContextMenu();
});

// ---------------------------------------------------------------------------
// Folders view (cached; refreshed when dirty)
// ---------------------------------------------------------------------------
let folderCache = null;
let folderDirty = true;

function markFoldersDirty() {
  folderDirty = true;
}

async function loadFolders() {
  if (folderCache && !folderDirty) {
    renderFolders(folderCache);
    return;
  }
  try {
    const folders = await invoke("get_folders");
    folderCache = folders;
    folderDirty = false;
    renderFolders(folders);
  } catch (e) {
    console.error(e);
  }
}

function renderFolders(folders) {
  els.folderList.innerHTML = "";
  if (!folders.length) {
    els.folderList.innerHTML = `<div class="empty">${t("folders.empty")}</div>`;
  } else {
    for (const f of folders) {
      const row = document.createElement("div");
      row.className = "folder-item";
      const left = document.createElement("div");
      left.style.flex = "1";
      left.style.minWidth = "0";
      const pathEl = document.createElement("div");
      pathEl.className = "folder-item__path";
      pathEl.textContent = f.path;
      pathEl.title = f.path;
      const countEl = document.createElement("div");
      countEl.className = "folder-item__count";
      countEl.textContent = t("folders.count", { count: f.photo_count });
      left.appendChild(pathEl);
      left.appendChild(countEl);
      const btn = document.createElement("button");
      btn.className = "folder-item__remove";
      btn.textContent = t("folders.remove");
      btn.addEventListener("click", async () => {
        await invoke("remove_folder", { id: f.id });
        markFoldersDirty();
        loadFolders();
        loadPhotos();
      });
      row.appendChild(left);
      row.appendChild(btn);
      // click to filter
      row.style.cursor = "pointer";
      row.addEventListener("click", (ev) => {
        if (ev.target === btn) return;
        switchView("photos");
        loadPhotos(f.id);
      });
      els.folderList.appendChild(row);
    }
  }
  els.folderStatus.textContent = t("folders.status.count", { count: folders.length });
}

// ---------------------------------------------------------------------------
// Search (name + semantic/tag) with 500ms debounce per LIMITS.md:145.
// The right box searches via the mode dropdown (semantic | tag); the left
// box is the filename search. Right box takes priority when filled.
// ---------------------------------------------------------------------------
let searchTimer = null;
function scheduleSearch() {
  clearTimeout(searchTimer);
  searchTimer = setTimeout(runSearch, 500);
}
els.searchInput.addEventListener("input", scheduleSearch);
els.semanticSearchInput.addEventListener("input", scheduleSearch);
els.searchMode.addEventListener("change", scheduleSearch);

async function runSearch() {
  const q2 = els.semanticSearchInput.value.trim();
  const qName = els.searchInput.value.trim();
  const mode = els.searchMode.value;
  if (q2) {
    try {
      const res = await invoke("search", { query: q2, mode });
      allPhotos = res;
      renderPhotos(applyFilters(res));
    } catch (e) {
      console.error(e);
      renderPhotos([]);
      const msg = String(e);
      if (mode === "semantic") {
        els.photoStatus.textContent = msg.includes("not ready")
          ? t("search.semantic.unavailable")
          : t("search.semantic.error");
      } else {
        els.photoStatus.textContent = t("search.tag.error");
      }
    }
    return;
  }
  if (!qName) {
    loadPhotos();
    return;
  }
  try {
    const nameRes = await invoke("search_files", { query: qName });
    allPhotos = nameRes || [];
    renderPhotos(applyFilters(allPhotos));
  } catch (e) {
    console.error(e);
  }
}

// ---------------------------------------------------------------------------
// Toolbar / events
// ---------------------------------------------------------------------------
els.btnAdd.addEventListener("click", async () => {
  try {
    const selected = await openDialog({ directory: true, multiple: false });
    if (!selected) return;
    const path = Array.isArray(selected) ? selected[0] : selected;
    // add_folder now returns after the scan completes, so counts and the
    // photo list are final — no setTimeout polling needed.
    await invoke("add_folder", { path });
    markFoldersDirty();
    await loadFolders();
    await loadPhotos();
  } catch (e) {
    console.error(e);
    alert(String(e));
  }
});

els.btnRefresh.addEventListener("click", async () => {
  els.btnRefresh.disabled = true;
  try {
    await invoke("scan_folders");
    markFoldersDirty();
    await loadFolders();
    await loadPhotos();
  } catch (e) {
    console.error(e);
  } finally {
    els.btnRefresh.disabled = false;
  }
});

// Tauri events: refresh on scan
listen("scan-complete", () => {
  markFoldersDirty();
  loadPhotos();
  loadFolders();
});

// Language switch: re-render current view with new locale
onLanguageChange(() => {
  applyStaticI18n();
  if (!els.viewPhotos.classList.contains("view--hidden")) {
    renderPhotos(currentPhotos);
  } else if (!els.viewFolders.classList.contains("view--hidden")) {
    if (folderCache) renderFolders(folderCache);
  } else if (!els.viewTags.classList.contains("view--hidden")) {
    renderTags();
  } else {
    renderSettings();
  }
});

// Initial load
(async () => {
  try {
    await initI18n();
  } catch (e) {
    console.error(e);
  }
  applyStaticI18n();
  // Debug flag gates AI-confidence badges — read it before first render.
  try {
    debugMode = (await invoke("get_setting", { key: "debug" })) === "1";
  } catch (e) {
    debugMode = false;
  }
  loadPhotos();
  loadFolders();
  detectAndReportRenderer();
})();
