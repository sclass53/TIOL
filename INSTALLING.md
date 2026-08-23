# Installing & Running — TIOL (LIMITS.md)

## Prerequisites (LIMITS.md §2.2)
- Rust 1.70+ (tested 1.98)
- Node.js 18+ + npm (tested v22.19 / 10.9)
- Windows: Visual Studio Build Tools (C++ workload) + WebView2 (preinstalled on Win10/11)
- macOS: Xcode Command Line Tools

## 1. Install
```bash
# repo root E:\ImageManager
npm install          # installs @tauri-apps/cli
# Rust deps are fetched on first cargo check/build
```

## 2. Dev
```bash
npm run tauri dev
# or
cargo tauri dev      # if tauri-cli installed globally
```
Window 1290×800, left 48px nav, flat dark theme (--bg-primary #1C1C1E etc., no animation).

## 3. Build
```bash
npm run tauri build
# output: src-tauri/target/release/bundle/
# Windows: .msi/.exe  macOS: .app/.dmg  Linux: .deb/.AppImage
# Prod binary ~ 5-10 MB (uses OS WebView)
```

## 4. Data Locations
- SQLite: `%APPDATA%/com.tiol.desktop/db.sqlite` (or `~/Library/Application Support/` on macOS, `~/.config/` on Linux)
- Thumbnails: `{app_data}/cache/thumbnails/` — max 500 MB LRU by mtime
- First run after the rename auto-migrates data from the old `com.imagemanager.demo` dir (DB + thumbnail cache).
- To reset: delete `db.sqlite` and `cache/thumbnails/`

## 5. Features (Demo, pure local, no network)
- `add_folder(path)` → inserts `folders`, triggers incremental scan
- `scan_folders` → manual rescan all folders (chunks of 2)
- `watcher` → `notify` + 3s debounce → auto rescan (tokio background)
- `search_files(query)` → MockSkill hard-coded map (风景→landscape, 猫→cat …) + LIKE fallback
- `get_photos(folder_id?)` / `get_folders` / `get_thumbnail(path)` → 360px JPEG

## 6. Verify
```bash
cd src-tauri
cargo check
cargo clippy
cargo test   # add if tests present
```
Frontend: vanilla HTML/CSS/JS, no framework, BEM, no animation/transition/filter/gradient/shadow — only 1px borders + 4px radius.

## 7. Troubleshooting
- `icon.ico is not in 3.00 format` → regenerated via `icons/icon.ico` (PIL 256px)
- `tauri-build unknown field externalBin` → removed field (v1.5 config)
- First `cargo check` downloads ~460 crates via ustc mirror.

## 8. Future
Replace `ai::MockSkill` with ONNX Runtime or remote API; keep DB/scanner modules isolated for service extraction.
