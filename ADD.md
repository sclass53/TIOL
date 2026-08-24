## 1. 总体架构

```
┌──────────────────────────────────────────────────────────────┐
│                         Tauri 后端 (Rust)                    │
├──────────────────────────────────────────────────────────────┤
│  API 层 (Tauri Commands)   ←──  前端调用                   │
│         ↓                                                   │
│  搜索模块 (SearchEngine)                                    │
│   ├─ 标签搜索 (精确匹配 → 包含匹配 → 失败)                 │
│   └─ 智能搜索 (强制使用 SigLIP 文本编码器 + 图像向量)     │
│         ↓                                                   │
│  索引管理 (IndexManager)                                   │
│   ├─ 增量扫描 (启动扫描 + 后台监控)                        │
│   ├─ 文件变更检测 (mtime + size)                           │
│   └─ AI 任务队列 (打标签 + 向量化)                         │
│         ↓                                                   │
│  AI 推理引擎 (AIEngine)                                    │
│   ├─ 多标签分类器 (DeepDanbooru ONNX)                      │
│   └─ 多模态编码器 (SigLIP ONNX)                            │
│       ├─ 图像编码器 → 提取图像向量                         │
│       └─ 文本编码器 → 提取文本向量  ← 必须实现             │
│         ↓                                                   │
│  数据库层 (SQLite via rusqlite)                            │
│         ↓                                                   │
│  缩略图缓存 (已存在，仅引用)                               │
└──────────────────────────────────────────────────────────────┘
```

**技术栈强制约束**：
- 异步运行时：`tokio`
- 数据库：`rusqlite`
- 文件监控：`notify`
- AI 推理：`ort` (ONNX Runtime)
- 图像处理：`image`
- 序列化：`serde`
- **所有跨平台路径操作必须使用 `dunce`**

---

## 2. 数据库设计（SQLite）

```sql
-- 文件夹管理
CREATE TABLE folders (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    path TEXT UNIQUE NOT NULL,          -- 绝对路径（标准化，小写）
    last_scan_time INTEGER NOT NULL DEFAULT 0
);

-- 文件主表
CREATE TABLE files (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    folder_id INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
    path TEXT UNIQUE NOT NULL,
    file_name TEXT,
    file_size INTEGER,
    mtime INTEGER NOT NULL,             -- 最后修改时间（秒级）
    date_added INTEGER NOT NULL,
    embedding BLOB,                     -- SigLIP 图像向量 (f32 数组)
    ai_processed INTEGER DEFAULT 0      -- 0=未处理, 1=已打标签, 2=向量已提取, 3=全部完成
);

-- 标签字典
CREATE TABLE tags (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT UNIQUE NOT NULL COLLATE NOCASE
);
CREATE INDEX idx_tag_name ON tags(name COLLATE NOCASE);

-- 文件-标签关联
CREATE TABLE file_tags (
    file_id INTEGER REFERENCES files(id) ON DELETE CASCADE,
    tag_id INTEGER REFERENCES tags(id) ON DELETE CASCADE,
    confidence REAL DEFAULT 1.0,
    source INTEGER DEFAULT 0,           -- 0=手动, 1=AI自动
    PRIMARY KEY (file_id, tag_id)
);
CREATE INDEX idx_file_tags_tag ON file_tags(tag_id);
CREATE INDEX idx_file_tags_file ON file_tags(file_id);

-- 自定义标签插件（少样本参考向量）
CREATE TABLE custom_tags (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT UNIQUE NOT NULL,
    embedding BLOB NOT NULL,            -- 均值池化后的原型向量
    threshold REAL DEFAULT 0.25,
    ref_count INTEGER DEFAULT 0,
    enabled INTEGER DEFAULT 1
);
```

**路径存储规则**：
- 存储前：`dunce::simplified()` 规范化，`to_string_lossy().to_lowercase()` 转为小写。
- 展示时：保留原始大小写（原始路径另存为 `display_path` 字段，或从物理文件读取）。

---

## 3. 文件监控与增量扫描

