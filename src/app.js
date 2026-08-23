// Tauri v2 global API (withGlobalTauri) — static frontend, no bundler
const { invoke, convertFileSrc } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { open: openDialog } = window.__TAURI__.dialog;

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
  cacheHint: document.getElementById("cache-hint"),
  editOverlay: document.getElementById("edit-overlay"),
  editInput: document.getElementById("edit-input"),
  editSave: document.getElementById("edit-save"),
  editCancel: document.getElementById("edit-cancel"),
  searchInput: document.getElementById("search-input"),
  descSearchInput: document.getElementById("desc-search-input"),
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

els.navPhotos.addEventListener("click", () => switchView("photos"));
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
  // Deterministic initial thumbnail load: explicitly request the first
  // screenful top-down (the observer's initial callback proved unreliable
  // for cards already in the DOM — it skipped the first rows).
  // NOTE: iterate BOTTOM-UP — setThumb unshifts to the queue head, so the
  // last processed card would win the front; reversed order keeps card 0
  // (top row) first in the serve order.
  for (let i = renderedCount - 1; i >= 0; i--) {
    const card = els.photoGrid.children[i];
    if (card && card._img && card._photo) {
      // Mark as handled by the initial load: the observer/click must not
      // re-queue or re-prioritize these (that reordered rows).
      card._img._initial = true;
      setThumb(card._img, card._photo);
    }
  }
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

function setThumb(img, photo) {
  const cached = thumbSrcCache.get(photo.path);
  if (cached !== undefined) {
    if (cached) img.src = cached;
    else showPlaceholder(img, photo);
    return;
  }
  const idx = thumbQueue.findIndex((q) => q.p.path === photo.path);
  if (idx >= 0) {
    // Scroll-time cards move to the front (viewport-first). Initial-screenful
    // cards keep their top-down serve order — reprioritizing them reorders
    // the rows (the observer's initial callback did exactly that).
    if (!img._initial) {
      const item = thumbQueue.splice(idx, 1)[0];
      thumbQueue.unshift(item);
    }
    return;
  }
  // Already handled by the explicit initial load (queued, in flight or
  // served) — never enqueue a duplicate that would jump the queue.
  if (img._initial) return;
  // New requests go to the front (viewport-first instead of FIFO).
  thumbQueue.unshift({ img, photo });
  pumpThumbs();
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
      .catch(() => {
        thumbSrcCache.set(photo.path, "");
        showPlaceholder(img, photo);
      })
      .finally(() => {
        thumbInFlight--;
        pumpThumbs();
      });
  }
}

// --- description edit dialog (in-app, replaces the native prompt()) ---
let editPhoto = null;

function openEditDialog(photo) {
  editPhoto = photo;
  els.editInput.value = photo.description || "";
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
    await invoke("update_description", { id: p.id, description: els.editInput.value });
    p.description = els.editInput.value;
    runSearch();
  } catch (e) {
    alert(String(e));
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
  thumbObserver.observe(thumb);
  card._photo = p;

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
  if (p.description) {
    const descEl = document.createElement("div");
    descEl.className = "card__desc";
    descEl.textContent = p.description;
    descEl.title = p.description;
    meta.appendChild(descEl);
  }
  card.appendChild(thumb);
  card.appendChild(meta);
  // click: prioritize this card's thumbnail (queue head), then open preview
  card.style.cursor = "pointer";
  card.addEventListener("click", () => {
    setThumb(img, p);
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
// Dual search (name + description) with 500ms debounce per LIMITS.md:145.
// When both boxes have text, results are intersected (AND).
// ---------------------------------------------------------------------------
let searchTimer = null;
function scheduleSearch() {
  clearTimeout(searchTimer);
  searchTimer = setTimeout(runSearch, 500);
}
els.searchInput.addEventListener("input", scheduleSearch);
els.descSearchInput.addEventListener("input", scheduleSearch);

async function runSearch() {
  const qName = els.searchInput.value.trim();
  const qDesc = els.descSearchInput.value.trim();
  if (!qName && !qDesc) {
    loadPhotos();
    return;
  }
  try {
    let nameRes = null;
    let descRes = null;
    if (qName) nameRes = await invoke("search_files", { query: qName });
    if (qDesc) descRes = await invoke("search_description", { query: qDesc });
    let merged;
    if (nameRes && descRes) {
      const descPaths = new Set(descRes.map((r) => r.path));
      merged = nameRes.filter((r) => descPaths.has(r.path));
    } else {
      merged = nameRes || descRes || [];
    }
    renderPhotos(merged);
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
  loadPhotos();
  loadFolders();
  detectAndReportRenderer();
})();
