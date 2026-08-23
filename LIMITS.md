# 项目总览：AI 本地照片素材管理工具 (Tauri Demo 版)

## 1. 项目定位与原则
- **定位**：本地优先、跨平台的轻量级 AI 照片素材管理桌面端。
- **版本说明**：**当前为 V1.0 Demo 演示版**，纯本地运行，无任何网络请求、用户登录或云端依赖。未来可扩展后端服务（如多端同步），但 Demo 版不包含。
- **核心原则**：
  1. **纯本地**：所有数据存储在本地 SQLite，不联网。
  2. **极简扁平 UI**：无动画、无毛玻璃、无渐变、无多余装饰，采用现代扁平风格（纯色、4px 圆角、清晰分割线）。
  3. **资源节约**：严格控制 CPU、内存和磁盘占用。
  4. **技术栈**：Tauri (Rust 后端 + Web 前端)，前端不依赖大型框架，使用原生 HTML/CSS/JS。

---

## 2. 技术栈与构建配置

### 2.1 总览
- **框架**：Tauri v1/v2（Rust 后端 + 系统 WebView 前端）
- **Rust 版本**：1.70+
- **前端**：原生 HTML5 + CSS3 + ES6 JavaScript（无框架，保持轻量）
- **数据库**：SQLite3（通过 Rust 的 `rusqlite` 库操作）
- **图像处理**：Rust 的 `image` 库生成缩略图
- **文件监控**：Rust 的 `notify` 库（跨平台文件系统监听）
- **并发**：Rust 的 `tokio` 异步运行时 + 线程池

### 2.2 开发环境与产物体积
- **开发工具**：只需安装 Rust、Node.js（用于 Tauri 构建）和系统依赖（Windows 需 VS 构建工具，macOS 需 Xcode）。
- **产物体积**：基础应用打包后约 **3-10 MB**（不含 WebView，因使用系统原生 WebView）。
- **内存占用**：空载应用约 **30-80 MB**，功能丰富后预计 **100-200 MB**，远低于 Electron。

---

## 3. 项目目录结构
```
src-tauri/                # Rust 后端
├── src/
│   ├── main.rs           # 入口，启动 Tauri 应用
│   ├── db/
│   │   └── mod.rs        # SQLite 操作封装 (rusqlite)
│   ├── scanner/
│   │   └── mod.rs        # 文件扫描 (增量扫描)
│   ├── watcher/
│   │   └── mod.rs        # 文件系统监控 (notify)
│   ├── ai/
│   │   └── mod.rs        # AI 搜索代理 (Mock)
│   └── utils/
│       └── mod.rs        # 路径处理、缩略图缓存
├── Cargo.toml
└── tauri.conf.json       # Tauri 配置 (窗口、权限等)

src/                      # 前端静态资源 (HTML/CSS/JS)
├── index.html            # 主页面
├── styles.css            # 全局样式 (扁平、现代化)
├── app.js                # 前端主逻辑 (与后端通信)
└── assets/               # 图标等资源
```

---

## 4. 前端 UI 设计（扁平化、现代化，无动画）

### 4.1 全局样式 Token (CSS 变量)
```css
:root {
    --bg-primary: #1C1C1E;
    --bg-surface: #2C2C2E;
    --bg-elevated: #3A3A3C;
    --separator: #48484A;
    --text-primary: #F2F2F7;
    --text-secondary: #8E8E93;
    --accent: #0A84FF;
    --radius: 4px;
    --font: -apple-system, "Segoe UI", Roboto, sans-serif;
}
```

### 4.2 主窗口布局
- **尺寸**：默认 1200×800，可缩放。
- **布局**：`flex` 水平排列（左侧菜单 + 右侧内容）。
- **背景**：`#1C1C1E`，无圆角。

#### 左侧菜单栏 (宽 48px)
- 垂直排列，图标按钮（`📷`、`📁`），选中时图标变 `#0A84FF`，左侧显示 2px 竖条。
- 右侧 1px 分割线。

#### 右侧内容区 (Stack 切换)
- **照片视图**：
  - 搜索框：`<input>`，高 36px，背景 `#2C2C2E`，圆角 4px，无边框。
  - 缩略图网格：flex wrap，每个卡片 180x180，圆角 4px，边框 `1px solid #3A3A3C`，悬停边框 `#0A84FF`。
  - 底部状态栏：显示照片总数。