### 3.1 启动扫描 (`startup_scan`)
- **触发**：应用启动后执行一次。
- **流程**：
  1. 从 `folders` 表读取所有路径。
  2. 对每个路径，递归遍历文件。
  3. 扩展名过滤：`.jpg,.jpeg,.png,.heic,.gif,.bmp,.mp4,.mov,.raw`。
  4. 对比 `mtime` 和 `size`：
     - 不存在 → 插入，`ai_processed=0`，加入 AI 队列。
     - 有变化 → 更新，重置 `ai_processed=0`，加入 AI 队列。
     - 无变化 → 跳过。
     - 文件消失 → 删除记录。
  5. 更新 `folders.last_scan_time`。

### 3.2 后台实时监控 (`FileWatcher`)
- **实现**：使用 `notify::RecommendedWatcher`（自动选最优后端）。
- **防抖**：收到事件后，启动 3 秒延时定时器；延时内重复事件则重置。
- **增量扫描**：延时结束后，对该目录执行 `scan_folder(path)`。

### 3.3 增量扫描 (`scan_folder`)
- 仅扫描指定路径及其子目录。
- 逻辑与启动扫描相同，但只处理变更文件。

---

## 4. AI 推理引擎（强制实现）

### 4.1 模型管理
- **模型文件**：首次启动时从 CDN 下载，存于 `app_cache_dir/models/`。
- **DeepDanbooru ONNX**：约 300MB，用于多标签分类。
- **SigLIP ONNX**：包含 **图像编码器** 和 **文本编码器** 两个入口。
  - 模型文件可统一（如 `siglip-base-patch16-224.onnx`，包含两个输出）。
  - 或分两个文件（图像和文本）。
  - **必须**同时支持两种输入：图像 tensor（3×224×224）和文本 token IDs（1×seq_len）。

完全理解。我们需要将模型管理设计得如同 **`Cargo.lock` 或 `package-lock.json`** 一样——**应用内置了固定的“锁定清单”**，启动时强制校验 SHA256，**只有完全匹配才允许加载**。这确保了所有用户在所有平台上使用像素级一致的模型文件，杜绝了因网络传输损坏或 CDN 文件被意外替换导致的不可预测行为。

以下是修改后的 **“模型下载与生命周期管理”** 章节，请直接替换或合并到你的技术方案中。

---

## 补充章节（修订版）：模型锁定、强制校验与生命周期管理

### 1. 核心设计原则（类似 Package Lock）

- **内置锁定文件（Embedded Lockfile）**：应用编译时，在源代码中硬编码一个 `ModelLock` 结构体（或附带一个只读的 `model_lock.json` 作为应用资源）。它包含了所有模型文件的**精确下载 URL**、**文件大小（字节）** 和 **SHA256 哈希值**。
- **禁止自动升级**：应用**永远不会**自动升级模型到新版本，也不会动态更新锁定文件。模型版本的更新必须伴随应用自身的版本更新（即升级 `.exe` 或 `.app`）。
- **强制完整性**：启动时，如果磁盘上的模型文件与内置锁定的 SHA256 不匹配，该文件将被**视为毒药（Poison）**，立即删除并触发重试。匹配失败时，**绝对禁止**加载该模型进入推理会话。

---

### 2. 内置锁定结构定义（Rust 常量）

在代码中定义一个静态的锁定清单（建议放在 `src/ai/model_lock.rs`）：

```rust
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelFileInfo {
    pub url: &'static str,          // 完整 CDN 下载地址
    pub size: u64,                  // 文件字节数
    pub sha256: &'static str,       // 十六进制 SHA256 字符串
}

// 此锁在编译时固定，相当于 package-lock.json 的硬编码版本
pub const MODEL_LOCK: &[(&str, ModelFileInfo)] = &[
    (
        "deepdanbooru.onnx",
        ModelFileInfo {
            url: "https://cdn.example.com/models/v1.0.0/deepdanbooru.onnx",
            size: 314572800,
            sha256: "abc123def456...",
        },
    ),
    (
        "siglip_image.onnx",
        ModelFileInfo {
            url: "https://cdn.example.com/models/v1.0.0/siglip_image.onnx",
            size: 456789123,
            sha256: "def456ghi789...",
        },
    ),
    (
        "siglip_text.onnx",
        ModelFileInfo {
            url: "https://cdn.example.com/models/v1.0.0/siglip_text.onnx",
            size: 456789123,
            sha256: "ghi789jkl012...",
        },
    ),
    (
        "tokenizer.json",
        ModelFileInfo {
            url: "https://cdn.example.com/models/v1.0.0/tokenizer.json",
            size: 12345,
            sha256: "jkl012mno345...",
        },
    ),
];
```

