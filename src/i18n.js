// Minimal i18n module (no framework — LIMITS.md).
// Locale files: src/locales/<lang>.json (flat-ish nested keys, vue-i18n style).
// Language preference is persisted in SQLite via set_setting("language", ...).
const { invoke } = window.__TAURI__.core;

export const SUPPORTED = ["en-US", "zh-CN"];
export const DEFAULT_LANG = "en-US";

let current = DEFAULT_LANG;
let messages = {};
const listeners = [];

export function currentLang() {
  return current;
}

async function loadMessages(lang) {
  // Retry — the asset-protocol fetch can fail transiently at app startup
  // (observed on macOS: buttons showed raw i18n keys until a manual
  // language click re-fetched successfully, C-11.7).
  let lastErr = null;
  for (let attempt = 0; attempt < 4; attempt++) {
    try {
      const res = await fetch(`locales/${lang}.json`);
      if (!res.ok) throw new Error(`locale not found: ${lang} (HTTP ${res.status})`);
      return await res.json();
    } catch (e) {
      lastErr = e;
      await new Promise((r) => setTimeout(r, 300 * (attempt + 1)));
    }
  }
  console.error(`loadMessages(${lang}) failed after retries:`, lastErr);
  throw lastErr;
}

export function t(key, params) {
  let val = messages;
  for (const part of key.split(".")) {
    if (val == null) return key;
    val = val[part];
  }
  if (typeof val !== "string") return key;
  if (params) {
    for (const [k, v] of Object.entries(params)) {
      val = val.split(`{${k}}`).join(String(v));
    }
  }
  return val;
}

/** Re-render static HTML (data-i18n / data-i18n-placeholder / data-i18n-title / data-i18n-aria). */
export function applyStaticI18n() {
  document.documentElement.lang = current;
  document.title = t("app.title");
  document.querySelectorAll("[data-i18n]").forEach((el) => {
    el.textContent = t(el.dataset.i18n);
  });
  document.querySelectorAll("[data-i18n-placeholder]").forEach((el) => {
    el.placeholder = t(el.dataset.i18nPlaceholder);
  });
  document.querySelectorAll("[data-i18n-title]").forEach((el) => {
    el.title = t(el.dataset.i18nTitle);
  });
  document.querySelectorAll("[data-i18n-aria]").forEach((el) => {
    el.setAttribute("aria-label", t(el.dataset.i18nAria));
  });
}

/** Register a callback fired after the language actually changes. */
export function onLanguageChange(fn) {
  listeners.push(fn);
}

/**
 * Load default locale immediately (no flash), then apply the persisted
 * language from settings if it differs. On failure, retries in the
 * background until the locale arrives (UI self-heals — no raw keys).
 */
export async function initI18n() {
  try {
    await loadMessages(current);
  } catch (e) {
    // Fall back to retrying in the background (up to ~40s).
    let attempts = 0;
    const timer = setInterval(async () => {
      attempts++;
      if (attempts > 20) {
        clearInterval(timer);
        return;
      }
      try {
        await loadMessages(current);
        clearInterval(timer);
        applyStaticI18n();
      } catch (e2) {
        /* keep waiting */
      }
    }, 2000);
    console.error("initI18n: initial locale load failed, retrying in background:", e);
    throw e;
  }
  let saved = null;
  try {
    saved = await invoke("get_setting", { key: "language" });
  } catch (e) {
    console.error(e);
  }
  if (saved && SUPPORTED.includes(saved) && saved !== current) {
    current = saved;
    messages = await loadMessages(current);
  }
}

/** Switch language: persist + reload messages + notify listeners. */
export async function setLanguage(lang) {
  if (!SUPPORTED.includes(lang) || lang === current) return;
  current = lang;
  messages = await loadMessages(lang);
  try {
    await invoke("set_setting", { key: "language", value: lang });
  } catch (e) {
    console.error(e);
  }
  for (const fn of listeners) {
    try {
      fn();
    } catch (e) {
      console.error(e);
    }
  }
}
