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
  navRejects: document.getElementById("nav-rejects"),
  navSettings: document.getElementById("nav-settings"),
  viewPhotos: document.getElementById("view-photos"),
  viewFolders: document.getElementById("view-folders"),
  viewTags: document.getElementById("view-tags"),
  viewRejects: document.getElementById("view-rejects"),
  viewSettings: document.getElementById("view-settings"),
  langOptions: document.getElementById("lang-options"),
  themeOptions: document.getElementById("theme-options"),
  toggleFxAnim: document.getElementById("toggle-fx-anim"),
  toggleFxShadow: document.getElementById("toggle-fx-shadow"),
  fxAnimState: document.getElementById("fx-anim-state"),
  fxShadowState: document.getElementById("fx-shadow-state"),
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
  searchModeBtns: document.querySelectorAll("#search-mode .searchbar__mode-btn"),
  semanticSearchInput: document.getElementById("semantic-search-input"),
  btnSelectMode: document.getElementById("btn-select-mode"),
  selectionBar: document.getElementById("selection-bar"),
  selectionBarSecondary: document.getElementById("selection-bar-secondary"),
  selectionCount: document.getElementById("selection-count"),
  btnSelectionTag: document.getElementById("btn-selection-tag"),
  btnSelectionRate: document.getElementById("btn-selection-rate"),
  btnSelectionExport: document.getElementById("btn-selection-export"),
  btnSelectionDelete: document.getElementById("btn-selection-delete"),
  btnSelectionClearTags: document.getElementById("btn-selection-clear-tags"),
  btnSelectionCancel: document.getElementById("btn-selection-cancel"),
  tagpickOverlay: document.getElementById("tagpick-overlay"),
  tagpickList: document.getElementById("tagpick-list"),
  tagpickCancel: document.getElementById("tagpick-cancel"),
  editChips: document.getElementById("edit-chips"),
  editSuggest: document.getElementById("edit-suggest"),
  photoGrid: document.getElementById("photo-grid"),
  photoStatus: document.getElementById("photo-status"),
  rejectGrid: document.getElementById("reject-grid"),
  rejectStatus: document.getElementById("reject-status"),
  rejectSearchInput: document.getElementById("reject-search-input"),
  ratingFilterRejects: document.getElementById("rating-filter-rejects"),
  btnSelectModeRejects: document.getElementById("btn-select-mode-rejects"),
  btnColorFilterRejects: document.getElementById("btn-color-filter-rejects"),
  btnRejectCond: document.getElementById("btn-reject-cond"),
  rejectCondPanel: document.getElementById("reject-cond-panel"),
  rejectCondItems: document.getElementById("reject-cond-items"),
  btnRejectCondClear: document.getElementById("btn-reject-cond-clear"),
  rateOverlay: document.getElementById("rate-overlay"),
  ratePicker: document.getElementById("rate-picker"),
  rateCancel: document.getElementById("rate-cancel"),
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
// The photo grid being rendered right now (photos view or rejects view,
// C-19) — every grid operation below targets this element.
let currentGrid = els.photoGrid;
const rejectGrid = els.rejectGrid;

// --- collapsible sidebar (C-19.2): labels + width animate; expanded by
// default, state persisted in localStorage ---
const sidebarEl = document.getElementById("sidebar");
const sidebarToggle = document.getElementById("sidebar-toggle");
const SIDEBAR_KEY = "tiol-sidebar";

function applySidebar(open) {
  sidebarEl.classList.toggle("sidebar--open", open);
  sidebarToggle.textContent = open ? "◀" : "▶";
  sidebarToggle.title = t(open ? "sidebar.collapse" : "sidebar.expand");
  try {
    localStorage.setItem(SIDEBAR_KEY, open ? "1" : "0");
  } catch (e) {
    /* non-fatal */
  }
}
sidebarToggle.addEventListener("click", () => {
  applySidebar(!sidebarEl.classList.contains("sidebar--open"));
});
{
  let saved = "1"; // expanded by default
  try {
    saved = localStorage.getItem(SIDEBAR_KEY) || "1";
  } catch (e) {
    /* non-fatal */
  }
  applySidebar(saved !== "0");
}

function switchView(name) {
  // Capture pre-switch visibility — the hidden classes flip below, and the
  // "leaving a grid" check must reason about where we came FROM (C-19.11).
  const prevPhotosVisible = !els.viewPhotos.classList.contains("view--hidden");
  const prevRejectsVisible = !els.viewRejects.classList.contains("view--hidden");
  const isPhotos = name === "photos";
  const isFolders = name === "folders";
  const isTags = name === "tags";
  const isRejects = name === "rejects";
  els.viewPhotos.classList.toggle("view--hidden", !isPhotos);
  els.viewFolders.classList.toggle("view--hidden", !isFolders);
  els.viewTags.classList.toggle("view--hidden", !isTags);
  els.viewRejects.classList.toggle("view--hidden", !isRejects);
  els.viewSettings.classList.toggle("view--hidden", name !== "settings");
  els.navPhotos.classList.toggle("sidebar__btn--active", isPhotos);
  els.navFolders.classList.toggle("sidebar__btn--active", isFolders);
  els.navTags.classList.toggle("sidebar__btn--active", isTags);
  els.navRejects.classList.toggle("sidebar__btn--active", isRejects);
  els.navSettings.classList.toggle("sidebar__btn--active", name === "settings");
  // Photo grids: switch the target of all grid operations (C-19).
  if (isPhotos) {
    currentGrid = els.photoGrid;
    els.photoStatus.classList.remove("view--hidden");
  } else if (isRejects) {
    currentGrid = rejectGrid;
  }
  // Shared filters (colors/lens/focal/rating) are page-specific: switching
  // between Photos and Rejects clears them so one page's conditions never
  // leak into the other (C-19.9).
  if ((isPhotos && lastGridView === "rejects") || (isRejects && lastGridView === "photos")) {
    clearSharedFilters();
  }
  if (isPhotos || isRejects) lastGridView = isPhotos ? "photos" : "rejects";
  // Leaving a photo grid exits multi-select mode (C-19.11): the mode is
  // page-bound — photos and rejects each have their own select button, and
  // carrying the mode across pages made the first click on the other page
  // act as "cancel" instead of entering select mode.
  const leavingGrid =
    (prevPhotosVisible && !isPhotos) || (prevRejectsVisible && !isRejects);
  if (leavingGrid && selectMode) setSelectMode(false);
  // Defer to next frame so the unhidden view has settled before measuring.
  if (isPhotos || isRejects) requestAnimationFrame(fillGridIfNeeded);
  requestAnimationFrame(updateSidebarIndicator);
}

/// Slide the active-indicator bar to the currently active nav button (C-19.10).
function updateSidebarIndicator() {
  const ind = document.getElementById("sidebar-indicator");
  const active = document.querySelector(".sidebar__btn--active");
  if (!ind || !active) return;
  ind.style.transform = `translateY(${active.offsetTop + 6}px)`;
}
let lastGridView = "photos";