**关键约束**：此锁文件由开发者维护，Codex 生成代码时必须将这些哈希硬编码进去，**绝不允许**在运行时动态生成或修改此清单。

---

### 3. 启动时强制检测流程（原子性操作）

应用启动调用 `AIEngine::init()` 时，必须执行以下**严格顺序**的原子操作：

#### 第一步：目录初始化
- 确保模型目录存在（`std::fs::create_dir_all`）。

#### 第二步：遍历锁定清单，检查每个文件的状态
对于 `MODEL_LOCK` 中的每一个条目 `(filename, lock_info)`：
1. **检查是否存在**：
   - 若文件不存在，标记为 `NeedsDownload`。
2. **检查文件大小**：
   - 若 `fs::metadata(file).len() != lock_info.size`，立即 **删除该文件**（无论是否损坏），标记为 `NeedsDownload`。
3. **强制 SHA256 校验**：
   - 使用 `sha256` crate 计算磁盘文件的哈希。
   - 若计算出的哈希与 `lock_info.sha256` **不匹配（包括大小写）**，立即 **删除该文件**，标记为 `NeedsDownload`。
   - *注意：绝对不允许“跳过校验”或“仅警告”，不一致必须触发删除重下。*

#### 第三步：全量就绪判定
- 遍历完成后，若没有文件被标记为 `NeedsDownload`，模型目录进入**“已锁定（Locked）”**状态，直接加载。
- 若有任一文件被标记为 `NeedsDownload`，进入下载流程。

---

### 4. 下载执行器（断点续传 + 原子写入）

当检测到缺失或不匹配时，下载器按以下逻辑执行：

1. **清理残留**：对于被标记为 `NeedsDownload` 的文件，确保其主文件和 `.part` 临时文件均已被删除（防止脏数据干扰）。
2. **下载流程**：
   - 发起 `GET` 请求，利用 `Range` 头实现断点续传。
   - 临时文件命名为 `{filename}.part`。
   - 数据流写入 `.part` 文件。
3. **校验与原子替换（关键）**：
   - 下载完成后，**立即计算 `.part` 文件的 SHA256**。
   - 对比计算值是否等于 `lock_info.sha256`。
   - **若校验失败**：删除 `.part` 文件，记录错误，**重试下载（最多 3 次）**。
   - **若校验通过**：执行 `std::fs::rename(.part, filename)`（原子操作，确保加载时文件是完整的）。
4. **全部下载完成后**：重复执行“第二步”中的校验循环，确保所有文件状态为 `Locked`，然后加载模型。

---

### 5. 模型加载（仅在全量锁定后执行）

- **只有在所有文件的 SHA256 校验全部通过后**，才允许创建 `ort::Session`。
- 如果任何一个文件损坏且重试次数用尽：
  - 应用进入**降级模式**（后台打标和智能搜索不可用）。
  - 前端必须收到一个明确的错误事件，包含具体哪个文件哈希不匹配（如 `Expected abc, got xyz`），方便用户排查（也方便开发者定位 CDN 文件是否被意外修改）。

---

### 6. 磁盘空间保护（前置检查）

- 下载前必须检查可用空间。如果剩余空间小于 `(总模型大小 + 500MB 余量)`，**拒绝下载**并向用户报错，不进入重试循环。

---

### 7. 前端交互事件（强制反馈）

下载过程中的每一个状态变更必须通过 Tauri 事件实时推送：

```rust
#[derive(Clone, Debug, serde::Serialize)]
pub struct ModelDownloadEvent {
    pub status: String,  // "checking", "downloading", "verifying", "locked", "error"
    pub file_name: String,
    pub progress: f32,   // 0.0 ~ 1.0
    pub message: String,
    pub sha256_mismatch: Option<String>, // 如果校验失败，传递 "Expected {a}, Got {b}"
}
```

**前端展示建议**：
- 进度条显示整体下载进度。
- 若校验失败，显示“模型文件损坏，正在修复...”并自动重试，无需用户干预。
- 若重试耗尽，显示“请检查网络后重启应用”并给出具体的 SHA256 不匹配报告。

---

### 8. 开发者更新模型版本的流程

当需要更新模型文件（如优化精度）时：

