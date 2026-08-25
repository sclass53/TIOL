# BUILD.md — 构建 TIOL（Windows / macOS）

> 面向贡献者的构建指南。架构与产品说明见 ADD.md / MIGRATE1.md，变更历史见 CHANGES.md。
> 模型下载与哈希锁定见 `src-tauri/src/ai/model_lock.rs`。

## 0. 技术栈速览

- **Tauri v2**（Rust 后端 + 原生 HTML/CSS/JS 前端，**无打包器、无 node_modules**）
- 前端直接嵌入二进制（`frontendDist: ../src`），构建**不需要 npm install**
- AI：纯 SigLIP2 int8（vision/text 编码器 + tokenizer，约 412MB，运行时下载），ORT 2.0.0-rc.13 以 `load-dynamic` 方式加载本机 onnxruntime 动态库
- SQLite（rusqlite bundled，源码编译，无外部依赖）

## 1. 环境准备

### Windows

| 依赖 | 版本 | 说明 |
|---|---|---|
| Rust | 1.70+（`rust-version = "1.70"`） | https://rustup.rs，MSVC toolchain |
| VS Build Tools | 最新 | 勾选 **"使用 C++ 的桌面开发"**（link.exe / lib.exe 必需） |
| tauri-cli | 2.x | `cargo install tauri-cli --locked`（构建走 `cargo tauri`，不依赖 npm） |
| WebView2 | Win10/11 自带 | 老系统需单独安装 |
| Node.js | 18+ | **仅**镜像代理脚本（§4.2）与模型预下载脚本需要 |

### macOS

| 依赖 | 版本 | 说明 |
|---|---|---|
| Rust | 1.70+ | rustup，添加目标：`rustup target add aarch64-apple-darwin x86_64-apple-darwin` |
| Xcode Command Line Tools | 最新 | `xcode-select --install`（clang + 链接器） |
| tauri-cli | 2.x | 同上 |
| Rust 目标 | — | Apple Silicon 构建 `aarch64-apple-darwin`；Intel 用 `x86_64-apple-darwin`；通用二进制可在任一目标交叉编译后 lipo 合并 |

> **macOS 打包前必须生成图标**：当前仓库只有小尺寸 `icon.png/icon.ico`，macOS 需要 `icon.icns`：
> ```bash
> # 准备一张 1024×1024 的 PNG，然后：
> npx tauri icon path/to/icon-1024.png   # 生成 icons/ 全套（icns/ico/png）
> ```

## 2. ONNX Runtime 动态库（关键前置）

应用通过 `ort::load-dynamic` 在**运行时**加载本机 ONNX Runtime，编译期不需要它；但**运行/打包必须提供**：

- **Windows**：`vendor/onnxruntime/win-x64/onnxruntime.dll`（仓库已内置，CPU 版）
  - 开发运行：复制到 `src-tauri/target/debug/onnxruntime.dll`（`cargo tauri dev` 前）
  - 发布：复制到 `src-tauri/target/release/onnxruntime.dll`（与 TIOL.exe 同目录）
  - **警告（C-11.2）**：Win11 24H2+ 系统内置了 `C:\Windows\system32\onnxruntime.dll`（最小版，CPU EP 无 ConvInteger 内核）。若 exe 旁缺少我们的 DLL，ort 会经 PATH 误加载系统版，表现为 cpu 模式报 `Could not find an implementation for ConvInteger(10)`。应用启动时会自动把 `ORT_DYLIB_PATH` 钉到 exe 同目录的 DLL（存在时），**但 exe 旁仍必须放我们的 DLL**。
  - 注意：该 DLL 不含 DirectML/CUDA EP——`gpu` 模式在本仓库环境下实为 CPU 回退（标签显示后端名，属已知外观问题）