/// Reset the SHARED filter state (colors / lens / focal / rating) — used
/// when switching between the Photos and Rejects pages (C-19.9).
function clearSharedFilters() {
  activeColorFilters.clear();
  activeLensFilters.clear();
  focalMin = null;
  focalMax = null;
  minRating = 0;
  filterFocalMin.value = "";
  filterFocalMax.value = "";
  activeRatings.clear();
  for (let i = 0; i <= 5; i++) activeRatings.add(i);
  if (ratingFilterEl) {
    ratingFilterEl.querySelectorAll('input[type="checkbox"]').forEach((cb) => { cb.checked = true; });
  }
  if (els.ratingFilterRejects) renderRejectRatings();
  renderFilterDots();
  updateFilterButton();
}
// --- Theme (dark / light / liquid-glass) — persisted in localStorage, no backend needed ---
const THEME_KEY = "tiol-theme";
const MENU_BG_KEY = "menu_bg_color";

function applyTheme(theme) {
  document.documentElement.setAttribute("data-theme", theme);
  if (theme === "liquid-glass") {
    document.body.classList.add("theme-liquid-glass");
  } else {
    document.body.classList.remove("theme-liquid-glass");
  }
}
function initTheme() {
  let saved = "dark";
  try { saved = localStorage.getItem(THEME_KEY) || "dark"; } catch (e) {}
  applyTheme(saved);
}

async function applyMenuBgColor(color) {
  if (!color) return;
  document.documentElement.style.setProperty("--menu-bg-custom", color);
  try { await invoke("set_setting", { key: MENU_BG_KEY, value: color }); } catch (e) {}
}

async function initMenuBgColor() {
  let saved = "#ffffff";
  try { saved = (await invoke("get_setting", { key: MENU_BG_KEY })) || "#ffffff"; } catch (e) {}
  if (els.menuBgColor) els.menuBgColor.value = saved;
  applyMenuBgColor(saved);
}

function renderThemeButtons() {
  const cur = document.documentElement.getAttribute("data-theme") || "dark";
  els.themeOptions.querySelectorAll("[data-theme]").forEach((btn) => {
    btn.classList.toggle("btn--active", btn.dataset.theme === cur);
  });
}

els.themeOptions.addEventListener("click", (ev) => {
  const btn = ev.target.closest("[data-theme]");
  if (!btn) return;
  const theme = btn.dataset.theme;
  applyTheme(theme);
  try { localStorage.setItem(THEME_KEY, theme); } catch (e) {}
  renderThemeButtons();
});

// --- FX toggles (C-19.11): animations / shadows — localStorage, default ON ---
const FX_ANIM_KEY = "tiol-fx-anim";
const FX_SHADOW_KEY = "tiol-fx-shadow";

function applyFx() {
  let anim = "1";
  let shadow = "1";
  try {
    anim = localStorage.getItem(FX_ANIM_KEY) || "1";
    shadow = localStorage.getItem(FX_SHADOW_KEY) || "1";
  } catch (e) {}
  document.body.classList.toggle("fx-anim-off", anim !== "1");
  document.body.classList.toggle("fx-shadow-off", shadow !== "1");
  if (els.fxAnimState) els.fxAnimState.textContent = t(anim === "1" ? "settings.on" : "settings.off");
  if (els.fxShadowState) els.fxShadowState.textContent = t(shadow === "1" ? "settings.on" : "settings.off");
  els.toggleFxAnim.classList.toggle("btn--active", anim === "1");
  els.toggleFxShadow.classList.toggle("btn--active", shadow === "1");
}

els.toggleFxAnim.addEventListener("click", () => {
  const next = ((localStorage.getItem(FX_ANIM_KEY) || "1") === "1") ? "0" : "1";
  try { localStorage.setItem(FX_ANIM_KEY, next); } catch (e) {}
  applyFx();
});
els.toggleFxShadow.addEventListener("click", () => {
  const next = ((localStorage.getItem(FX_SHADOW_KEY) || "1") === "1") ? "0" : "1";
  try { localStorage.setItem(FX_SHADOW_KEY, next); } catch (e) {}
  applyFx();
});

els.menuBgColor?.addEventListener("input", (ev) => {
  applyMenuBgColor(ev.target.value);
});
els.navPhotos.addEventListener("click", () => {
  switchView("photos");
  onboardingOnPhotosClicked();
  // Re-fetch so cards show freshly computed tags (stale-tag fix).
  if (!els.searchInput.value.trim() && !els.semanticSearchInput.value.trim()) {
    loadPhotos();
  }
});
els.navFolders.addEventListener("click", () => { switchView("folders"); loadFolders(); });
els.navTags.addEventListener("click", () => { switchView("tags"); renderTags(); });
els.navRejects.addEventListener("click", () => {
  switchView("rejects");
  loadRejects();
  // Re-render the condition labels in the CURRENT language — the initial
  // render runs before initI18n resolves (default en-US), so entering the
  // page must refresh them (C-19.1).
  renderRejectConds();
  // Kick off the one-time metrics analysis (eyes/exposure; instant when
  // everything is already cached in the DB).
  ensureRejectAnalysis();
});
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
  renderThemeButtons();
  applyFx();
  await initMenuBgColor();
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
  // The confirm dialog must not overlap the selection bar either (C-19.9).
  setSelectionBarVisible(false);
}
function closeConfirmDialog() {
  confirmCallback = null;
  els.confirmOverlay.hidden = true;
  setSelectionBarVisible(true);
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
  const g = currentGrid;
  const gap = 12;
  const cardW = 180;
  const contentW = (g.clientWidth || 1200) - 32; // 16px padding each side
  return Math.max(1, Math.floor((contentW + gap) / (cardW + gap)));
}