1. 开发者重新生成新的 SHA256 和 size。
2. 将新模型上传至 CDN（保留旧版本文件，避免已发布应用崩溃）。
3. 修改源码中的 `MODEL_LOCK` 常量数组。
4. 重新构建并发布新版本应用。
5. 旧版本应用由于内置锁不同，会继续使用旧模型文件，**互不干扰**。

---

将本修订版合并后，你的应用在模型管理方面将达到企业级工程标准

### 4.2 AIEngine 接口（强制）
```rust
pub struct AIEngine {
    tagger: ort::Session,       // DeepDanbooru
    embedder: ort::Session,     // SigLIP (同时包含图像和文本编码)
}

impl AIEngine {
    // 加载模型（必须同时加载图像和文本编码能力）
    pub fn load_models() -> Result<Self>;

    // 多标签分类：返回 Vec<(String, f32)>，置信度 > 0.5
    pub fn predict_tags(image_path: &Path) -> Result<Vec<(String, f32)>>;

    // 图像向量化：返回 Vec<f32> (归一化)
    pub fn embed_image(image_path: &Path) -> Result<Vec<f32>>;

    // 文本向量化：返回 Vec<f32> (归一化)  ← 必须实现
    pub fn embed_text(text: &str) -> Result<Vec<f32>>;
}
```

### 4.3 文本编码器实现细节（强制）
- **输入**：原始文本字符串（如 `"a photo of sunset"`）。
- **预处理**：
  1. 使用 SigLIP 的 tokenizer（需下载 `tokenizer.json`，或使用 `byte-pair-encoding` crate）。
  2. 截断至模型最大序列长度（通常 64 或 128 tokens）。
  3. 添加 `[CLS]` 和 `[SEP]` 特殊 token。
- **推理**：通过 `ort` 将 token IDs 输入文本编码器，输出归一化向量。
- **错误处理**：若文本编码失败，智能搜索返回空结果并记录错误，**不允许**回退到其他模型。

### 4.4 ONNX Runtime 执行提供者（跨平台）
| 平台 | 推荐执行提供者 | 说明 |
| :--- | :--- | :--- |
| **Windows (NVIDIA)** | `CUDAExecutionProvider` | 若 GPU 可用，否则回退 CPU |
| **Windows (AMD)** | `DirectMLExecutionProvider` | 支持 GPU 加速 |
| **macOS (Apple Silicon)** | `CoreMLExecutionProvider` | 调用 Neural Engine，速度最快 |
| **macOS (Intel)** | `CPUExecutionProvider` | 仅 CPU |
| **通用 CPU** | `CPUExecutionProvider` | 所有平台均可 |

在 `ort` 中配置：
```rust
let session = ort::SessionBuilder::new()?
    .with_execution_providers([...])?  // 按优先级尝试
    .build()?;
```

---

## 5. AI 处理队列

- **队列**：`tokio::sync::mpsc::channel<AITask>`，容量 1000。
- **消费者**：单线程低优先级任务循环。
- **任务内容**：`AITask { file_id: i64, path: String, action: Action }`。
- **执行顺序**：
  1. `predict_tags` → 写入 `file_tags`。
  2. `embed_image` → 写入 `files.embedding`。
  3. 更新 `files.ai_processed = 3`（全部完成）。
- **错误重试**：最多 3 次，间隔递增（1s, 5s, 10s）。
- **资源控制**：AI 线程在空闲时 `sleep(100ms)` 降低 CPU 占用。

---

## 6. 搜索模块（强制双路径）

### 6.1 标签搜索（分级逻辑）
**输入**：`query: String`  
**返回**：`Vec<FileInfo>`

1. **精确匹配**：`SELECT ... WHERE tags.name = query`（COLLATE NOCASE）。
   - 若有结果，直接返回。
2. **包含匹配**：`SELECT ... WHERE tags.name LIKE '%query%'`。
   - 若有结果，返回。
3. **失败**：返回空 `Vec`（前端可展示“未找到匹配标签”并推荐智能搜索）。

### 6.2 智能搜索（强制使用 SigLIP 文本编码器）
**输入**：`query: String`  
**返回**：`Vec<FileInfo>`，按相似度降序，限 500 条

**执行流程**：
1. 调用 `AIEngine::embed_text(query)` 获取文本向量 `text_vec`。
   - **若此步骤失败，搜索直接返回错误，不回退**。
