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
  navSettings: document.getElementById("nav-settings"),
  viewPhotos: document.getElementById("view-photos"),
  viewFolders: document.getElementById("view-folders"),
  viewSettings: document.getElementById("view-settings"),
  langOptions: document.getElementById("lang-options"),
  toggleHwDecode: document.getElementById("toggle-hw-decode"),
  hwDecodeHint: document.getElementById("hw-decode-hint"),
  btnRestart: document.getElementById("btn-restart"),
  gpuStatus: document.getElementById("gpu-status"),
  btnClearCache: document.getElementById("btn-clear-cache"),
  btnClearTags: document.getElementById("btn-clear-tags"),
  cacheHint: document.getElementById("cache-hint"),
  confirmOverlay: document.getElementById("confirm-overlay"),
  confirmText: document.getElementById("confirm-text"),
  confirmOk: document.getElementById("confirm-ok"),
  confirmCancel: document.getElementById("confirm-cancel"),
  taggingBadge: document.getElementById("tagging-badge"),
  taggingFill: document.getElementById("tagging-fill"),
  taggingCount: document.getElementById("tagging-count"),
  editOverlay: document.getElementById("edit-overlay"),
  editInput: document.getElementById("edit-input"),
  editSave: document.getElementById("edit-save"),
  editCancel: document.getElementById("edit-cancel"),
  searchInput: document.getElementById("search-input"),
  searchMode: document.getElementById("search-mode"),
  semanticSearchInput: document.getElementById("semantic-search-input"),
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
  els.viewPhotos.classList.toggle("view--hidden", !isPhotos);
  els.viewFolders.classList.toggle("view--hidden", !isFolders);
  els.viewSettings.classList.toggle("view--hidden", name !== "settings");
  els.navPhotos.classList.toggle("sidebar__btn--active", isPhotos);
  els.navFolders.classList.toggle("sidebar__btn--active", isFolders);
  els.navSettings.classList.toggle("sidebar__btn--active", name === "settings");
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
  renderTags();
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
  confirmDialog(t("settings.clearTagsConfirm"), async () => {
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
let aiProvider = null; // "auto" | "gpu" | "cpu" | "mlx"
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
  els.taggingBadge.hidden = remaining <= 0;
  if (remaining > 0) {
    els.taggingCount.textContent = t("tagging.remaining", { count: remaining });
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

// --- Custom tag management (MIGRATE1.md §2.3: user-defined zero-shot tags) ---
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
    li.className = "settings__tag-empty";
    li.textContent = t("settings.tagsEmpty");
    tagEls.list.appendChild(li);
    return;
  }
  for (const tg of tags) {
    const li = document.createElement("li");
    li.className = "settings__tag-item";
    const name = document.createElement("span");
    name.className = "settings__tag-name";
    name.textContent = tg.name;
    const meta = document.createElement("span");
    meta.className = "settings__tag-meta";
    meta.textContent = `${t("settings.tagThreshold")}: ${Number(tg.threshold).toFixed(2)} · ${t("settings.tagCount", { count: tg.photo_count })}`;
    const del = document.createElement("button");
    del.className = "btn btn--ghost";
    del.textContent = t("settings.removeTag");
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
    alert(t("settings.tagNameRequired"));
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
    els.photoGrid.innerHTML = `<div class="empty">${t("photos.empty")}</div>`;
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

// --- tag edit dialog (comma-separated, replaces the description editor) ---
let editPhoto = null;

async function openEditDialog(photo) {
  editPhoto = photo;
  // Prefill with the file's MANUAL tags (source=0) only.
  let manual = [];
  try {
    const tags = await invoke("get_file_tags", { fileId: photo.id });
    manual = (tags || [])
      .filter((tg) => tg.source === 0)
      .map((tg) => tg.name);
  } catch (e) {
    reportJs("get-tags", String(e));
  }
  els.editInput.value = manual.join(", ");
  els.editOverlay.hidden = false;
  els.editInput.focus();
  els.editInput.select();
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
    const tags = els.editInput.value
      .split(",")
      .map((s) => s.trim())
      .filter((s) => s.length > 0);
    const updated = await invoke("update_tags", { fileId: p.id, tags });
    p.tags = updated.tags || [];
    if (p._card) renderCardMeta(p._card, p);
    runSearch();
  } catch (e) {
    alert(String(e));
  }
}

/// Re-render the meta row of one card (name + tag list) after a tag edit.
function renderCardMeta(card, p) {
  const meta = card.querySelector(".card__meta");
  if (!meta) return;
  meta.querySelectorAll(".card__desc").forEach((el) => el.remove());
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
els.editInput.addEventListener("keydown", (e) => {
  if (e.key === "Enter") closeEditDialog(true);
  if (e.key === "Escape") closeEditDialog(false);
});

function buildCard(p) {
  const card = document.createElement("div");
  card.className = "card";
  const thumb = document.createElement("div");
  thumb.className = "card__thumb";
  const img = document.createElement("img");
  img.alt = p.filename;
  img.loading = "lazy";
  img.onerror = () => {
    thumb.textContent = p.filename;
    thumb.classList.add("card__thumb--placeholder");
    img.remove();
  };
  thumb.appendChild(img);
  thumb._img = img;
  thumb._photo = p;
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

  const meta = document.createElement("div");
  meta.className = "card__meta";
  const metaRow = document.createElement("div");
  metaRow.className = "card__meta-row";
  const nameEl = document.createElement("span");
  nameEl.className = "card__meta-name";
  nameEl.textContent = p.filename;
  nameEl.title = p.path;
  const editBtn = document.createElement("button");
  editBtn.className = "card__edit";
  editBtn.textContent = "✎";
  editBtn.title = t("card.edit.title");
  editBtn.addEventListener("click", async (ev) => {
    ev.stopPropagation();
    openEditDialog(p);
  });
  metaRow.appendChild(nameEl);
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
    renderPhotos(photos);
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
    this.els.meta.textContent = `${photo.filename} · ${formatSize(photo.size)}`;
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

function showContextMenu(x, y, photo) {
  hideContextMenu();
  const menu = document.createElement("div");
  menu.className = "ctx-menu";
  const item = document.createElement("button");
  item.className = "ctx-menu__item";
  item.textContent = t("menu.reveal");
  item.addEventListener("click", () => {
    hideContextMenu();
    invoke("reveal_in_folder", { path: photo.path }).catch((e) => alert(String(e)));
  });
  menu.appendChild(item);
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
      renderPhotos(res);
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
    renderPhotos(nameRes || []);
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
