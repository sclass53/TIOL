// Regenerates src/locales/messages.js from src/locales/*.json.
// The embedded module makes startup i18n deterministic (no fetch() — the
// asset-protocol fetch failed at startup on Windows AND macOS, showing raw
// i18n keys until a manual language switch; C-11.11).
// Usage: node scripts/gen-messages.js
const fs = require('fs');
const path = require('path');

const dir = path.join(__dirname, '..', 'src', 'locales');
const out = path.join(dir, 'messages.js');

const langs = ['zh-CN', 'en-US'];
const messages = {};
for (const lang of langs) {
  messages[lang] = JSON.parse(fs.readFileSync(path.join(dir, `${lang}.json`), 'utf8'));
}

const body = `// AUTO-GENERATED from locales/*.json — edit the JSON files, then run
// \`node scripts/gen-messages.js\` to refresh this module (C-11.11).
export const MESSAGES = ${JSON.stringify(messages, null, 2)};
`;

fs.writeFileSync(out, body);
console.log(`wrote ${out} (${Buffer.byteLength(body)} bytes)`);