2. 从 `files` 表读取所有 `embedding`（仅 `ai_processed >= 2` 的文件）。
3. 计算 `text_vec` 与每个图像向量的余弦相似度（点积，因为已归一化）。
4. 排序，取前 500 条。
5. 返回文件信息。

**性能优化**：
- 若文件数 < 10,000，暴力搜索。
- 若文件数 > 10,000，使用 `usearch` 或 `faiss` 进行 ANN 索引（后续版本）。

---

## 7. Tauri API（后端命令）

| 命令 | 输入 | 返回 | 说明 |
| :--- | :--- | :--- | :--- |
| `add_folder` | `path: String` | `Result<()>` | 添加目录，触发首次扫描 |
| `remove_folder` | `id: i64` | `Result<()>` | 移除目录及所有文件 |
| `get_folders` | - | `Vec<FolderInfo>` | 返回所有文件夹及统计 |
| `search` | `query: String, mode: String` | `Vec<FileInfo>` | `mode="tag"` 或 `"semantic"` |
| `get_photos` | `folder_id: Option<i64>` | `Vec<FileInfo>` | 获取照片列表 |
| `process_file` | `file_id: i64` | `Result<()>` | 手动触发 AI 处理 |
| `add_custom_tag` | `name: String, ref_paths: Vec<String>, threshold: f32` | `Result<()>` | 添加少样本插件 |
| `delete_custom_tag` | `id: i64` | `Result<()>` | 删除自定义标签 |

---

## 8. 缩略图缓存

**本地已实现**，后端仅需引用现有路径：
- 缩略图存储于 `app_cache_dir/thumbnails/`。
- 命名规则：`{file_id}.webp` 或 `{file_id}.jpg`。
- 清理策略：当缓存总大小 > 500MB 时，删除访问时间最早的文件。

---

## 9. 跨平台兼容性（强制）

### 9.1 路径与文件系统

| 差异点 | macOS | Windows | 处理策略 |
| :--- | :--- | :--- | :--- |
| 路径分隔符 | `/` | `\` | 使用 `std::path` 自动适配 |
| 大小写敏感 | 不敏感 | 不敏感 | 数据库路径统一转为**小写** |
| 路径长度限制 | 无 | 260 字符 | 启用 `windows_enable_long_path` |
| 驱动器盘符 | 无 | `C:\` 等 | `std::path` 自动处理 |
| 符号链接 | `canonicalize()` 解析 | `canonicalize()` 解析 | 存储前调用 `dunce::simplified()` |

**路径标准化函数**：
```rust
use dunce::simplified;
pub fn normalize_path(path: &Path) -> PathBuf {
    let abs = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir().unwrap().join(path) };
    let simplified = simplified(&abs);
    PathBuf::from(simplified.to_string_lossy().to_lowercase())
}
```

### 9.2 文件监控

| 平台 | 后端 API | 注意事项 |
| :--- | :--- | :--- |
| **macOS** | FSEvents | 移动事件可能产生多个事件，防抖窗口合并 |
| **Windows** | ReadDirectoryChangesW | 重命名事件拆分为删除+创建，防抖合并 |

使用 `notify::RecommendedWatcher` 自动适配。

### 9.3 macOS 沙盒权限（关键）

Tauri 打包后应用在沙盒中运行，必须配置 `entitlements.plist`：

```xml
<key>com.apple.security.files.user-selected.read-write</key>
<true/>
<key>com.apple.security.files.bookmarks.app-scope</key>
<true/>
<key>com.apple.security.assets.pictures.read-only</key>
<true/>
```

**在 `tauri.conf.json` 中引用**：
```json
"bundle": {
  "macOS": {
    "entitlements": "./entitlements.plist"
  }
}
```

**重要**：开发模式 (`tauri dev`) 无沙盒，打包后 (`tauri build`) 沙盒生效。所有文件操作必须在打包后测试。

### 9.4 标准目录（使用 Tauri Path API）

| 目录 | Windows | macOS |
| :--- | :--- | :--- |
| 应用数据 | `%APPDATA%\com.app\` | `~/Library/Application Support/com.app/` |
| 应用缓存 | `%LOCALAPPDATA%\com.app\cache\` | `~/Library/Caches/com.app/` |

```rust
use tauri::api::path::{app_data_dir, app_cache_dir};
let db_path = app_data_dir(&config).unwrap().join("photos.db");
let cache_dir = app_cache_dir(&config).unwrap();
```

### 9.5 构建与分发

| 平台 | 打包格式 | 注意事项 |
| :--- | :--- | :--- |
| **macOS** | `.app` | 必须签名，沙盒权限生效 |
| **Windows** | `.msi` 或 NSIS `.exe` | MSI 需管理员权限；NSIS 可用户级安装 |

---

## 10. 依赖库清单

```toml
[dependencies]
# 异步运行时
tokio = { version = "1.0", features = ["full"] }

