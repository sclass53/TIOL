# BUILD.md — 构建 TIOL 为 Windows 可执行程序（exe）

## 1. 环境准备（前置条件）

| 依赖 | 版本要求 | 说明 |
|---|---|---|
| Rust | 1.70+（实测 1.98） | https://rustup.rs |
| tauri-cli | 2.x | `cargo install tauri-cli --locked`（或全局已装） |
| VS Build Tools | 最新 | Windows 安装器选 **"使用 C++ 的桌面开发"** 工作负载（链接器 link.exe 需要） |
| WebView2 Runtime | Win10/11 自带 | 老系统需到微软官网安装 |
| Node.js | 18+（可选） | 仅 `npm run tauri dev` 调试时需要；**纯构建不依赖 npm** |

> 本项目前端是原生 HTML/CSS/JS（无打包器），`frontendDist: ../src` 的静态文件
> 会作为资源直接嵌入二进制，因此构建**不需要** npm install / node_modules。

## 2. 构建发布版（exe）

在仓库根目录 `E:\ImageManager` 执行：

```bash
# 一步构建（编译 release + 生成安装包）
cargo tauri build

# 只编译不打包（快速验证能否编译通过）
cargo build --release
```

## 3. 构建产物位置

```
src-tauri/target/release/
├── TIOL.exe                          # 裸可执行文件（绿色版，可直接运行）
└── bundle/
    ├── msi/TIOL_0.1.0_x64_en-US.msi  # WiX 安装包
    └── nsis/TIOL_0.1.0_x64-setup.exe # NSIS 安装包
```

分发时给用户 **NSIS 的 setup.exe** 或 **MSI** 即可，双击安装，无需 WebView2 之外的任何运行时。

## 4. 版本号与产品信息

- 版本号：`src-tauri/tauri.conf.json` 的 `version`（打包文件名会带上）
- 产品名/二进制名：`productName` / `mainBinaryName`（当前为 TIOL）
- 应用标识：`identifier`（当前 `com.tiol.desktop`，决定数据目录 `%APPDATA%/com.tiol.desktop`）

## 5. 常见问题排查

| 现象 | 原因与解决 |
|---|---|
| `link.exe` 找不到 / LNK 错误 | 未装 VS Build Tools 的 C++ 工作负载 |
| 报 `icon.ico is not in 3.00 format` | 图标损坏，用 PIL 重新生成 256×256 ICO（见 INSTALLING.md §7） |
| `cargo tauri build` 下载依赖慢/失败 | 配置国内镜像（.cargo/config.toml 指向 rsproxy / ustc）后重试 |
| 杀毒软件拦截/误报 | Tauri 产物未签名；可在 `tauri.conf.json` 配置 `bundle.windows.certificateThumbprint` 做代码签名 |
| 打包报 MSI/NSIS 相关错误 | 单独跑 `cargo build --release` 确认编译本身没问题，再查 bundle 日志 |

## 6. 日常开发调试（非打包）

```bash
# 开发模式：热重载 Rust 改动、前端文件刷新即生效
cargo tauri dev

# （可选）若 npm 侧 CLI 可用，也可：
npm run tauri dev
```

> 注意：本仓库的 npm 侧 `@tauri-apps/cli` 曾因环境问题未正确安装，
> 构建与开发统一走 `cargo tauri`（tauri-cli 2.x）这条路径最稳。

## 7. 数据与缓存位置（运行时）

- SQLite 数据库：`%APPDATA%\com.tiol.desktop\db.sqlite`
- 缩略图缓存：`%APPDATA%\com.tiol.desktop\cache\thumbnails`（上限 500MB，自动清理）
- 重命名前的旧数据（`com.imagemanager.demo`）会在首次启动时自动迁移
- 重置应用：删除上述目录后重启