- **macOS**：仓库**未内置**，需要从官方 Releases 获取 **universal2 且编译时启用 CoreML EP 的 `libonnxruntime.dylib`**（微软官方 macOS 构建默认含 CoreML EP）：
  - **版本必须 ≥ 1.17**（ort 2.0.0-rc.13 的 `ORT_API_VERSION = 17`，旧版本加载时报 `BadVersion`——推荐 **1.20.x**，如 `onnxruntime-osx-universal2-1.20.1.tgz`）
  - 开发运行：放到可执行文件同目录，或设置环境变量 `ORT_DYLIB_PATH=/path/to/libonnxruntime.dylib`
  - 发布：随 .app 打包（`bundle.resources` 放入 `Contents/Frameworks` 并签名），或首次启动下载到应用缓存目录后设置 `ORT_DYLIB_PATH`（参见 CHANGES.md C-10.6 的方案 B）
  - CoreML.framework 为 macOS 系统自带，**无需安装**；但 dylib 本身必须分发
- **测试**：`cargo test` 的二进制在 `target/debug/deps/` 下找不到 DLL，跑真实模型测试前必须：
  ```powershell
  $env:ORT_DYLIB_PATH = "E:\ImageManager\src-tauri\target\debug\onnxruntime.dll"
  ```

## 3. 构建

```bash
# 开发模式（热重载 Rust；前端为嵌入资源，改动需重新编译）
cargo tauri dev

# 发布构建（编译 release + 生成安装包）
cargo tauri build

# 只编译不打包（快速验证；产物为裸 exe/dylib 可执行文件）
cargo build --release
```

产物位置（Windows）：

```
src-tauri/target/release/
├── TIOL.exe            # 裸可执行文件（需同目录 onnxruntime.dll）
└── bundle/
    ├── msi/TIOL_*.msi          # WiX 安装包
    └── nsis/TIOL_*-setup.exe   # NSIS 安装包（分发推荐）
```

macOS 产物为 `src-tauri/target/release/bundle/macos/TIOL.app`（+ dmg）。

### 打包工具说明

- `cargo tauri build` 首次会**自动下载 NSIS（Windows）/ WiX（Windows MSI）/ dmg 工具**（tauri-cli 内置逻辑）。若网络受限下载失败：
  - 用 `cargo build --release` 得到裸二进制，手动组装便携版（exe/dylib + onnxruntime + 说明）
  - 或预先把工具放入 tauri-cli 的缓存目录（`%LOCALAPPDATA%\tauri\`）
- **代码签名**：Windows 在 `tauri.conf.json` 的 `bundle.windows.certificateThumbprint` 配置证书；macOS 用 `signingIdentity` + `notarytool` 公证（未签名产物会被 Gatekeeper 拦截，贡献者本地运行不受影响）

## 4. 镜像源设置

### 4.1 crates.io（正常网络，推荐）

国内/网络慢的用户直接配置镜像（二选一）：

```toml
# ~/.cargo/config.toml 或 src-tauri/.cargo/config.toml
[source.crates-io]
replace-with = "rsproxy-sparse"

[source.rsproxy-sparse]
registry = "sparse+https://rsproxy.cn/index/"
```
或 ustc：
```toml
[source.ustc]
registry = "sparse+https://mirrors.ustc.edu.cn/crates.io-index/"
```

> 注意：`src-tauri/.cargo/config.toml` 已被 .gitignore 排除——本仓库作者环境用它指向本地代理，**贡献者请使用自己的全局配置**，不要提交该文件。

### 4.2 严格受限网络（无外网证书 / 沙箱）

本作者环境 schannel TLS 被限制（cargo 直连报 `SEC_E_NO_CREDENTIALS`），但 Node TLS 可用。方案：

```bash
# 1. 启动本地代理：把 cargo 稀疏索引转发到 ustc 镜像并重写 .crate 下载地址
node scripts/cargo-proxy.js          # 监听 127.0.0.1:8013

# 2. 让 cargo 走代理（src-tauri/.cargo/config.toml 已配 replace-with='local-proxy'）
$env:CARGO_HOME = "E:\ImageManager\.cargo-home"   # 本机离线 vendored 缓存
cargo fetch

# 3.（一次性）旧 crate 时间戳修复：ustc 部分 tar 为 epoch-0，Windows 解包失败，
#    此脚本按 Cargo.lock 重新打包并预置进本地缓存
node scripts/vendor-crates.js