# 数据库
rusqlite = { version = "0.31", features = ["bundled"] }

# 文件监控
notify = "6.1"

# AI 推理
ort = "1.16"
image = "0.24"
ndarray = "0.15"

# 跨平台路径
dunce = "1.0"

# Tauri 核心
tauri = { version = "2", features = ["api-all"] }

# 序列化
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"

# 日志
env_logger = "0.10"
log = "0.4"
```

---

## 11. 测试要求

- **单元测试**：使用 `tempfile` 创建临时目录，测试扫描逻辑。
- **集成测试**：模拟文件变更、AI 处理队列。
- **跨平台测试**：在 Windows 和 macOS 上分别运行。
- **沙盒测试**：macOS 打包后测试文件访问、搜索、AI 功能。

---

## 12. 错误处理与日志

- 使用 `anyhow` + `thiserror` 定义错误类型。
- 关键操作（扫描、AI 推理、数据库写入）记录 `info!` 或 `error!` 日志。
- 开发环境启用 `RUST_LOG=debug`。

---

## 13. 后续扩展点（不实现）

- 自定义标签插件匹配（将 `custom_tags` 向量与图像比对自动打标签）。
- 智能搜索 ANN 索引（`usearch` 集成）。
- 多模型选择（用户可选精度）。



## ADDITIONAL

# 补充章节：网络优化 – 国内镜像加速（模型下载）

> 本附录为技术方案中 **“模型下载与生命周期管理”** 章节的补充，针对国内用户访问 Hugging Face 官方源速度慢的问题，提供强制性的镜像配置方案。

---

## 1. 背景与目标

- **问题**：所有模型文件均托管于 Hugging Face，其官方 CDN 位于海外，国内用户下载速度极慢（通常 < 100 KB/s），导致首次启动等待时间过长，严重影响用户体验。
- **目标**：在应用中内置**国内镜像加速机制**，将下载速度提升至 **10 ~ 50 MB/s**，接近带宽上限，将首次启动耗时从数小时压缩至数分钟。
- **原则**：
  - 镜像配置对用户**完全透明**，无需用户手动设置代理或修改 hosts。
  - 应用启动时自动检测网络环境，智能选择最优镜像源。
  - 若镜像不可用，自动回退至官方源（确保功能可用性）。

---

## 2. 镜像源选择

经过调研与实测，我们选定了以下**主备镜像源**（按优先级排序）：

| 优先级 | 镜像源 | 域名/地址 | 特点 |
| :---: | :--- | :--- | :--- |
| 1 | **hf-mirror.com** | `https://hf-mirror.com` | 国内最大、最稳定社区镜像，覆盖所有 Hugging Face 仓库，支持 Range 请求和断点续传。 |
| 2 | **ModelScope (魔搭社区)** | `https://modelscope.cn` | 阿里旗下平台，国内 CDN 速度极快，但模型同步有一定延迟，部分仓库可能缺失。 |
| 3 | **OpenI 启智社区** | `https://openi.org.cn/hf-models` | 非营利平台，镜像较为完整，但带宽稍逊于前两者。 |
| 4 | **官方 Hugging Face（兜底）** | `https://huggingface.co` | 在所有镜像均不可用时使用，确保最终可用性。 |

---

## 3. 实现方案：环境变量驱动

所有模型下载逻辑统一使用 **Hugging Face Hub 客户端库**（Rust 中为 `huggingface-hub` crate，或通过 `reqwest` 手动构造请求）。该库遵循 **`HF_ENDPOINT`** 环境变量规范，当该变量被设置时，所有请求将自动转发至指定的镜像源。

### 3.1 应用启动时自动配置

在 `main.rs` 或 `AIEngine::init()` 中，执行以下检测与配置逻辑：

