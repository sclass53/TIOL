// CI PR check (C-16): bilingual support gate.
// 1. zh-CN.json and en-US.json must have IDENTICAL key sets (deep-flattened).
// 2. Every i18n key referenced from src/app.js / src/index.html must exist.
// 3. src/locales/messages.js must be in sync with the JSON sources
//    (regenerate with `node scripts/gen-messages.js` after editing JSON).
// Exits non-zero on any failure.
const fs = require("fs");
const path = require("path");

const root = path.join(__dirname, "..");
const localeDir = path.join(root, "src", "locales");

function flat(o, p = "", out = {}) {
  for (const k of Object.keys(o)) {
    const kk = p ? `${p}.${k}` : k;
    if (o[k] && typeof o[k] === "object" && !Array.isArray(o[k])) {
      flat(o[k], kk, out);
    } else {
      out[kk] = o[k];
    }
  }
  return out;
}

let failed = false;
const fail = (msg) => {
  console.error(`✗ ${msg}`);
  failed = true;
};

(async () => {
  // 1) Key parity between the two locales.
  const en = flat(JSON.parse(fs.readFileSync(path.join(localeDir, "en-US.json"), "utf8")));
  const zh = flat(JSON.parse(fs.readFileSync(path.join(localeDir, "zh-CN.json"), "utf8")));
  for (const k of Object.keys(en)) {
    if (!(k in zh)) fail(`en-US has a key missing from zh-CN: ${k}`);
  }
  for (const k of Object.keys(zh)) {
    if (!(k in en)) fail(`zh-CN has a key missing from en-US: ${k}`);
  }
  console.log(
    `✓ ${Object.keys(en).length} keys — en/zh parity: ${Object.keys(en).length === Object.keys(zh).length ? "OK" : "MISMATCH"}`
  );

  // 2) Referenced keys must exist.
  const js = fs.readFileSync(path.join(root, "src", "app.js"), "utf8");
  const html = fs.readFileSync(path.join(root, "src", "index.html"), "utf8");
  const used = new Set();
  for (const m of js.matchAll(/t\("([A-Za-z0-9.]+)"/g)) used.add(m[1]);
  for (const m of js.matchAll(/data-i18n="([A-Za-z0-9.]+)"/g)) used.add(m[1]);
  for (const m of html.matchAll(/data-i18n(?:-(?:placeholder|title|aria))?="([A-Za-z0-9.]+)"/g)) {
    used.add(m[1]);
  }
  // DOM-tag false positives from the t("...") regex (createElement etc.).
  const dom = new Set([
    "li", "span", "button", "canvas", "webgl", "div", "img",
    ".card", "tr", "td", "input", "select",
  ]);
  for (const k of used) {
    if (!(k in en) && !dom.has(k)) fail(`referenced i18n key is missing: ${k}`);
  }
  console.log(`✓ ${used.size} referenced keys checked`);

  // 3) messages.js must match the JSON sources byte-for-byte (same body the
  // generator produces, including the comment header).
  try {
    const messages = {};
    for (const lang of ["zh-CN", "en-US"]) {
      messages[lang] = JSON.parse(fs.readFileSync(path.join(localeDir, `${lang}.json`), "utf8"));
    }
    const expected =
      `// AUTO-GENERATED from locales/*.json — edit the JSON files, then run\n` +
      `// \`node scripts/gen-messages.js\` to refresh this module (C-11.11).\n` +
      `export const MESSAGES = ${JSON.stringify(messages, null, 2)};\n`;
    const actual = fs.readFileSync(path.join(localeDir, "messages.js"), "utf8");
    if (actual.trim() !== expected.trim()) {
      fail(`messages.js is out of sync with locales/*.json — run \`node scripts/gen-messages.js\``);
    } else {
      console.log("✓ messages.js in sync with locales/*.json");
    }
  } catch (e) {
    fail(`messages.js check failed: ${e.message}`);
  }

  if (failed) {
    console.error("❌ i18n checks failed");
    process.exit(1);
  }
  console.log("✅ all i18n checks passed");
})();