# 4. 完全离线构建（索引已缓存后）：
$env:CARGO_NET_OFFLINE = "true"
cargo build
```

- `.cargo-home/` 是作者机器的离线依赖缓存（已 gitignore，**勿提交**）
- 若你的环境能直连 crates.io 或 rsproxy，**不需要代理脚本**，用 §4.1 即可

### 4.3 AI 模型（内置下载器，用户无需操作）

- 模型目录：应用数据目录下 `models/`（Windows `%LOCALAPPDATA%\com.tiol.desktop\models`；macOS `~/Library/Caches/com.tiol.desktop/models`——以 tauri `path()` 解析为准）
- 首次启动自动下载 3 个文件（vision/text int8 + tokenizer，约 412MB），镜像链：**hf-mirror.com → openi.org.cn → huggingface.co**
- 模型锁：`model_lock.rs` 硬编码 URL + size + SHA256，校验失败即删重下（断点续传 `.part` + 原子改名）
- 手动预下载（本机代理网络）：`node scripts/download-models.js <目标目录>`，之后拷入模型目录即可跳过下载
- 开发环境已装好模型时，测试可用 `TIOL_MODEL_DIR` 指向模型目录

## 5. 测试

```bash
cargo test          # 单元测试（db/扫描/标签搜索，无需模型）
```

真实模型测试（默认 `#[ignore]`，需模型 + ORT 库 + 测试图片）：

```powershell
$env:ORT_DYLIB_PATH  = "E:\ImageManager\src-tauri\target\debug\onnxruntime.dll"
$env:TIOL_MODEL_DIR  = "$env:LOCALAPPDATA\com.tiol.desktop\models"   # 或任意模型目录
$env:TIOL_TEST_IMAGES = "E:\ImageManager\test_imgs"                  # 任意含图片的目录
cargo test -- --ignored
```

- `text_embed_never_panics`：SigLIP 文本编码（人/中文/英文/空串回归）
- `siglip_tag_match_sanity`：图像-标签余弦 sanity（top-1 应为真实匹配）
- `preprocess_experiment`：诊断用，输出多图×多标签余弦供调阈值参考

## 6. 常见问题排查

| 现象 | 原因与解决 |
|---|---|
| `Could not connect to 127.0.0.1:8013` | 本地代理没启动（§4.2）或改用 `CARGO_NET_OFFLINE=true` / 全局镜像 |
| `link.exe` / LNK 错误 | 未装 VS Build Tools C++ 工作负载 |
| 运行报找不到 onnxruntime | DLL/dylib 未放对位置（§2）或未设 `ORT_DYLIB_PATH` |
| 模型一直"正在下载" | 网络受限：手动预下载（§4.3）；镜像链会自动回退 |
| macOS 打包报 icon 错误 | 先生成 `icon.icns`（§1 macOS） |
| macOS 加载 dylib 被拒 | 未签名/未公证；或应用 entitlements 需 `com.apple.security.cs.disable-library-validation` |
| 打包报 MSI/NSIS 工具下载失败 | 网络受限：`cargo build --release` 手动组装便携版（§3） |

## 7. 运行时数据位置

| 数据 | Windows | macOS |
|---|---|---|
| SQLite 数据库 | `%APPDATA%\com.tiol.desktop\db.sqlite` | `~/Library/Application Support/com.tiol.desktop/db.sqlite` |
| 缩略图缓存 | `%APPDATA%\com.tiol.desktop\cache\thumbnails` | 同构（`cache/thumbnails`，上限 500MB 自动清理） |
| AI 模型 | `%LOCALAPPDATA%\com.tiol.desktop\models` | `~/Library/Caches/com.tiol.desktop/models` |
| 旧数据迁移 | 旧版 `com.imagemanager.demo` 首次启动自动迁移 | — |

重置应用：删除上述目录后重启。

## 8. 推理后端说明（设置页可选）

| 模式 | 行为 |
|---|---|
| auto | 探测 CUDA → DirectML → CoreML → CPU，真实冒烟推理验证后选用 |
| gpu | 强制 GPU 提供链（本仓库内置 DLL 无 GPU EP 时实际回退 CPU，标签显示真实后端） |
| cpu | 仅 CPU |
| mlx | Apple 加速器：macOS 走 CoreML（ort 2.0.0-rc.13 无 MLX EP，CoreML 即 Apple 原生加速）；非 Apple 平台回退 CPU |