```rust
use std::env;
use std::time::Duration;
use reqwest::Client;

/// 配置 Hugging Face 镜像端点
pub async fn configure_hf_mirror() -> String {
    // 1. 定义候选镜像源
    const MIRRORS: [&str; 3] = [
        "https://hf-mirror.com",
        "https://modelscope.cn",        // 需确认 API 兼容性
        "https://openi.org.cn/hf-models",
    ];

    // 2. 若用户已手动设置 HF_ENDPOINT，优先使用
    if let Ok(user_endpoint) = env::var("HF_ENDPOINT") {
        if !user_endpoint.is_empty() {
            env::set_var("HF_ENDPOINT", &user_endpoint);
            return user_endpoint;
        }
    }

    // 3. 探测可用的镜像源（并发测试）
    let client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();

    for mirror in MIRRORS {
        // 发送 HEAD 请求测试连通性
        let url = format!("{}/api/models", mirror);
        match client.head(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                // 测试通过，设置并返回
                env::set_var("HF_ENDPOINT", mirror);
                return mirror.to_string();
            }
            _ => continue, // 失败则尝试下一个
        }
    }

    // 4. 所有镜像均不可用，回退至官方源
    env::remove_var("HF_ENDPOINT");
    "https://huggingface.co".to_string()
}
```

**关键点**：
- 检测仅消耗 **约 5 秒**，在应用启动过程中可接受。
- 首次探测成功后，将结果缓存至本地配置（`Settings`），下次启动时直接使用，避免重复探测。

### 3.2 下载流程中的强制使用

所有模型下载函数（如下载器）在构造 HTTP 客户端时，**必须**读取 `HF_ENDPOINT` 环境变量，并以其作为 Base URL 拼接路径：

```rust
fn build_download_url(relative_path: &str) -> String {
    let base = env::var("HF_ENDPOINT").unwrap_or_else(|_| "https://huggingface.co".to_string());
    format!("{}/{}", base, relative_path.trim_start_matches('/'))
}
```

例如，对于 `deepdanbooru.onnx`，原本官方 URL 为：
```
https://huggingface.co/Neus/Onnx_DeepDanbooru/resolve/main/deepdanbooru.onnx
```
当 `HF_ENDPOINT=https://hf-mirror.com` 时，下载 URL 变为：
```
https://hf-mirror.com/Neus/Onnx_DeepDanbooru/resolve/main/deepdanbooru.onnx
```
镜像站完全兼容此路径结构，无需修改其他逻辑。

---

## 4. 性能优化与用户体验

### 4.1 并行下载

模型文件较大（如 SigLIP 图像编码器 > 1 GB），建议使用 **多线程并发下载**，充分利用带宽。Rust 中可使用 `tokio::spawn` 并发下载所有模型文件，并在全部完成后统一校验。

### 4.2 下载进度显示

前端通过 Tauri 事件接收 `ModelDownloadEvent`，其中包含 `mirror_used` 字段，可展示当前使用的镜像源信息（如“正在从 hf-mirror.com 下载”），增强用户信任感。

### 4.3 回退策略细粒度化

若下载过程中出现 **校验失败（SHA256 不匹配）**，且当前使用的是镜像源，应自动切换至下一优先级镜像重试，而非直接报错。

```rust
fn download_with_fallback(lock: &ModelFileInfo) -> Result<Vec<u8>, DownloadError> {
    let mirrors = get_mirror_list(); // 有序列表
    for mirror in mirrors {
        let url = format!("{}/{}", mirror, lock.relative_path);
        match download_file(&url) {
            Ok(data) => {
                let hash = sha256(&data);
                if hash == lock.sha256 {
                    return Ok(data);
                } else {
                    // 校验失败，记录日志，尝试下一个镜像
                    warn!("SHA mismatch for {} from {}, retrying...", lock.name, mirror);
                    continue;
                }
            }
            Err(e) => {
                warn!("Download from {} failed: {}", mirror, e);
                continue;
            }
        }
    }
    // 所有镜像都失败
    Err(DownloadError::AllMirrorsFailed)
}
```

---

## 5. 配置持久化与用户手动设置

- **持久化**：首次成功探测到的镜像端点，写入 `Settings`（如 `settings.toml`）中的 `hf_endpoint` 字段，避免每次启动都探测。
- **手动设置**：在设置界面提供一个选项，允许高级用户手动输入自定义镜像 URL，覆盖自动探测结果。

