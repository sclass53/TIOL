<!-- Language Switcher -->
[![中文](https://img.shields.io/badge/中文-README-blue.svg)](README.zh.md)
[![English](https://img.shields.io/badge/English-README-blue.svg)](README.md)

# TIOL — AI Local Photo Manager

隐私优先，本地优先的照片管理应用。自动为您的摄影作品打标签。支持快速，智能的搜索。

![示例图片](examples/example.png)

支持 **MacOS**, **Windows**, 和**Linux**。

照片全程留在您自己的硬盘上，AI 推理完全离线运行，不需要任何云端服务。

## 功能

- 💻 **多平台支持**: 支持Windows, MacOS, 和Linux. 支持英伟达 CUDA, cpu, 苹果 CoreML, 等等
- 📷 **基于镜头分类**: 筛选不同镜头、不同焦段下拍摄的照片。
- 📁 **目录管理**：添加/移除照片目录，文件系统监控（新增/修改自动入队处理）
- 🔍 **双路径搜索**：**语义搜索**（描述要找的内容，如 "a cup of coffee"）+ **标签搜索**（快速）
- 🏷️ **自定义标签**：AI 打标——在“标签”页输入任意标签（中英文均可）；文件变更自动索引.
- 🖥️ **AI 引擎可选**：auto / GPU / CPU / Apple CoreML（Neural Engine 原生加速），全本地运行
- 🔒 **模型锁定**：SHA256 校验 + 断点续传 + 国内镜像回退，模型损坏自动修复

## 安装

[点击查看打包好的版本](https://github.com/sclass53/TIOL/releases)
[官方网站](tiol.netlify.app)

## 快速开始

```bash
# 依赖：Rust 1.70+、tauri-cli 2.x（macOS 另需 Xcode CLT；Windows 另需 VS Build Tools）
cargo tauri dev        # 开发模式
cargo tauri build      # 发布构建（Windows 生成 msi/nsis，macOS 生成 .app/dmg）
```

首次启动自动下载 AI 模型（约 412MB，hf-mirror → openi → huggingface 镜像链）。
**Windows 构建/运行前**需把 `vendor/onnxruntime/win-x64/onnxruntime.dll` 复制到可执行文件同目录；**macOS** 需自备 universal2 版 `onnxruntime.dylib`（官方构建含 CoreML EP）——详见 [BUILD.md](BUILD.md)。

## 数据位置

- 数据库/缩略图：Windows `%APPDATA%\com.tiol.desktop`；macOS `~/Library/Application Support/com.tiol.desktop`
- AI 模型：Windows `%LOCALAPPDATA%\com.tiol.desktop\models`；macOS `~/Library/Caches/com.tiol.desktop/models`