- **目录视图**：
  - 顶部操作栏：添加目录按钮（文字 `+ 添加目录`，主色）、刷新按钮（图标）。
  - 列表：每个项显示路径和照片数，背景 `#2C2C2E`，圆角 4px，间距 2px。
  - 底部状态栏：显示管理的文件夹数量。

> **严格禁止**：动画、过渡、毛玻璃、渐变、阴影（除必要的边框），所有交互瞬间切换。

---

## 5. 后端 Rust 核心逻辑

### 5.1 数据库操作
- 使用 `rusqlite` 操作 SQLite。
- 表结构同之前方案（`folders`, `files`, `tags`, `file_tags`）。
- 数据库文件存放在 `app_dir/db.sqlite`。

### 5.2 文件扫描（增量扫描）
- **启动时**：读取所有 `folders`，遍历每个目录，对比文件的 `mtime` 和 `size`，更新数据库。
- **扫描策略**：使用 `std::fs` 递归遍历，仅处理图片/视频扩展名。
- **变更检测**：如文件修改时间或大小变化，更新 `files` 记录，并通知前端（通过 Tauri 事件）刷新 UI。

### 5.3 后台文件监控
- 使用 `notify` 库监听所有已添加的目录。
- 收到变更事件后，**防抖延迟 3 秒**（`tokio::time::delay_for`），然后对该目录执行增量扫描。
- 监控在单独的 tokio 任务中运行，不影响 UI。

### 5.4 AI 搜索（Mock）
- 前端发送搜索词，后端接收后调用 `MockSkill`（硬编码关键词映射）。
- 执行 SQL 查询，返回文件路径列表给前端。
- 真实 AI 技能可后续通过插件或独立服务接入。

### 5.5 缩略图缓存
- 缩略图生成使用 `image` 库，存储于 `cache/thumbnails/`。
- 最大缓存大小：**500 MB**。当超出时，基于文件访问时间（`atime`）清理最旧文件。

### 5.6 资源控制
- 使用 `tokio` 的 `current_thread` 运行时，限制并发任务数（如扫描任务最多 2 个并发）。
- AI 搜索任务使用独立线程池，空闲时延迟执行。

---

## 6. 前后端通信（Tauri IPC）

- 前端通过 `invoke` 调用后端命令：
  - `scan_folders`：手动刷新所有目录
  - `add_folder(path)`：添加新目录
  - `remove_folder(id)`：移除目录
  - `search_files(query)`：执行搜索
  - `get_photos(folder_id?)`：获取照片列表
- 后端通过 `emit` 事件通知前端（如扫描进度、文件变更）。
- 所有数据传递使用 JSON 序列化。

---

## 7. 前端交互逻辑

- **页面加载**：调用 `get_photos(null)` 获取所有照片，渲染网格。
- **搜索框**：输入防抖 500ms，调用 `search_files`，更新网格。
- **菜单切换**：显示/隐藏对应视图。
- **添加目录**：使用 Tauri 的 `dialog` 模块选择文件夹，调用 `add_folder`，然后刷新列表。
- **目录列表**：每项显示路径和照片数，提供移除按钮。

---

## 8. 构建与打包

- 使用 Tauri 内置打包工具：
  ```bash
  npm run tauri build
  ```
- 输出：`.exe` (Windows), `.app` (macOS), `.deb/AppImage` (Linux)。
- 无需额外打包配置，Tauri 自动处理资源。

---

## 9. 部署与运行

- 用户只需下载单个安装包（约 5-10 MB）安装即可。
- 所有数据存储在用户数据目录（`~/Library/Application Support/` 或 `%APPDATA%`），不污染系统。

---

## 10. 未来扩展性

- **后端服务**：当前 Demo 无后端，但后端逻辑（扫描、AI）已封装成独立模块，未来可迁移为 Web 服务或 gRPC 服务。
- **AI 能力**：可通过替换 `MockSkill` 为真实模型调用（如 ONNX Runtime）或远程 API。
- **云同步**：可添加同步模块，但保持向后兼容。

---

## 11. 代码规范与约束

- **Rust**：遵循标准 Rust 编码规范，使用 `clippy`。
- **前端**：CSS 使用 BEM 命名，JS 使用 ES6 模块。
- **禁止项**：前端禁用 CSS 动画、过渡、滤镜；后端禁用不安全代码（`unsafe`）。
- **日志**：使用 `log` 和 `env_logger` 输出关键操作。

---

**结束语**：本方案利用 Tauri 实现轻量化跨平台桌面应用。请依据此方案开始开发，优先实现数据库与扫描模块，再构建前端界面。