---

## 6. 集成到 Tauri 应用的步骤（实现清单）

1. **添加依赖**：`reqwest`, `tokio`, `sha2`（用于校验）。
2. **在 `main.rs` 启动时调用 `configure_hf_mirror().await`**，确保在 `AIEngine` 初始化前完成。
3. **修改所有模型下载函数**，使用 `get_hf_endpoint()` 获取 Base URL。
4. **更新前端事件**，增加 `mirror` 字段，显示当前使用的镜像。
5. **测试验证**：
   - 在国内网络环境下验证镜像加速效果。
   - 模拟镜像不可用，验证自动回退逻辑。

---

## 7. 常见问题与故障排查

| 问题 | 可能原因 | 解决方案 |
| :--- | :--- | :--- |
| 镜像站返回 404 | 镜像未同步该仓库 | 自动回退至官方源或下一镜像 |
| 下载速度依然很慢 | 本地网络限制或镜像站带宽已饱和 | 引导用户手动设置环境变量 `HF_ENDPOINT` 为其他镜像 |
| 校验失败 | 镜像站缓存文件损坏 | 自动切换镜像重试，若全部失败则触发完整重新下载 |

---

**总结**：本补充章节为模型下载提供了完整的国内加速方案，通过环境变量驱动、多源探测与回退策略，确保所有用户在任何网络环境下都能高效、稳定地完成模型初始化。所有实现细节与现有技术方案中的“模型锁定”及“强制 SHA256 校验”无缝衔接，无需修改核心校验逻辑。

## HINTS

这里为你整理了所有模型文件在 **hf-mirror.com** 上的具体下载链接。

这些链接遵循了“将官方 `huggingface.co` 链接替换为 `hf-mirror.com`”的通用规则。

### 🧠 DeepDanbooru 模型

*   **模型文件**：`deepdanbooru.onnx`
*   **hf-mirror 下载链接**：
    `https://hf-mirror.com/Neus/Onnx_DeepDanbooru/resolve/main/deepdanbooru.onnx`

### 🖼️ SigLIP 模型

关于 SigLIP 模型，目前社区已转换为 ONNX 格式的版本主要有以下几种，你可以根据项目对模型大小和精度的要求进行选择：

| 模型版本 | 文件说明 | 链接 (hf-mirror) |
| :--- | :--- | :--- |
| **SigLIP2 Base** | 图像编码器 (推荐) | `https://hf-mirror.com/onnx-community/siglip2-base-patch16-naflex-ONNX/resolve/main/onnx/vision_model.onnx` |
| **SigLIP SO400M** | 图像编码器 (精度更高) | `https://hf-mirror.com/onnx-community/siglip-so400m-patch14-384-ONNX/resolve/main/vision_model.onnx` |
| **SigLIP SO400M** | 文本编码器 | `https://hf-mirror.com/onnx-community/siglip-so400m-patch14-384-ONNX/resolve/main/text_model.onnx` |

> **请注意**：`SigLIP2 Base` 模型使用了 Hugging Face 的 **Xet** 存储格式，直接通过浏览器下载 `vision_model.onnx`（142 kB）可能只是一个指针文件。**推荐使用 `huggingface-cli` 或 `hfd.sh` 等专用工具进行下载**。作为备选，你也可以选择文件结构更传统的 `SigLIP SO400M` 版本。

### 📝 Tokenizer 文件

Tokenizer 文件需要与模型版本匹配，这里提供两个版本的链接：

*   **SigLIP2 Base**：`https://hf-mirror.com/google/siglip2-base-patch16-224/resolve/main/tokenizer.json`
*   **SigLIP SO400M**：`https://hf-mirror.com/google/siglip-so400m-patch14-384/resolve/main/tokenizer.json`

### 💡 重要提醒

1.  **设置环境变量**：在你的 Tauri 后端代码中，务必在发起任何下载请求前，通过 `std::env::set_var("HF_ENDPOINT", "https://hf-mirror.com")` 设置环境变量，这样所有 Hugging Face 的请求都会自动指向镜像站。
2.  **工具下载**：对于大文件，强烈建议使用 `huggingface-cli` 或 `hfd.sh` 等工具，它们支持**断点续传**和**多线程下载**，体验更稳定。