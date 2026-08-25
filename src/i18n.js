// Minimal i18n module (no framework — LIMITS.md).
// Messages are EMBEDDED at build time via locales/messages.js (generated
// from locales/*.json by scripts/gen-messages.js) — no fetch() at startup:
// the asset-protocol fetch failed on Windows AND macOS first load, leaving
// raw i18n keys on the buttons until a manual language switch (C-11.11).
// Language preference is persisted in SQLite via set_setting("language", ...).
const { invoke } = window.__TAURI__.core;

import { MESSAGES } from "./locales/messages.js";

export const SUPPORTED = ["en-US", "zh-CN"];
export const DEFAULT_LANG = "en-US";

let current = DEFAULT_LANG;
let messages = MESSAGES[DEFAULT_LANG];
const listeners = [];

export function currentLang() {
  return current;
}

function loadMessages(lang) {
  const msgs = MESSAGES[lang];
  if (!msgs) throw new Error(`locale not found: ${lang}`);
  return msgs;
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
 * language from settings if it differs. Synchronous + embedded — can never
 * fail on startup (C-11.11).
 */
export async function initI18n() {
  messages = loadMessages(current);
  let saved = null;
  try {
    saved = await invoke("get_setting", { key: "language" });
  } catch (e) {
    console.error(e);
  }
  if (saved && SUPPORTED.includes(saved) && saved !== current) {
    current = saved;
    messages = loadMessages(current);
  }
}

/** Switch language: persist + reload messages + notify listeners. */
export async function setLanguage(lang) {
  if (!SUPPORTED.includes(lang) || lang === current) return;
  current = lang;
  messages = loadMessages(lang);
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