/// Render a photo list into the current grid. By default the grid's scroll
/// position is PRESERVED (forced refreshes after tagging/rating/filtering
/// must not yank the user back to the top, C-19.10); pass
/// `{ scrollTop: 0 }` for fresh result sets (searches, folder switches).
function renderPhotos(photos, opts = {}) {
  const scrollTop = opts.scrollTop !== undefined ? opts.scrollTop : currentGrid.scrollTop;
  currentPhotos = photos;
  renderedCount = 0;
  currentGrid.innerHTML = "";
  thumbObserver.disconnect();
  // Queued entries reference cards from the previous render — drop them.
  thumbQueue.length = 0;
  if (!photos.length) {
    const rejectsActive =
      !els.viewRejects.classList.contains("view--hidden") &&
      activeRejectConds.size > 0;
    currentGrid.innerHTML = `<div class="empty">${t(hasActiveFilters() || rejectsActive ? "photos.filterEmpty" : "photos.empty")}</div>`;
    els.photoStatus.textContent = t("photos.status.count", { count: 0 });
    pagePhotoSub.textContent = t("photos.status.count", { count: 0 });
    return;
  }
  // Initial render: exactly the top 5 rows (in order). Further rows are
  // rendered on scroll / viewport fill.
  renderChunk(cardsPerRow() * 5);
  // Restore the scroll position AFTER content exists — assigning scrollTop to
  // an empty grid is clamped to 0, which silently dropped the position and
  // made every forced re-render jump back to the top (C-19.10).
  currentGrid.scrollTop = scrollTop;
  if (scrollTop > 0) {
    // The initial chunk may be shorter than the target — keep filling
    // (bounded) and re-applying until the position actually sticks.
    let guard = 0;
    while (
      currentGrid.scrollTop < scrollTop &&
      renderedCount < currentPhotos.length &&
      guard++ < 20
    ) {
      scrollToViewport();
      currentGrid.scrollTop = scrollTop;
    }
  }
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
  const g = currentGrid;
  const restoreWindow =
    scrollTop > 0 ? scrollTop - g.clientHeight : 0; // enqueue near the restored viewport
  for (let i = renderedCount - 1; i >= 0; i--) {
    const card = currentGrid.children[i];
    const img = card && card._img;
    if (!img || !card._photo) continue;
    // Deep restore: only the cards around the restored position matter —
    // enqueueing every rendered card would starve the visible ones (C-19.10).
    if (restoreWindow > 0) {
      const ct = card.offsetTop;
      if (ct + card.offsetHeight < restoreWindow || ct > scrollTop + g.clientHeight * 2) continue;
    }
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
  const startIdx = renderedCount;
  for (let i = startIdx; i < end; i++) {
    const card = buildCard(currentPhotos[i]);
    // Entry-animation stagger index within this chunk (C-19.12); capped in CSS.
    card.style.setProperty("--i", String(i - startIdx));
    currentGrid.appendChild(card);
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
  // Both grid pages (photos/rejects) have their own select button — keep
  // their labels and active state in sync (C-19.11).
  const label = t(on ? "photos.selectDone" : "photos.selectMode");
  els.btnSelectMode.textContent = label;
  els.btnSelectMode.classList.toggle("searchbar__select--active", on);
  els.btnSelectModeRejects.textContent = label;
  els.btnSelectModeRejects.classList.toggle("searchbar__select--active", on);
  els.selectionBar.hidden = !on;
  els.selectionBarSecondary.hidden = !on;
  if (!on) {
    selectedIds.clear();
  }
  // Update already-rendered cards in place (no re-render: keeps scroll pos).
  // BOTH grids: switchView may have already swapped currentGrid when this
  // runs on a view change, and the other grid must not keep its "selecting"
  // class or stale checkboxes (C-19.11).
  for (const grid of [els.photoGrid, rejectGrid]) {
    grid.classList.toggle("selecting", on);
    for (const card of grid.children) {
      if (!card._photo) continue;
      card.classList.toggle("card--selected", selectedIds.has(card._photo.id));
      const cb = card.querySelector(".card__check");
      if (cb) cb.hidden = !on;
      // Reject badge stays visible in select mode — it shifts down via CSS.
      const rj = card.querySelector(".card__reject");
      if (rj) rj.hidden = !(card._photo.colors || []).includes("reject");
    }
  }
  updateSelectionBar();
}

/// Re-apply multi-select state to a freshly rendered grid (C-19.11):
/// renderPhotos rebuilds cards, so the selected class must be re-applied
/// after a delete that keeps select mode on.
function applySelectionToGrid(grid) {
  for (const card of grid.children) {
    if (!card._photo) continue;
    card.classList.toggle("card--selected", selectedIds.has(card._photo.id));
  }
}

function updateSelectionBar() {
  if (!selectMode) return;
  const n = selectedIds.size;
  els.selectionCount.textContent = t("photos.selectedCount", { count: n });
  els.btnSelectionTag.disabled = n === 0;
  els.btnSelectionClearTags.disabled = n === 0;
  els.btnSelectionRate.disabled = n === 0;
  els.btnSelectionExport.disabled = n === 0;
  els.btnSelectionDelete.disabled = n === 0;
  // "Delete files" is a REJECTS-page action only (C-19.10).
  els.btnSelectionDelete.hidden = els.viewRejects.classList.contains("view--hidden");
}

/// Dialogs (add-tag / rate / confirm) must never overlap the floating
/// selection bar — hide it while any of them is open (C-19.9).
function setSelectionBarVisible(visible) {
  els.selectionBar.hidden = !(visible && selectMode);
  els.selectionBarSecondary.hidden = !(visible && selectMode);
}

function toggleSelect(photo) {
  if (selectedIds.has(photo.id)) selectedIds.delete(photo.id);
  else selectedIds.add(photo.id);
  if (photo._card) {
    photo._card.classList.toggle("card--selected", selectedIds.has(photo.id));
  }
  updateSelectionBar();
}

// Selection-action feedback (C-19.6/C-19.10): shown as a popup ABOVE the
// bottom selection bar — positioned dynamically by measuring the bar, so it
// can never overlap regardless of the bar's current height.
let selectionHintTimer = null;
function showSelectionHint(text) {
  const el = document.createElement("div");
  el.className = "toast selection-toast";
  el.textContent = text;
  document.body.appendChild(el);
  try {
    const bar = els.selectionBar;
    if (bar && !bar.hidden) {
      const r = bar.getBoundingClientRect();
      el.style.bottom = `${Math.max(8, window.innerHeight - r.top + 10)}px`;
    }
  } catch (e) {
    /* fall back to the CSS bottom */
  }
  clearTimeout(selectionHintTimer);
  selectionHintTimer = setTimeout(() => el.remove(), 2500);
}

els.btnSelectMode.addEventListener("click", () => setSelectMode(!selectMode));
els.btnSelectionCancel.addEventListener("click", () => setSelectMode(false));

// "Export" (C-19.10): copy the selected photos into a chosen folder — works
// from BOTH photo grids.
els.btnSelectionExport.addEventListener("click", async () => {
  if (!selectedIds.size) return;
  let dest = null;
  try {
    dest = await openDialog({ directory: true, multiple: false });
  } catch (e) {
    alert(String(e));
    return;
  }
  if (!dest) return;
  const path = Array.isArray(dest) ? dest[0] : dest;
  const ids = [...selectedIds];
  try {
    const n = await invoke("export_files", { fileIds: ids, destDir: path });
    showSelectionHint(t("photos.exported", { count: n }));
  } catch (e) {
    alert(String(e));
  }
});

// "Delete" (C-19.10, rejects page): permanently remove the selected photos
// from disk AND the library — confirmed first.
els.btnSelectionDelete.addEventListener("click", () => {
  const n = selectedIds.size;
  confirmDialog(t("photos.deleteConfirm", { count: n }), async () => {
    const ids = [...selectedIds];
    try {
      await invoke("delete_files", { fileIds: ids });
      // STAY in select mode so more files can be deleted in one pass
      // (C-19.11); drop the deleted ids and re-apply the selection after
      // the fresh render. Serialized: loadPhotos must not race loadRejects
      // or it would cross-paint the full list into the rejects grid.
      for (const id of ids) selectedIds.delete(id);
      showSelectionHint(t("photos.deleted", { count: ids.length }));
      await loadRejects();
      await loadPhotos();
      applySelectionToGrid(rejectGrid);
      updateSelectionBar();
    } catch (e) {
      alert(String(e));
    }
  });
});

// "Delete tags" (red): strip ALL tags (text + colors) from the selection.
els.btnSelectionClearTags.addEventListener("click", () => {
  const n = selectedIds.size;
  confirmDialog(t("photos.deleteTagsConfirm", { count: n }), async () => {
    const ids = [...selectedIds];
    try {
      await invoke("clear_tags_from_files", { fileIds: ids });
      showSelectionHint(t("photos.tagsDeleted", { count: ids.length }));
      // Update the data, then re-render — the in-place path missed the
      // top-right reject badge (C-19.10).
      for (const p of currentPhotos) {
        if (!selectedIds.has(p.id)) continue;
        p.tags = [];
        p.colors = [];
      }
      if (els.viewRejects.classList.contains("view--hidden")) {
        renderPhotos(applyFilters(allPhotos));
      } else {
        renderPhotos(applyRejectConds(applyFilters(allRejects)));
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
// Bound to BOTH grids (photos view + rejects view, C-19).
let dragSel = null;
function onGridMouseDown(e) {
  if (!selectMode || e.button !== 0) return;
  if (e.target.closest("button, input, select, .card__edit, .card__stars")) return;
  const grid = e.currentTarget;
  const gridRect = grid.getBoundingClientRect();
  // Don't hijack the vertical scrollbar.
  if (e.clientX > gridRect.left + gridRect.width - 16) return;
  e.preventDefault(); // no text selection / native image drag
  const box = document.createElement("div");
  box.className = "selection-box";
  document.body.appendChild(box); // fixed positioning: viewport coords
  const startX = e.clientX;
  const startY = e.clientY;
  dragSel = { startX, startY, box, gridRect, moved: false, last: null };
  const onMove = (ev) => {
    if (!dragSel) return;
    const x = ev.clientX;
    const y = ev.clientY;
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
    // Live highlight of intersecting cards (viewport coords — the box is
    // fixed-positioned now, C-19.10).
    for (const card of grid.children) {
      if (!card._photo) continue;
      const r = card.getBoundingClientRect();
      const hit = r.left < R && r.right > L && r.top < B && r.bottom > T;
      card.classList.toggle("card--sel-hover", hit);
    }
  };
  const onUp = () => {
    window.removeEventListener("mousemove", onMove);
    window.removeEventListener("mouseup", onUp);
    const d = dragSel;
    dragSel = null;
    box.remove();
    for (const card of grid.children) {
      card.classList.remove("card--sel-hover");
    }
    if (!d || !d.moved) return; // plain click → card click toggles it
    const { L, T, R, B } = d.last;
    for (const card of grid.children) {
      if (!card._photo) continue;
      const r = card.getBoundingClientRect();
      const hit = r.left < R && r.right > L && r.top < B && r.bottom > T;
      if (hit && !selectedIds.has(card._photo.id)) {
        selectedIds.add(card._photo.id);
        card.classList.add("card--selected");
      }
    }
    updateSelectionBar();
  };
  window.addEventListener("mousemove", onMove);
  window.addEventListener("mouseup", onUp);
}
els.photoGrid.addEventListener("mousedown", onGridMouseDown);
rejectGrid.addEventListener("mousedown", onGridMouseDown);

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
      setSelectionBarVisible(true);
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
  setSelectionBarVisible(false);
  tagpickSearch.value = "";
  renderTagPickList();
  els.tagpickOverlay.hidden = false;
});
els.tagpickCancel.addEventListener("click", () => {
  els.tagpickOverlay.hidden = true;
  setSelectionBarVisible(true);
});
els.tagpickOverlay.addEventListener("click", (e) => {
  if (e.target === els.tagpickOverlay) {
    els.tagpickOverlay.hidden = true;
    setSelectionBarVisible(true);
  }
});

// ---------------------------------------------------------------------------
// Color labels (C-14): applied from the selection-bar dots, shown as dots on
// the cards (right of the filename), filterable from the search bar (union).
// ---------------------------------------------------------------------------
const COLOR_ORDER = ["red", "orange", "yellow", "green", "blue", "purple", "reject"];
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

// Star rating filter (C-17): show only photos whose rating is in the active
// set. Each checkbox toggles one rating (0–5). Unchecked ratings are hidden.
// Default: all ratings selected.
const activeRatings = new Set([0, 1, 2, 3, 4, 5]);
const ratingFilterEl = document.getElementById("rating-filter");
ratingFilterEl.addEventListener("change", () => {
  activeRatings.clear();
  ratingFilterEl.querySelectorAll('input[type="checkbox"]').forEach((cb) => {
    if (cb.checked) activeRatings.add(parseInt(cb.value, 10));
  });
  renderPhotos(applyFilters(allPhotos));
});

function hasActiveFilters() {
  return (
    activeColorFilters.size > 0 ||
    activeLensFilters.size > 0 ||
    focalMin != null ||
    focalMax != null ||
    activeRatings.size < 6
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
    // Star rating (C-17): photo must have a rating currently checked.
    if (activeRatings.size < 6 && !activeRatings.has(p.rating || 0)) return false;
    return true;
  });
}

function updateFilterButton() {
  btnColorFilter.classList.toggle("searchbar__filter--active", hasActiveFilters());
}

// Selection-bar dots: one per color; clicking applies/toggles it on ALL
// selected photos (phone-gallery semantics, handled by toggle_color_tag).
// The "reject" marker is drawn as a circle with an X (C-19.10).
function styleColorDot(dot, c) {
  if (c === "reject") {
    dot.classList.add("color-dot--reject");
  } else {
    dot.style.background = COLOR_HEX[c];
  }
}

const selectionColorDots = document.getElementById("selection-color-dots");
for (const c of COLOR_ORDER) {
  const dot = document.createElement("button");
  dot.className = "color-dot color-dot--sel";
  styleColorDot(dot, c);
  dot.title = t(`colors.${c}`);
  dot.addEventListener("click", async () => {
    if (!selectedIds.size) return;
    const ids = [...selectedIds];
    try {
      const all = await invoke("toggle_color_tag", { fileIds: ids, color: c });
      // Update the data, then force a re-render — in-place DOM updates proved
      // unreliable for badge visibility (C-19.10).
      for (const p of currentPhotos) {
        if (!selectedIds.has(p.id)) continue;
        const cs = p.colors || [];
        if (all && !cs.includes(c)) p.colors = [...cs, c];
        else if (!all && cs.includes(c)) p.colors = cs.filter((x) => x !== c);
      }
      if (els.viewRejects.classList.contains("view--hidden")) {
        renderPhotos(applyFilters(allPhotos));
      } else {
        renderPhotos(applyRejectConds(applyFilters(allRejects)));
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

// Anchor a dropdown panel under its button, clamped to the viewport so it
// never overflows on the right in fullscreen (C-19.10).
function positionPanel(panel, btn) {
  const r = btn.getBoundingClientRect();
  panel.hidden = false;
  const pw = panel.offsetWidth || 240;
  let left = Math.max(8, r.left);
  if (left + pw > window.innerWidth - 8) {
    left = Math.max(8, window.innerWidth - pw - 8);
  }
  panel.style.left = `${left}px`;
  panel.style.top = `${r.bottom + 6}px`;
}

function renderFilterDots() {
  colorFilterDots.textContent = "";
  for (const c of COLOR_ORDER) {
    const dot = document.createElement("button");
    dot.className =
      "color-dot color-dot--filter" +
      (activeColorFilters.has(c) ? " color-dot--on" : "");
    styleColorDot(dot, c);
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
    positionPanel(colorFilterPanel, btnColorFilter);
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
  activeRatings.clear();
  for (let i = 0; i <= 5; i++) activeRatings.add(i);
  ratingFilterEl.querySelectorAll('input[type="checkbox"]').forEach((cb) => {
    cb.checked = true;
  });
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

// Keep filling until the viewport is covered (only while a photo grid is
// visible — photos or rejects view). Bounded per frame: at most 3 chunks,
// continue on the next frame, so a large library can never block the UI.
function fillGridIfNeeded() {
  if (els.viewPhotos.classList.contains("view--hidden") && els.viewRejects.classList.contains("view--hidden")) return;
  const g = currentGrid;
  let passes = 0;
  while (renderedCount < currentPhotos.length && g.scrollHeight <= g.clientHeight + 300) {
    renderChunk();
    if (++passes >= 3) {
      requestAnimationFrame(fillGridIfNeeded);
      return;
    }
  }
  // Startup trap (C-19.11): the rAF from loadPhotos can fire BEFORE the
  // webview's first layout — clientHeight is 0, the loop exits immediately
  // and nothing ever re-triggers it, leaving only the initial rows rendered
  // (no scrollbar either, so scrolling can't recover either). Retry every
  // frame until the grid has a real height.
  if (renderedCount < currentPhotos.length && g.clientHeight === 0) {
    requestAnimationFrame(fillGridIfNeeded);
  }
}

// Window resize can also leave the viewport under-filled — same idempotent
// fill (C-19.11).
window.addEventListener("resize", () => requestAnimationFrame(fillGridIfNeeded));

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
  const g = currentGrid;
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

function onGridScroll() {
  const g = currentGrid;
  if (renderedCount >= currentPhotos.length) return;
  // Viewport bottom is beyond the rendered region -> render the viewed area.
  if (g.scrollTop + g.clientHeight * 2 + 600 > g.scrollHeight) {
    scheduleScrollFill();
  }
}
els.photoGrid.addEventListener("scroll", onGridScroll);
rejectGrid.addEventListener("scroll", onGridScroll);

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
  // root: null = viewport — shared by the photos grid AND the rejects grid
  // (C-19); the 300px margin still preloads just before cards scroll in.
  { root: null, rootMargin: "300px" }
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

/// Fill the first `rating` stars of a card's star row (0 = all empty, 5 =
/// all filled). Rerun after a rating change to refresh the card in place.
function renderCardStars(el, rating) {
  for (let i = 0; i < el.children.length; i++) {
    el.children[i].classList.toggle("card__star--on", i < rating);
  }
}

/// Set a photo's star rating (C-17): clicking star N rates it N; clicking
/// the current value again clears it. Persists via set_rating, refreshes
/// the card in place, then re-applies an active rating filter so a photo
/// that dropped below the threshold disappears right away.
async function setPhotoRating(p, n) {
  const next = p.rating === n ? 0 : n;
  try {
    const updated = await invoke("set_rating", { fileId: p.id, rating: next });
    p.rating = updated.rating || 0;
    if (p._card) {
      const stars = p._card.querySelector(".card__stars");
      if (stars) renderCardStars(stars, p.rating);
    }
    if (activeRatings.size < 6) renderPhotos(applyFilters(allPhotos));
  } catch (e) {
    alert(String(e));
  }
}

/// Re-render the meta row of one card (name + color dots + tag list) after a
/// tag/color edit.
function renderCardColors(el, colors) {
  el.textContent = "";
  for (const c of colors || []) {
    // The reject marker is NOT rendered here — it has its own top-right
    // badge on the thumbnail (C-19.10).
    if (c === "reject") continue;
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
  // Reject marker (C-19.10): circle-with-X pinned to the thumb's TOP-RIGHT.
  // In select mode it shifts DOWN (CSS .grid.selecting) to make room for the
  // checkbox — always visible, never hidden (C-19.10).
  const rejectX = document.createElement("span");
  rejectX.className = "card__reject";
  rejectX.title = t("colors.reject");
  rejectX.hidden = !(p.colors || []).includes("reject");
  thumb.appendChild(rejectX);
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
  // Star rating row (C-17): below the thumbnail — click star N to rate the
  // photo 1-5 (clicking the current value clears it); unrated stars show a
  // white outline, rated stars a yellow fill. Never opens the preview.
  const stars = document.createElement("div");
  stars.className = "card__stars";
  stars.title = t("card.rating.title");
  for (let n = 1; n <= 5; n++) {
    const s = document.createElement("span");
    s.className = "card__star";
    s.textContent = "★";
    s.dataset.n = String(n);
    s.title = t("card.rating.star", { n });
    s.addEventListener("click", (ev) => {
      ev.stopPropagation();
      setPhotoRating(p, n);
    });
    stars.appendChild(s);
  }
  // Hover preview: fill stars up to the one under the cursor.
  stars.addEventListener("mouseover", (ev) => {
    const s = ev.target.closest(".card__star");
    if (!s) return;
    const n = Number(s.dataset.n);
    for (let i = 0; i < 5; i++) {
      stars.children[i].classList.toggle("card__star--hover", i < n);
    }
  });
  stars.addEventListener("mouseleave", () => {
    for (let i = 0; i < 5; i++) {
      stars.children[i].classList.remove("card__star--hover");
    }
  });
  renderCardStars(stars, p.rating || 0);
  card.appendChild(stars);
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

async function loadPhotos(folderId = null, opts = {}) {
  try {
    const photos = await invoke("get_photos", { folderId });
    allPhotos = photos;
    // ALWAYS render into the photos grid: currentGrid may be the rejects
    // grid when a refresh is triggered from there (delete / scan), and
    // painting the full unfiltered list into it caused a visible flash of
    // every photo on the rejects page (C-19.11).
    const wasGrid = currentGrid;
    currentGrid = els.photoGrid;
    renderPhotos(applyFilters(photos), opts);
    currentGrid = wasGrid;
    // Fill the viewport beyond the initial chunk — startup renders only the
    // first screenful and nothing triggers the fill loop otherwise (C-19.7).
    requestAnimationFrame(fillGridIfNeeded);
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
        loadPhotos(f.id, { scrollTop: 0 });
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
function setSearchMode(mode) {
  els.searchModeBtns.forEach((btn) => {
    btn.classList.toggle("searchbar__mode-btn--active", btn.dataset.mode === mode);
  });
}

els.searchMode.addEventListener("click", (e) => {
  const btn = e.target.closest(".searchbar__mode-btn");
  if (!btn) return;
  setSearchMode(btn.dataset.mode);
  scheduleSearch();
});
// Default to semantic on startup.
setSearchMode("semantic");

async function runSearch() {
  const q2 = els.semanticSearchInput.value.trim();
  const qName = els.searchInput.value.trim();
  const mode = els.searchMode.querySelector(".searchbar__mode-btn--active")?.dataset.mode || "semantic";
  if (q2) {
    try {
      const res = await invoke("search", { query: q2, mode });
      allPhotos = res;
      renderPhotos(applyFilters(res), { scrollTop: 0 });
    } catch (e) {
      console.error(e);
      renderPhotos([], { scrollTop: 0 });
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
    renderPhotos(applyFilters(allPhotos), { scrollTop: 0 });
  } catch (e) {
    console.error(e);
  }
}

// ---------------------------------------------------------------------------
// Rejects view (C-19): a photos-like grid WITHOUT filename search and the
// semantic/tag mode — instead a "reject conditions" panel (blur / under /
// over / eyes-closed; UI-only for now) sits next to the filter button.
// ---------------------------------------------------------------------------
let allRejects = []; // unfiltered result of the current rejects query

function renderRejectStatus(n) {
  els.rejectStatus.textContent = t("photos.status.count", { count: n });
}

async function loadRejects() {
  try {
    const photos = await invoke("get_photos", { folderId: null });
    allRejects = photos;
    // Shared filters (colors/lens/focal/rating) ∩ reject conditions.
    const shown = applyRejectConds(applyFilters(photos));
    const wasGrid = currentGrid;
    currentGrid = rejectGrid;
    renderPhotos(shown);
    currentGrid = wasGrid;
    renderRejectStatus(shown.length);
    requestAnimationFrame(fillGridIfNeeded);
  } catch (e) {
    console.error(e);
  }
}

let rejectSearchTimer = null;
els.rejectSearchInput.addEventListener("input", () => {
  clearTimeout(rejectSearchTimer);
  rejectSearchTimer = setTimeout(runRejectSearch, 500);
});

async function runRejectSearch() {
  const q = els.rejectSearchInput.value.trim();
  if (!q) {
    loadRejects();
    return;
  }
  try {
    const res = await invoke("search", { query: q, mode: "semantic" });
    allRejects = res;
    const shown = applyRejectConds(applyFilters(res));
    const wasGrid = currentGrid;
    currentGrid = rejectGrid;
    renderPhotos(shown);
    currentGrid = wasGrid;
    renderRejectStatus(shown.length);
  } catch (e) {
    console.error(e);
    const wasGrid = currentGrid;
    currentGrid = rejectGrid;
    renderPhotos([]);
    currentGrid = wasGrid;
    els.rejectStatus.textContent = t("search.semantic.error");
  }
}

// Rejects rating filter uses the same activeRatings Set as the photos view.
// The checkbox panel lives inside #reject-cond-panel and mirrors #color-filter.

// Both "Filter" buttons (photos + rejects) open the same global panel,
// anchored under whichever button was clicked.
els.btnColorFilterRejects.addEventListener("click", async (e) => {
  e.stopPropagation();
  colorFilterPanel.hidden = !colorFilterPanel.hidden;
  if (!colorFilterPanel.hidden) {
    positionPanel(colorFilterPanel, els.btnColorFilterRejects);
    await renderFilterLens();
  }
});

// Reject conditions (C-19.3): default ALL checked (user request); blur is
// UI-only (not implemented), the other three filter the rejects grid. The
// analysis (eyes-closed semantics + exposure pixels) runs once per library
// and is cached in the DB (incremental for new files).
const REJECT_CONDS = ["blur", "under", "over", "eyes", "rejected"];
const activeRejectConds = new Set(["blur", "under", "over", "eyes", "rejected"]);

/// Filter by the checked reject conditions (UNION inside — any matched
/// condition shows the photo; blur is skipped until implemented). Photos
/// whose metric is still unknown (NULL) fail the condition (C-15 rule).
function applyRejectConds(photos) {
  if (!activeRejectConds.size) return photos;
  return photos.filter((p) => {
    for (const c of activeRejectConds) {
      if (c === "blur") continue;
      if (c === "over" && p.overexposed === 1) return true;
      if (c === "under" && p.underexposed === 1) return true;
      if (c === "eyes" && p.eyes_closed === 1) return true;
      if (c === "rejected" && (p.colors || []).includes("reject")) return true;
    }
    return false;
  });
}

// Run the (incremental) analysis — triggered once at startup AND once on
// the first entry to the rejects page, then only again after a rescan
// (new files). The backend dedupes concurrent runs and completes the whole
// library in one pass, so repeated page entries must NOT re-trigger it
// (C-19.6 — the "multiple analysis passes" complaint).
let rejectAnalysisTriggered = false;
async function ensureRejectAnalysis() {
  if (rejectAnalysisTriggered) return;
  rejectAnalysisTriggered = true;
  try {
    await invoke("compute_reject_metrics");
  } catch (e) {
    console.error(e);
  }
}
const rejectBadge = document.getElementById("reject-badge");
const rejectFill = document.getElementById("reject-fill");
const rejectCount = document.getElementById("reject-count");
listen("reject-analysis-progress", (ev) => {
  const d = ev.payload || {};
  if (d.total > 0) {
    els.rejectStatus.textContent = t("rejects.analyzing", {
      done: d.done,
      total: d.total,
    });
    rejectBadge.hidden = false;
    rejectCount.textContent = `${d.done}/${d.total}`;
    rejectFill.style.width = `${Math.round((d.done / d.total) * 100)}%`;
  }
});
listen("reject-analysis-complete", () => {
  rejectBadge.hidden = true;
  renderRejectStatus(0);
  loadRejects();
});

function readRejectRatings() {
  activeRatings.clear();
  els.ratingFilterRejects.querySelectorAll('input[type="checkbox"]').forEach((cb) => {
    if (cb.checked) activeRatings.add(parseInt(cb.value, 10));
  });
}

function renderRejectRatings() {
  if (!els.ratingFilterRejects) return;
  els.ratingFilterRejects.querySelectorAll('input[type="checkbox"]').forEach((cb) => {
    cb.checked = activeRatings.has(parseInt(cb.value, 10));
  });
}

els.ratingFilterRejects?.addEventListener("change", () => {
  readRejectRatings();
  if (els.viewRejects.classList.contains("view--hidden")) {
    renderPhotos(applyFilters(allPhotos));
  } else {
    renderPhotos(applyRejectConds(applyFilters(allRejects)));
  }
});

function renderRejectConds() {
  els.rejectCondItems.textContent = "";
  for (const c of REJECT_CONDS) {
    const label = document.createElement("label");
    label.className = "reject-cond__item";
    const cb = document.createElement("input");
    cb.type = "checkbox";
    cb.checked = activeRejectConds.has(c);
    cb.addEventListener("change", () => {
      if (cb.checked) activeRejectConds.add(c);
      else activeRejectConds.delete(c);
      renderRejectConds();
      els.btnRejectCond.classList.toggle(
        "searchbar__filter--active",
        activeRejectConds.size > 0
      );
      // Re-filter ONLY — toggling conditions must never re-trigger the
      // analysis (it runs once at startup / on entering the page).
      if (els.viewRejects.classList.contains("view--hidden")) {
        renderPhotos(applyFilters(allPhotos));
      } else {
        loadRejects();
      }
    });
    const span = document.createElement("span");
    span.textContent = t(`rejects.${c}`);
    label.appendChild(cb);
    label.appendChild(span);
    els.rejectCondItems.appendChild(label);
  }
}
renderRejectConds();

els.btnRejectCond.addEventListener("click", (e) => {
  e.stopPropagation();
  els.rejectCondPanel.hidden = !els.rejectCondPanel.hidden;
  if (!els.rejectCondPanel.hidden) {
    positionPanel(els.rejectCondPanel, els.btnRejectCond);
  }
});
els.btnRejectCondClear.addEventListener("click", () => {
  activeRejectConds.clear();
  renderRejectConds();
  activeRatings.clear();
  for (let i = 0; i <= 5; i++) activeRatings.add(i);
  renderRejectRatings();
  updateFilterButton();
  els.btnRejectCond.classList.remove("searchbar__filter--active");
});
document.addEventListener("click", (e) => {
  if (
    !els.rejectCondPanel.hidden &&
    !e.target.closest("#reject-cond-panel, #btn-reject-cond")
  ) {
    els.rejectCondPanel.hidden = true;
  }
});

// Multi-select "Rate" (C-19): pick 1-5 stars, apply to ALL selected photos.
els.btnSelectionRate.addEventListener("click", () => {
  if (!selectedIds.size) return;
  setSelectionBarVisible(false);
  renderRatePicker();
  els.rateOverlay.hidden = false;
});

function renderRatePicker() {
  els.ratePicker.textContent = "";
  for (let n = 1; n <= 5; n++) {
    const btn = document.createElement("button");
    // All stars start UNSELECTED (dark) — clicking star N applies N and
    // closes the dialog; there is no preselection state (C-19.6).
    btn.className = "rate-picker__star";
    btn.textContent = "★";
    btn.title = t("card.rating.star", { n });
    btn.dataset.rating = String(n);
    btn.addEventListener("click", async () => {
      const ids = [...selectedIds];
      els.rateOverlay.hidden = true;
      setSelectionBarVisible(true);
      try {
        await invoke("set_rating_files", { fileIds: ids, rating: n });
        showSelectionHint(t("photos.rated", { count: ids.length, rating: n }));
        // Force a re-render of the current grid: the in-place star refresh
        // proved unreliable, and the fresh cards read p.rating directly.
        for (const p of currentPhotos) {
          if (selectedIds.has(p.id)) p.rating = n;
        }
        if (els.viewRejects.classList.contains("view--hidden")) {
          renderPhotos(applyFilters(allPhotos));
        } else {
          renderPhotos(applyRejectConds(applyFilters(allRejects)));
        }
      } catch (e) {
        alert(String(e));
      }
    });
    els.ratePicker.appendChild(btn);
  }
}
els.rateCancel.addEventListener("click", () => {
  els.rateOverlay.hidden = true;
  setSelectionBarVisible(true);
});
els.rateOverlay.addEventListener("click", (e) => {
  if (e.target === els.rateOverlay) {
    els.rateOverlay.hidden = true;
    setSelectionBarVisible(true);
  }
});
els.btnSelectModeRejects.addEventListener("click", () => setSelectMode(!selectMode));

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
    onboardingAfterAdd();
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

// Tauri events: refresh on scan (a scan may also add NEW lens names from
// freshly added photos — drop the lens-list cache so the filter panel
// re-fetches it next time it opens; C-15.4).
listen("scan-complete", async () => {
  lensCache = null;
  // New/changed files may need reject metrics — allow one more analysis pass.
  rejectAnalysisTriggered = false;
  markFoldersDirty();
  // Serialized: concurrent loadPhotos/loadRejects would cross-paint into the
  // other grid (C-19.11).
  await loadPhotos();
  await loadRejects();
  loadFolders();
});

// Language switch: re-render current view with new locale
onLanguageChange(() => {
  applyStaticI18n();
  initTheme();
  requestAnimationFrame(updateSidebarIndicator);
  if (!els.viewPhotos.classList.contains("view--hidden")) {
    renderPhotos(currentPhotos);
  } else if (!els.viewFolders.classList.contains("view--hidden")) {
    if (folderCache) renderFolders(folderCache);
  } else if (!els.viewTags.classList.contains("view--hidden")) {
    renderTags();
  } else if (!els.viewRejects.classList.contains("view--hidden")) {
    renderPhotos(currentPhotos);
    renderRejectConds();
  } else {
    renderSettings();
  }
});

// ---------------------------------------------------------------------------
// Self-update detection (C-18): hash of the running exe vs
// tiol.netlify.app/version.json. Checked once ~2s after startup + the
// settings-page button. The banner appears only when a remote hash differs.
// ---------------------------------------------------------------------------
const updateBanner = document.getElementById("update-banner");
const updateText = document.getElementById("update-text");
const btnUpdateDownload = document.getElementById("btn-update-download");
const btnUpdateLater = document.getElementById("btn-update-later");
const btnCheckUpdate = document.getElementById("btn-check-update");
let updateUrl = null;

function showUpdateBanner(version, url) {
  updateUrl = url;
  updateText.textContent = t("update.available", { version });
  updateBanner.hidden = false;
}

btnUpdateDownload.addEventListener("click", () => {
  updateBanner.hidden = true;
  if (updateUrl) {
    try {
      window.__TAURI__.shell.open(updateUrl).catch((e) => alert(String(e)));
    } catch (e) {
      alert(String(e));
    }
  }
});
btnUpdateLater.addEventListener("click", () => {
  updateBanner.hidden = true;
});

async function checkForUpdates(manual) {
  try {
    const info = await invoke("check_update");
    if (info.available) {
      showUpdateBanner(info.version || "", info.url || "");
    } else if (manual) {
      toast(t("update.upToDate"));
    }
  } catch (e) {
    // Offline / parse problems: stay silent unless the user asked manually.
    if (manual) toast(t("update.offline"));
    return false;
  }
  return true;
}

btnCheckUpdate.addEventListener("click", () => checkForUpdates(true));

// ---------------------------------------------------------------------------
// First-run onboarding (C-19.7/C-19.8): step-by-step highlight tour shown
// ONCE per install (DB flag "onboarding_done"). Kept short — the steps after
// "add a folder" were removed (C-19.8): ① collapse arrow ② folder icon.
// ---------------------------------------------------------------------------
let onboardingActive = false;
let onboardingStep = 0;
const ONBOARD_STEPS = [
  { target: "#sidebar-toggle", key: "onboarding.s1", mode: "next" },
  { target: "#nav-folders", key: "onboarding.s2", mode: "finish" },
];

function onboardingShow() {
  if (onboardingActive) return;
  onboardingActive = true;
  onboardingStep = 0;
  renderOnboarding();
}

function onboardingHide() {
  onboardingActive = false;
  const root = document.getElementById("onboarding-root");
  if (root) root.remove();
  // One-time flag: never show again on this install.
  invoke("set_setting", { key: "onboarding_done", value: "1" }).catch(() => {});
}

function onboardingAdvance() {
  onboardingStep++;
  if (onboardingStep >= ONBOARD_STEPS.length) {
    onboardingHide();
  } else {
    renderOnboarding();
  }
}

function renderOnboarding() {
  const old = document.getElementById("onboarding-root");
  if (old) old.remove();
  const step = ONBOARD_STEPS[onboardingStep];
  const root = document.createElement("div");
  root.id = "onboarding-root";

  // Highlight box (pointer-events: none — the user still interacts).
  const box = document.createElement("div");
  box.className = "onboarding-box";
  let anchor = null;
  if (step.target) {
    anchor = document.querySelector(step.target);
  }
  if (anchor) {
    const r = anchor.getBoundingClientRect();
    box.style.left = `${r.left - 4}px`;
    box.style.top = `${r.top - 4}px`;
    box.style.width = `${r.width + 8}px`;
    box.style.height = `${r.height + 8}px`;
  } else {
    // Top-right "progress badge" area (step 5) — the badge may be hidden.
    box.style.right = "10px";
    box.style.top = "10px";
    box.style.width = "240px";
    box.style.height = "96px";
  }
  root.appendChild(box);

  // Bubble (interactive).
  const bubble = document.createElement("div");
  bubble.className = "onboarding-bubble";
  const text = document.createElement("div");
  text.className = "onboarding-bubble__text";
  text.textContent = t(step.key);
  const actions = document.createElement("div");
  actions.className = "onboarding-actions";
  const skip = document.createElement("button");
  skip.className = "btn btn--ghost";
  skip.textContent = t("onboarding.skip");
  skip.addEventListener("click", onboardingHide);
  actions.appendChild(skip);
  if (step.mode !== "wait-add" && step.mode !== "wait-photos") {
    const next = document.createElement("button");
    next.className = "btn btn--primary";
    next.textContent = t(step.mode === "finish" ? "onboarding.done" : "onboarding.next");
    next.addEventListener("click", onboardingAdvance);
    actions.appendChild(next);
  }
  bubble.appendChild(text);
  bubble.appendChild(actions);
  root.appendChild(bubble);

  // Position the bubble below the highlight; flip above when near bottom.
  const boxRect = box.getBoundingClientRect();
  bubble.style.left = `${Math.max(8, Math.min(boxRect.left, window.innerWidth - 340))}px`;
  const below = boxRect.bottom + 12;
  if (below + 130 < window.innerHeight) {
    bubble.style.top = `${below}px`;
  } else {
    bubble.style.top = `${Math.max(8, boxRect.top - 130)}px`;
  }
  document.body.appendChild(root);
}

// Hook: add-folder succeeded while step 3 is waiting → advance to step 4.
function onboardingAfterAdd() {
  if (onboardingActive && ONBOARD_STEPS[onboardingStep].mode === "wait-add") {
    onboardingAdvance();
  }
}
// Hook: user clicked the Photos nav while step 4 is waiting → advance.
function onboardingOnPhotosClicked() {
  if (onboardingActive && ONBOARD_STEPS[onboardingStep].mode === "wait-photos") {
    onboardingAdvance();
  }
}

// Initial load
(async () => {  try {
    await initI18n();
  } catch (e) {
    console.error(e);
  }
  applyStaticI18n();
    initTheme();
  applyFx();
  // Debug flag gates AI-confidence badges — read it before first render.
  try {
    debugMode = (await invoke("get_setting", { key: "debug" })) === "1";
  } catch (e) {
    debugMode = false;
  }
  loadPhotos();
  loadFolders();
  // Startup-fill fallback (C-19.11): the rAF inside loadPhotos may fire
  // before the webview's first layout and never get re-triggered, leaving
  // only a couple of rows rendered until the user switches pages. The fill
  // is idempotent and stops once the viewport is covered — retry on timers.
  setTimeout(fillGridIfNeeded, 300);
  setTimeout(fillGridIfNeeded, 1500);
  // Position the active-indicator WITHOUT the glide transition at startup —
  // boot never calls switchView, so the bar would otherwise sit at the top
  // of the sidebar (above the camera icon) until the first view switch.
  // Snap it straight to the active button (C-19.10).
  const indEl = document.getElementById("sidebar-indicator");
  const activeBtn = document.querySelector(".sidebar__btn--active");
  if (indEl && activeBtn) {
    indEl.style.transition = "none";
    indEl.style.transform = `translateY(${activeBtn.offsetTop + 6}px)`;
    // Re-enable the CSS transition after the snap — leaving the inline
    // `transition: none` in place would kill the glide animation for the
    // rest of the session (C-19.10).
    requestAnimationFrame(() => {
      indEl.style.transition = "";
    });
  }
  detectAndReportRenderer();
  // First-run onboarding (C-19.7): show only when the flag is missing —
  // the first release that ships the tour shows it to every install.
  try {
    const done = await invoke("get_setting", { key: "onboarding_done" });
    if (done !== "1") setTimeout(onboardingShow, 900);
  } catch (e) {
    /* keep silent — tour is optional */
  }
  // Startup update check (C-18): deferred so it never races first paint;
  // dev builds short-circuit in the backend (debug_assertions). Retried once
  // 30s later if the first attempt threw (transient network).
  setTimeout(async () => {
    if (!(await checkForUpdates(false))) {
      setTimeout(() => checkForUpdates(false), 30000);
    }
  }, 2000);
  // Reject-metrics warmup (C-19.3): start exposure/eyes analysis in the
  // background right after startup so the rejects page is populated by the
  // time the user visits it (progress badge shows while it runs).
  setTimeout(() => ensureRejectAnalysis(), 5000);
})();
