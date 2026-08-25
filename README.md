<!-- Language Switcher -->
[![中文](https://img.shields.io/badge/中文-README-blue.svg)](README.zh.md)
[![English](https://img.shields.io/badge/English-README-blue.svg)](README.md)

# TIOL — AI Local Photo Manager

Supports **MacOS**, **Windows**, and **Linux**. Privacy-first, local-first photo management. All photos stay on your own hard drive. AI inference runs completely offline (SigLIP2 + user-defined zero-shot labels). No cloud services required.

## Features

- 💻 **Multiplatform support**: Supports Windows, MacOS, and Linux. Supports Nvidia CUDA, cpu, Apple CoreML, and many more
- 📁 **Directory Management**: Add/remove photo directories, file system monitoring (new/modified files are automatically queued for processing)
- 🔍 **Dual‑path Search**: **Semantic search** (describe what you're looking for, e.g., *"a cup of coffee"*) + **Tag search** (millisecond SQL)
- 🏷️ **Custom Labels**: AI tagging – enter any labels (Chinese or English) in the Tags tab; files are indexed automatically on changes, tagging runs on demand via the “AI Tagging” button
- 🖥️ **AI Engine Options**: auto / GPU / CPU / Apple CoreML (macOS native Neural Engine acceleration) – fully local
- 🔒 **Model Lockdown**: SHA256 checksum + resumable downloads + fallback to domestic mirrors, auto‑repair for corrupted models
- 💻 **Multiplatform support**: Supports Windows, MacOS, and Linux. Supports Nvidia CUDA, cpu, Apple CoreML, and many more

## Quick Start

```bash
# Dependencies: Rust 1.70+, tauri-cli 2.x (macOS also needs Xcode CLT; Windows needs VS Build Tools)
cargo tauri dev        # development mode
cargo tauri build      # production build (Windows → msi/nsis, macOS → .app/dmg)
```

On first launch, the AI model (~412 MB) will be downloaded automatically (mirror chain: hf‑mirror → openi → huggingface).  
**Before building/running on Windows**, copy `vendor/onnxruntime/win-x64/onnxruntime.dll` to the same directory as the executable.  
**On macOS**, you need a universal2 `onnxruntime.dylib` (official builds include CoreML EP) – see [BUILD.md](BUILD.md) for details.

## Data Locations

- Database / thumbnails:  
  Windows: `%APPDATA%\com.tiol.desktop`  
  macOS: `~/Library/Application Support/com.tiol.desktop`
- AI models:  
  Windows: `%LOCALAPPDATA%\com.tiol.desktop\models`  
  macOS: `~/Library/Caches/com.tiol.desktop/models`
