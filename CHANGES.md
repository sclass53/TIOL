# TIOL — 变更记录 (CHANGES)

> 记录影响行为的关键改动与修复，供后续开发参考。环境注意事项见 BUILD.md / LIMITS.md / ADD.md。
> 改动编号规则：**C-NN**，按时间倒序（最新在最上）；引用改动时直接写编号。

## C-17 · 2025-08 — 照片星级评分 + 按评分筛选

**需求**：① 在每张照片预览图下方加一个打星界面，可打 1–5 星——未打星显示白色边框，打上后黄色填充；② 用下拉菜单筛选照片，只显示星值 ≥ 某阈值的照片。

**实现**：

1. **数据存储**：`files` 表新增 `rating INTEGER`（NULL = 未评分；migrate 自动加列，新建库 schema 同步）。`upsert_file` 的 `ON CONFLICT ... DO UPDATE` **不触碰 rating**——重新扫描/内容变更不会清掉用户评分（测试断言此行为）。`FileRecord` 新增 `rating: Option<i64>`（serde skip None）；`FILE_COLS/FILE_COLS_F` 追加为第 13 列——所有照片查询、文件名/语义/标签搜索自动携带。
2. **后端命令** `set_rating(file_id, rating)`：0–5 校验（0 = 清除，写 NULL），返回更新后的 `FileRecord` 供卡片原地刷新，无需整列重渲染。
3. **卡片打星 UI**：`buildCard` 在缩略图与文件名之间插入 `.card__stars` 行（5 颗星，22px）。未评分星星为细白描边（`-webkit-text-stroke`，跟随主题色 `--text-primary`），评分后黄色填充（`#ffcc00`）；悬停时预览填充到光标所在星。点击第 N 颗星评 N 分，**再次点击当前分值清除**；点击 stopPropagation 不触发预览，多选拖拽区域排除星标行。
4. **筛选下拉菜单**：搜索栏新增 `<select id="rating-filter">`（全部 / ≥1 / ≥2 / ≥3 / ≥4 / ≥5 星），选项走 i18n（`photos.ratingAll`…`photos.rating5`）。前端 `minRating` 状态参与 `applyFilters`（与颜色 ∩ 镜头 ∩ 焦段同为交集；未评分的照片视为 0 分，过不了 ≥1 的筛选）；`hasActiveFilters` 同步纳入，空结果显示「没有符合筛选条件的照片」；筛选面板「清除」按钮同时重置评分下拉。
5. **单测** `rating_roundtrip`：默认无评分 → 1–5 设置回读 → 0 清除 → 重扫（upsert）不丢评分。

**行为对照**：卡片预览图下方点 3 星 → 前 3 颗星黄色填充 ✓；再点第 3 颗 → 清除 ✓；下拉选「≥ 3 星」→ 只显示评了 ≥3 分的照片 ✓；搜索/其他筛选与评分叠加为交集 ✓。

## C-16 · 2025-07 — PR 合并前检查 workflow

**需求**：有 Pull Request（合并前）时运行全面检查：① 双语支持；② `.github/workflows` 不得被 PR 修改；③ 可以编译/构建。

**实现**：

1. **新 workflow `.github/workflows/pr-check.yml`**（`on: pull_request`，两个 job）：
   - `checks`（ubuntu-latest）：① `git diff base...HEAD -- .github/` 检测 PR 是否改动 workflow 目录——有改动直接报错（发布流水线神圣不可侵犯）；② `node scripts/check-i18n.js`；③ 前端 ESM 语法检查（`node --input-type=module --check`）。
   - `build-check`（windows-latest）：`cargo check`（发布平台编译验证，`dtolnay/rust-toolchain@stable`）。
2. **新校验脚本 `scripts/check-i18n.js`**（本地/CI 通用）：① en-US 与 zh-CN **深展平键集完全一致**（缺失即报错）；② app.js/index.html 引用的全部 i18n 键必须存在（排除 DOM 标签误匹配）；③ `messages.js` 与 JSON 源**逐字节同步**（不同步提示重跑 gen-messages.js）；任一失败 exit 1。已做破坏性测试（删键 → 捕获缺失 + 同步错误，恢复后通过）。
3. 发布构建仍只在 `build.yml`（tag 触发），本文件只守卫合并。

## C-15 · 2025-07 — EXIF 镜头/焦距：预览显示 + 镜头/焦段筛选

**需求**：① 读取照片 EXIF 镜头/焦距，不在卡片上直接显示，预览打开后在右侧窗口显示；② 筛选按钮面板增加"镜头"选项（勾选特定镜头）与"焦段"选项（min–max 范围），与颜色筛选互为**交集**；③ 筛选条件要求了某条数据而照片 EXIF 没有该条时，该照片不进入结果。

**实现**：

1. **EXIF 提取**（新模块 `exif.rs`，kamadak-exif 0.5）：读 `LensModel`(0xA434) 与 `FocalLength`(0x920A, mm)，失败/无 EXIF 返回 None（永不报错）；`display_value()` 的 ASCII 引号已剥离。提取时机：scanner 对**新增/变更文件**即时提取；启动后台**回填** `exif_checked=0` 的存量行（分页 500，读完即标记，无 EXIF 也只跑一次）。files 表新增 `lens TEXT / focal_length REAL / exif_checked INTEGER`（migrate 自动加列，upsert 内容变化时重置 exif_checked）。
2. **数据链路**：`FileRecord` 新增 `lens/focal_length`（serde skip None）；`FILE_COLS/FILE_COLS_F` 增加两列——所有照片查询/搜索自动携带。
3. **预览显示**：预览面板 meta 区改为多行——文件名·大小 / `镜头：xxx` / `焦距：50 mm`（有则显示，无则不显示；焦距整数不带小数）。
4. **筛选面板扩展**（与颜色筛选同一面板，全部**交集**）：
   - **镜头**：勾选式列表（`get_lens_list` 去重排序，会话内缓存），多选 = 并集；激活时照片 lens 必须命中其一，**无镜头数据的照片被排除**（C-15 规则）。
   - **焦段**：min/max 数字输入（mm，可只填一边，留空 = 不限）；激活时照片 `focal_length` 必须在范围内，**无焦距数据被排除**。
   - 颜色 ∩ 镜头 ∩ 焦段 三层叠加；「清除」一键清空全部；筛选按钮高亮反映任一激活。
5. **构建环境变更**：新增 crate 依赖在 E: 盘（**exFAT**）解包设置文件时间戳失败（cargo/bsdtar 的 SetFileTime 兼容问题，`os error 87`）→ `CARGO_HOME` 迁至 **`C:\Users\david\.cargo-tiol`**（1.2GB，robocopy 迁移，registry 源码保留）。今后所有 cargo 命令使用新 CARGO_HOME；E 盘旧目录已清空。
6. **单测**：`exif.rs` 手工构造含 LensModel/FocalLength 的 EXIF JPEG 验证解析（`reads_lens_and_focal_from_exif`）+ 无 EXIF 返回 None；db 层 `exif_columns_roundtrip`（回填队列/读写/镜头列表/变更重置），共 11 项全过。

**行为对照**：预览 → 右侧显示镜头与焦距 ✓；筛选面板勾镜头 A+B → 只显示 A/B 镜头照片（无镜头数据排除）✓；焦段 24–70 → 只显示焦距在区间且**有焦距数据**的照片 ✓；颜色+镜头+焦段同时筛选 → 交集 ✓。

## C-15.1 · 2025-07 — 多选删除标签 + 筛选/面板修复 + 窗口 1270

- **多选「删除标签」（红色按钮）**：selection-bar 新增 `.btn--danger` 红色「删除标签」；确认对话框后调用新命令 `clear_tags_from_files`（db `clear_all_tags_on_files`：删除这些文件的**全部文字标签（手动+AI）+ 颜色标签**），卡片原地刷新；完成提示文案从长句缩短为「已清除 N 张照片 / N photos cleared」（避免撑宽底栏按钮换行）。
- **镜头筛选修复**：匹配改为 `(p.lens || "").trim()` 比较（消除空白差异）；加诊断日志 `lens-filter`（筛选空结果且有镜头数据时输出实际字段值，供排查）；`"----"` 占位镜头（Sony 无镜头信息）在读取时即过滤 + `get_lens_list` 排除 + 预览不显示。
- **筛选面板布局修复**：面板固定宽度 240px；镜头列表改**块级 + 固定行高 28px + line-height 居中**（条目物理上不可能重叠），容器 160px + `min-height:0` + 内部滚动。
- **窗口宽度**：tauri.conf.json `1290 → 1270`。
- 单测 12 项全过。

## C-15.2 · 2025-07 — 设为壁纸（右键菜单）

- 右键卡片菜单新增「设为壁纸」：`set_wallpaper` 命令——**Windows** 用 `SystemParametersInfoW(SPI_SETDESKWALLPAPER)` 原始 FFI（零新依赖）；**macOS** 走 `osascript`（System Events 设置所有桌面，可能弹辅助功能授权）；**Linux** 用 `gsettings`（GNOME）。
- 成功显示底部 toast「壁纸已设置」（`.toast` 组件，2.2s 自动消失）；失败 alert 原因。
- i18n：`menu.wallpaper/wallpaperSet`（中英同步），`messages.js` 重新生成。

## C-14 · 2025-07 — 颜色标签（独立存储）+ 颜色筛选

**需求**（仿苹果相册）：① 多选模式底部 pill 中间放若干颜色圆点，点击即给选中照片应用颜色标签（一张图可多个颜色）；卡片文件名右侧显示对应颜色点；② 搜索框左侧加颜色筛选（多选、并集）。

**实现**：

1. **独立存储**（与文字标签完全分开）：新表 `color_tags(file_id, color)`，`PRIMARY KEY(file_id, color)` + `ON DELETE CASCADE`（files 删除自动清理）；`clear_all_tags`/`reindex_embeddings` 均不触碰颜色。颜色枚举：`red/orange/yellow/green/blue/purple`（前端 hex 映射：`#ff3b30/#ff9500/#ffcc00/#34c759/#0a84ff/#af52de`，后端校验白名单）。
2. **FileRecord.colors**：`FILE_COLS/FILE_COLS_F` 增加 colors 子查询（第 11 列），`map_file` 解析——所有照片查询/搜索自动携带颜色。
3. **toggle_color_tag**（db + 命令）：手机相册语义——**全部选中照片已有该色 → 全部移除；否则补到缺失的文件**（幂等），返回操作后是否全有。
4. **多选底栏颜色圆点**：`.selection-bar__dots`（count 与按钮之间，分隔线隔开）6 个 22px 圆点；点击调用 toggle 并**原地更新卡片颜色点**（不丢滚动）；点击反馈脉冲动画。
5. **卡片颜色点**：`renderCardColors` 在文件名右侧渲染 9px 圆点（多个颜色并排，title=颜色名）；`renderCardMeta` 同步支持。
6. **颜色筛选**：搜索栏最左 🎨 按钮 → 下拉面板（6 个 18px 圆点可多选 + 清除按钮）；**并集**——命中任一选中颜色即显示；筛选对全部照片/文件名/标签/语义搜索结果都生效（`allPhotos` 保存未过滤结果，`applyColorFilter` 统一过滤）；激活时按钮蓝色高亮；空结果显示"没有符合颜色筛选的照片"；点击外部收起。
7. **i18n**：新增 `colors.*`（6 色中英）、`photos.filterColor/filterClear/filterEmpty`，`messages.js` 重新生成。
8. **单测**：新增 `color_tags_toggle`（全有→移除/缺失→补全/多色共存/级联删除），共 9 项全过。

**行为对照**：进入多选 → 底栏点红点 → 选中照片卡片出现红点 ✓；再点一次 → 全部移除 ✓；一张图可红+蓝并存 ✓；🎨 选红+蓝 → 显示带任一色的照片（并集）✓；搜索词与颜色筛选叠加 ✓。


## C-13 · 2025-07 — 手机相册式多选 + 批量打标 + 标签编辑列表化

**需求**：① 照片视图右上角"多选"按钮，进入后每张卡片右上角出现多选框，支持鼠标拖拽框选（对全部照片/搜索结果同样可用）；② 选中照片可批量添加标签（从已有标签中选一个）；③ 卡片"编辑标签"改为列表式：显示当前标签 + 下方可选已有标签添加。

**实现**：

1. **多选模式**（`app.js`）：搜索栏右侧「多选」按钮切换；卡片右上角圆点多选框（`.card__check`），选中卡片高亮（`.card--selected`）；**拖拽框选**——在网格任意位置按下拖动画出橡皮筋（`.selection-box`），松手选中相交卡片（实时悬停高亮），点按（无位移）回落为单卡切换；多选模式下点击卡片切换选中而非打开预览。
2. **底部选中条**（`.selection-bar`，fixed 底部居中）：实时显示"已选 N 张"；「添加标签」→ 弹出标签选择面板（`get_all_tags` 列表，按使用次数排序，排除 unknown），点一个标签即批量追加；「取消」退出。操作完成显示短暂提示（"已为 N 张照片添加「tag」"）并刷新卡片标签，**选中保留**可继续操作。
3. **退出多选逻辑**（隐藏逻辑补齐）：再次点按钮/「取消」/按 **Esc**（依次关闭标签面板→编辑框→预览→退出多选）/切换到其他页签，均退出并清空选中。
4. **批量打标后端**：`add_manual_tags_batch`（db）+ `add_tags_to_files` 命令——把给定标签作为**手动标签**（source=0）追加到每个选中文件，不动已有标签（幂等）；`get_all_tag_names`（db）+ `get_all_tags` 命令——tags 表全部名字按使用次数降序，排除 unknown 哨兵。
5. **编辑标签对话框列表化**：上部「当前标签」chips——手动标签带 × 可删、AI 标签只读展示；中部「从已有标签添加」列表（点击即加入当前集，已添加的自动隐藏）；底部输入框回车添加新标签（Enter=添加，不再是保存）；保存时一次性 `update_tags` 应用全部手动标签。
6. **i18n**：新增 `photos.selectMode/selectDone/selectedCount/addTagSelected/tagsAdded`、`card.edit.current/suggest/noTags/noSuggest/remove`（占位文案改为"输入新标签名称后按回车"）、`tags.pickTitle/pickEmpty`（中英同步），`messages.js` 重新生成。
7. **单测**：新增 `manual_tags_batch_and_name_list`（批量追加/幂等/名字列表排序/unknown 排除）。

**行为对照**：点「多选」→ 卡片出多选框，拖拽框选/点击切换 ✓；底部条"已选 N 张 · 添加标签 · 取消" ✓；选标签 → 全部选中照片追加该标签（卡片标签即时刷新）✓；编辑标签 → 列表式增删 ✓；Esc/切页/取消均正确退出多选 ✓。


## C-12 · 2025-07 — 标签独立页签 + 手动打标（AI Tagging 按钮）

**需求**：① 标签功能从设置页移到独立"标签"页签（侧栏变 4 个：照片 / 目录 / 标签 / 设置）；② 打标改为**手动触发**——文件增删/变更只做索引（嵌入），添加/删除标签不再自动打标；只有点「AI 标记」按钮才按**全部当前标签**为照片打标（覆盖新增标签与新增/未标记文件）。

**实现**：

1. **任务类型化**（`queue.rs`）：`AITask` 新增 `kind: TaskKind`——`Index`（仅嵌入，不碰任何标签/unknown）与 `TagAll`（嵌入 + 全量标签匹配 + unknown 哨兵）。所有文件变更入队点（启动扫描 / watcher / add_folder / scan_folders / process_file）继续用 `AITask::new` → 自动变为纯索引；删除单标签任务（`with_tag`）与单标签分支。
2. **手动打标命令**（`main.rs`）：新增 `run_ai_tagging`——无标签定义时直接报错；否则分页查询 `get_files_missing_any_tag(5000, offset)`（**缺少任一当前自定义标签**的文件，含从未索引的新文件；手动标签与自定义标签同名视为已覆盖）逐页入队 `AITask::tag_all`；队列满（容量 1000）时 100ms 重试，杜绝一次点击丢任务；返回实际入队数供前端提示。
3. **移除自动打标链**：`retag_new_tag` / `AITask::with_tag` / `get_files_without_tag` / `add_custom_tag` 里的入队逻辑全部删除——加标签只重建向量缓存。C-10.3/10.5 的"新标签自动单标签检查"语义被本版本取代。
4. **浮窗文案按任务类型**：`AiProgress.tagging` 由"标签缓存非空"改为**最近处理任务的 kind**——索引任务显示"正在索引"，打标任务显示"正在标记中"。
5. **前端 4 页签**：新增 `view-tags`（工具条：⚡ AI 标记 + 清除标记（含确认）；标签添加行/列表/阈值/命中数；入队结果提示行 `#tagging-status`）；设置页删除全部标签区块与"清除标记"按钮；样式 `.settings__tag-*` 重命名为 `.tags__*`，新增 `.tags` 滚动容器。
6. **i18n**：新增 `nav.tags` 与 `tags.*` 命名空间（自 `settings.*` 迁移并改写 hint/empty 文案——"添加标签不会自动打标，点 AI 标记开始"），`messages.js` 重新生成。
7. **用户向文案去术语**：移除用户可见文本中的算法术语——"Zero-shot AI recognition, no training"（tags.hint）、"(natural language)"（搜索占位）、"Inference backend"→"AI 引擎"、"Inferring"→"Processing/处理中"；修正编辑对话框过时文案"AI 标签自动生成"→"AI 标签由「AI 标记」添加"（C-12 后打标已改手动）；README 同步。
8. **单测**：新增 `files_missing_any_tag_semantics`（单标签/全覆盖/多标签缺一/分页/手动同名覆盖），与既有 6 项一起通过。

**行为对照**：文件变更 → 自动索引（嵌入，不动标签）✓；加标签 → 只更新定义 ✓；删标签 → 立即从照片移除（原有）✓；点「AI 标记」→ 全量打标（命中写标签、全不中写 unknown，浮窗显示进度，完成后卡片/计数刷新）✓；无标签点按钮 → 提示"请先添加标签"。


## C-11 · 2025-07 — GitHub Actions 跨平台 CI 构建

**需求**：无需本地 Mac，通过 GitHub Actions 编译 macOS（及 Windows）发布版。

**实现**：

- `.github/workflows/build.yml`：双 job（`windows-latest` 出 msi/nsis + 便携 zip；`macos-14`（Apple Silicon）出 .app/dmg）。触发：push main（仅构建）、tag `v*`（草稿 Release）、手动 dispatch。
- 图标：新增 `app-icon.png`（1024×1024 占位图标，纯 Node 生成，无依赖），CI 里 `npx @tauri-apps/cli icon` 生成全套（含 macOS 必需的 icon.icns）。
- ONNX Runtime：Windows 用仓库内置 `vendor/onnxruntime/win-x64/onnxruntime.dll`；macOS 在 CI 下载官方 **universal2 含 CoreML EP** 的 `onnxruntime-osx-universal2-1.16.3.tgz`（版本+URL 在 workflow env 钉死，SHA256 待首次运行后补填 `ORT_MAC_SHA256`）。
- 产物：**安装包暂不内嵌 ORT 库**，功能性产物为便携包（Windows：exe+dll 压缩 zip；macOS：dylib 拷入 `.app/Contents/MacOS` 并 ad-hoc 签名）——运行时 dylib 分发（CHANGES.md C-10.6 方案 B 的打包变体）与安装器内嵌留待后续（需 `bundle.resources` + 启动时 `ORT_DYLIB_PATH` 探测）。
- macOS 签名/公证未配置（无 Apple 开发者证书 Secrets）；workflow 中已注释接入点（APPLE_* Secrets + tauri-action signing inputs）。

**注意事项**：① CI 网络畅通，crates 直连（`src-tauri/.cargo/config.toml` 本地代理已 gitignore，不影响 CI）；② `Cargo.lock` 已提交保证可复现；③ 首次运行后需把 onnxruntime tgz 的 SHA256 填入 `ORT_MAC_SHA256`（防供应链篡改）。

## C-11.1 · 2025-07 — 首启空标签行为确认 + 标签缓存竞态修复 + CI 兼容性核查

**① 无标签时不会打标（确认+防误解）**：首启/扫描时，`process_one` 在标签列表为空时**只做嵌入、跳过全部标签匹配**（`if !tagvecs.is_empty()` 分支），不产生任何标签尝试，也不写 unknown。新增一次性日志 `no user tags defined — embedding only, tagging skipped` 让行为可见；嵌入仍保留（语义搜索与后续打标都依赖它）。

**② 标签缓存竞态修复（真实 bug）**：启动时序为 引擎就绪(T0) → 标签缓存构建完成(T0+~0.5s)。在 T0~T0+0.5s 窗口内被处理的文件会在**空缓存**下完成（既不打标也不 unknown），且因 `ai_processed=3` 永不再查——与其余文件不一致。修复：新增 `cache_ready`（AtomicBool），`rebuild_tag_cache` 首次构建完成后置位，**消费者在首次构建完成前不处理任何任务**（上限约 120s，与引擎等待一致）。

**③ CI 兼容性核查结论**：ort 的 cuda/directml 模块是**按名字注册的 EP 派发器**（load-dynamic 设计），directml.rs 无任何平台 cfg、cuda.rs 的 windows cfg 仅涉及 DLL 预载路径 → macOS 编译兼容（coreml 特性在 Windows 构建已证反向成立）；`vendor/onnxruntime/win-x64/onnxruntime.dll` 与 `src-tauri/entitlements.plist` 已入库（CI 必需）；`src-tauri/.cargo/config.toml` 确认被 gitignore（CI 直连 crates.io）；**待提交**：`app-icon.png`、`.github/workflows/build.yml`（提交前提醒用户）。

## C-11.2 · 2025-07 — cpu 模式 ConvInteger 报错：系统内置 onnxruntime 劫持

**症状**：选择 cpu 后端 → `Model error — Could not find an implementation for ConvInteger(10)`（debug 构建正常，release 构建复现）。

**根因**：**Win11 24H2+ 系统内置 `C:\Windows\system32\onnxruntime.dll`**（最小版，CPU EP 无 ConvInteger 内核）。release exe 目录没有我们的 vendored DLL 时，ort load-dynamic 的裸名回退走 PATH 搜到系统版 → 加载错库 → ConvInteger 无实现。

**修复**：① `pin_ort_dylib()`（main() 最早处）：存在 `ORT_DYLIB_PATH` 则尊重，否则把 exe 同目录的平台库名（onnxruntime.dll / libonnxruntime.dylib / libonnxruntime.so）钉进 `ORT_DYLIB_PATH`，缺失时打警告；② 本次已把 DLL 复制到 `target/release/`（现有 release exe 无需重编译即可修复——ort 的 exe-dir 优先搜索会命中）。BUILD.md §2 增补该坑的说明。

## C-11.3 · 2025-07 — 新用户首次启动工作流修复与验证

**需求**：删除本机应用数据/模型/旧 demo 数据，模拟新用户首次启动（下载→建库→添加目录→搜索）全流程。

**发现并修复的真实 bug**：引擎加载只在启动时执行一次——新用户模型未下载时加载失败（Degraded），**下载完成后没有任何重试**，语义搜索/打标永久不可用（必须重启）。修复：`spawn_engine_load` 移到 **init_models_async 验证成功之后**（`models verified — loading engine`）；移除 setup 中的早期加载。已有模型时验证为哈希检查（~1-2s），体验不变。

**新用户全流程实测通过**（日志验证）：

1. 全新 DB 创建 + 旧 demo 数据迁移（无旧数据时跳过）
2. 模型 412MB **由应用自带下载器完成下载**（镜像链 hf-mirror，本环境首次真实跑通；断点续传经 .part 验证）
3. **下载完成 → 引擎自动加载**（backend=cuda），无需重启 ✓
4. 添加目录 → 1032 张扫描/缩略图/监控正常
5. 无标签时 `no user tags defined — embedding only, tagging skipped`（零打标尝试，C-11.1 行为可见）✓
6. cpu 模式正常（C-11.2 钉路径生效）✓

**注意**：新用户首次的 1032 张库需要几分钟后台嵌入（一次性成本，语义搜索前可用）；此后增量。

## C-11.4 · 2025-07 — 浮窗文案区分"索引/打标"（无标签时误导）

**现象**：未添加任何标签时添加图片库，右上角浮窗显示"正在标记中"——实际只做**嵌入（索引）**，打标已跳过（C-11.1 日志可见）。文案误导用户以为在打标签。

**修复**：`ai-queue-status` 事件新增 `tagging: bool`（消费者发射时读标签缓存是否非空）；前端浮窗按标志切换文案——无标签显示"**正在索引**·剩余 N 张"，有标签显示"**正在标记中**·剩余 N 张"。i18n 新增 `tagging.indexing/indexingRemaining`（中英）。语义不变：嵌入是语义搜索的前提，仍在前台一次性完成。

## C-11.5 · 2025-07 — CI 便携包审查与修正

**审查发现两个问题**：

1. **Windows 便携包复制了错误的文件名**：cargo 原始产物是 **`tiol.exe`**（小写，`TIOL.exe` 只在安装包内被重命名）——原步骤复制 `TIOL.exe` 会失败/产出缺 exe 的空 zip。修复：`Get-ChildItem tiol.exe, TIOL.exe` 取存在的那个，找不到直接 `throw` 失败。
2. **macOS dmg 缺 dylib（顺序错误）**：tauri 在嵌入 dylib **之前**就生成 dmg → 上传的 dmg 内没有 onnxruntime。修复：macOS 改为 `--bundles app` 只出 .app → 嵌入 dylib → ad-hoc 签名 → `ditto` 打包 `TIOL-macos-arm64.zip` 上传（dmg 留待 bundle.resources 方案）。

**便携包内容（功能完整）**：Windows zip = tiol.exe + onnxruntime.dll + README；macOS zip = TIOL.app（dylib 已内嵌 Contents/MacOS 并 ad-hoc 签名）。安装包（msi/nsis）仍不内嵌 ORT（已知缺口，见 C-11）。

## C-11.6 · 2025-07 — CI 修复：tauri-action 需要 npm install
**CI 首跑失败**：`tauri-action` 通过前端包管理器调用 `npm run tauri build`，而 CI 从未执行 `npm install` → `'tauri' is not recognized`（Win）/ `tauri: command not found`（macOS）。修复：两个 job 在 tauri-action 前增加 `npm install` 步骤（`@tauri-apps/cli ^2.11.4` 在 devDependencies，仅用于 CLI 二进制；前端本身无依赖）。

## C-11.7 · 2025-07 — 视频忽略 / macOS 启动自愈（i18n 重试 + 引擎看门狗）

**① 忽略视频**：scanner 的 ALLOWED_EXTS 移除 `mp4/mov/avi/mkv`——AI 管道与缩略图仅支持图像，视频不应入库（此前会被尝试嵌入→失败标记）。已入库的视频行会在下次扫描时自动清除（不再出现在 seen 集合 → delete_missing）。

**② macOS 语义搜索无结果/嵌入未开始**：最可能原因 = 引擎加载失败（dylib 缺失/CoreML 首次编译等）→ 消费者无限等待引擎 → 队列永不处理。修复：**引擎看门狗**——模型已验证后每 30s 重试加载（最多 10 分钟），瞬时失败自愈；`pin_ort_dylib` 启动即输出 dylib 存在性（缺失会打警告），配合设置页模型状态/调试日志可精确定位。若 macOS 上仍不工作，请提供调试日志（会明确显示 dylib 缺失或引擎加载错误原因）。

**③ macOS 启动时按钮显示 i18n 键名（如 folders.add）**：初始 `fetch(locales/xx.json)` 偶发失败 → messages 为空 → `t()` 返回键名；手动点语言后重新 fetch 成功。修复：`loadMessages` 重试 4 次（300ms 退避）；`initI18n` 初始失败后**后台每 2s 自愈重试**（≤40s），成功即重新应用文案，无需用户手动切换。

## C-11.8 · 2025-07 — 自动发布 Release

**需求**：打 tag 时自动发布 GitHub Release。

**实现**：① workflow 顶层加 `permissions: contents: write`；② 两个 job 的 tauri-action 启用 `tagName/releaseName = v__VERSION__`（用 `startsWith(github.ref,'refs/tags/')` 表达式门控——**只有 tag 推送才建 Release**，main 推送仍仅构建；`__VERSION__` 占位符由 tauri-action 替换为 tauri.conf.json 的 version）；③ `releaseDraft: true`（草稿，人工确认后发布）；④ 双 job 共用同一 tagName → Windows 与 macOS 的产物自动汇入**同一个 Release**；⑤ 便携包（Windows zip / macOS zip）用 `softprops/action-gh-release` 附加（同样 tag 门控）。

**使用**：`git tag v0.1.0 && git push origin v0.1.0` → CI 完成后 GitHub Releases 页出现草稿（含 msi/nsis/.app + 两个便携 zip）→ 人工点 Publish 正式发布。

## C-11.9 · 2025-07 — CI 修复：tauri-action 需要显式 GITHUB_TOKEN

**tag 触发运行失败**：`Error: GITHUB_TOKEN is required`——tauri-action 创建/更新 Release 时读取 `GITHUB_TOKEN` 环境变量，**不会自动注入**，必须在 job 级显式传入。修复：两个 job 加 `env: GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}`（自动 token，无需配置；配合 workflow 级 `permissions: contents: write` 具备写权限）。附带确认：macOS 编译+打包+tar 打包已成功，问题仅在 Release 创建一步。

## C-11.10 · 2025-07 — macOS 引擎永不加载：CoreML 探测卡死

**症状（macOS 日志）**：`models verified — loading engine` 后引擎一直不加载，看门狗每 30s 重试、无失败原因——加载任务在 **CoreML 探测/首次编译**上卡住（int8 模型的 ConvInteger 量化算子 CoreML EP 本就不支持；探测时会做真实 session 构建+冒烟推理，首次 CoreML 编译可长时间挂起），消费者因此永远等不到引擎 → 嵌入/打标/语义搜索全停。

**修复**：① macOS 的 **auto 模式直接走 CPU**（跳过 CoreML 探测——int8 模型本就不该上 CoreML，CPU 推理 ~150ms/张足够）；② `load()` 中任何 provider 的 session 构建失败**自动回退 CPU**（gpu/mlx 显式选择也不再整体失败，warn 日志说明）；③ 页面判断接受 `tauri://` 前缀（macOS 首次加载出现过 `tauri://localhost` 被误判为异常页而重定向错误页，一并修复）。

## C-11.11 · 2025-07 — 仅 tag 触发 CI + i18n 内嵌根治键名问题

**① workflow 改为仅 tag 触发**：移除 `push: branches: [main]`——日常推送不再编译；仅 `tag v*` 触发（构建+草稿 Release）与手动 dispatch（仅构建，不建 Release，tagName 门控保留）。

**② i18n 键名根治（Windows/macOS 均复现）**：初始 `fetch(locales/*.json)` 在双平台启动时持续失败（脚本加载正常、fetch 失败——asset 协议对两类请求的处理路径不同），重试方案无法根治。改为**语言包内嵌**：`scripts/gen-messages.js` 从 `locales/*.json` 生成 `locales/messages.js`（ES module，随前端打进二进制），`i18n.js` 启动时同步读取、**完全移除启动期 fetch**；语言切换仍走持久化设置。修改 JSON 后需重跑生成脚本（脚本已提交）。

## C-11.12 · 2025-07 — macOS 引擎加载失败可见化 + dylib 定位加固

**症状**：新构建（auto→cpu 生效）在 macOS 上仍"模型加载错误"，且日志无失败原因——ort 在 dylib 缺失/加载失败时内部 `expect()` **panic**（panic 只进 stderr，GUI 应用不可见）→ 看门狗静默重试、UI 只有笼统错误。

**修复**：① `spawn_engine_load` 用 `catch_unwind` 包裹加载，panic 消息写入日志与模型状态（含 BUILD.md §2 指引：`libonnxruntime.dylib` 需与可执行文件同目录）；② `pin_ort_dylib` 增加候选目录 `Contents/Frameworks`（macOS .app 惯例，防未来打包布局变化）；③ CI 的 macOS 嵌入步骤加 `test -f` 存在性校验（dylib 下载/嵌入失败即 job 报错，不再静默产出残缺包）。

## C-11.13 · 2025-07 — macOS BadVersion：onnxruntime 版本过旧

**确切报错**（catch_unwind 生效后拿到）：`Failed to load ONNX Runtime dylib: BadVersion { version_str: "1.16.3" }`——ort 2.0.0-rc.13 的 `ORT_API_VERSION = 17`，**运行时 onnxruntime 必须 ≥ 1.17**；CI 钉的 1.16.3 被版本检查拒绝（Windows 一直正常是因为内置 DLL 是 1.27）。

**修复**：CI 的 macOS onnxruntime 升级到 **1.20.1**（`onnxruntime-osx-universal2-1.20.1.tgz`，universal2 + CoreML EP）；BUILD.md §2 明确标注"必须 ≥ 1.17（推荐 1.20.x）"。**本地 macOS 构建的用户也需更换 ≥1.17 的 dylib**（当前 1.16.3 直接报 BadVersion）。

## C-11.14 · 2025-07 — macOS 全链路修复：fp16 模型 + CoreML 选项 + 图优化降级

**背景**：macOS 上 CPU 回退也报 `ConvInteger(10)`——**Apple Silicon 的官方 ORT CPU EP 没有 ConvInteger 内核**（MLAS 量化卷积在 ARM64 macOS 缺失），int8 模型在 macOS 上根本无法运行。

**修复**（全套）：

1. **模型平台化**：`MODEL_LOCK` → `model_lock()` 按平台返回——macOS 用 **fp16**（vision 186MB + text 564MB + tokenizer ≈ 784MB，哈希已锁定：`a1959f7b...` / `711da56a...`）；Windows/Linux 保持 int8（412MB）。`engine.rs` 按平台选模型文件。
2. **图优化 Level1**：fp16 导出的 `InsertedPrecisionFreeCast` 节点会让 1.20.x/1.27 的 LayerNorm 高级融合崩溃（`GetIndexFromName ... does not exist`；Python ORT 1.28 已修）。`build_session` 全局降到 **Level1 基础优化**——任何版本安全，int8 路径回归验证无损失（coffee=0.119 正常）。
3. **onnxruntime 1.20.1 → 1.28.1**：新版已改为分架构打包（`onnxruntime-osx-arm64-1.28.1.tgz`，30MB）——1.28 修复了融合 bug，且与 Windows DLL（1.27）同代。Intel Mac 不在支持范围（.app 仅 arm64）。
4. **MLX 选项 → CoreML 选项**：fp16 模型让 CoreML 可行（CoreML 支持 fp16/fp32、不支持 int8 量化）——设置页删除 "Apple MLX"，新增 **"Apple CoreML"**（`coreml` provider：macOS 走 CoreML EP，构建失败自动回退 CPU，不预探测避免首次编译卡顿）；i18n 同步。
5. **fp16 管道验证**：新增 `fp16_pipeline_check`（本机 CPU 直接验证 fp16 图加载 + 预处理 + pooler 提取 + 余弦尺度，sim=0.135 通过）——无需 Mac 即可确认管道正确。
6. 附带：`.gitignore` 增加 `models-fp16-dl/`（fp16 暂存目录）。

## C-10.5 · 2025-07 — 审查修复：unknown 双标签 / 平台与 i18n 审计 / 清理与 .gitignore

**需求**：① 有时照片同时获得正常标签和 unknown ② 检查 macOS/Windows 兼容性与中英文支持 ③ 代码逻辑复查 ④ 清理开发期无用缓存、写 .gitignore、更新 CHANGES.md。

**① unknown 双标签修复**：根因——**单标签路径（新标签）匹配成功时不清理 unknown 哨兵**：照片先被全量检查打成 unknown，之后新标签命中（单标签检查）→ unknown 保留 + 新标签写入 = 双标签。修复：单标签命中时先 `clear_unknown_tag`；另加启动自愈 `cleanup_stray_unknown()`（历史残留：删除"同时带真实标签与 unknown"照片上的 unknown，启动日志 `cleaned N stray 'unknown' tags`）。

**② 平台兼容性审计**（结论：代码已跨平台）：

- Windows：`reveal_in_folder` 用 `explorer /select`（cfg 门控）✓；CUDA/DirectML 提供链 ✓；`windows_subsystem` cfg 门控 ✓；`onnxruntime.dll` 打包（win-x64）✓。
- macOS：`open -R` 显示文件 ✓；CoreML + `mlx` 模式（Apple 加速器，探测失败回退 CPU）✓。
- Linux：`xdg-open` ✓（未经真机验证）。
- 注意（打包层面）：macOS/Linux 构建需各自平台的 onnxruntime 动态库（仓库只内置 win-x64 版）。
- i18n：zh-CN/en-US 全部键对同步（本轮起每次新增键成对添加）；`t()` 缺键回退键名；app.js 无硬编码 UI 字符串；index.html 仅 2 个启动即被覆盖的状态栏占位。

**③ 代码逻辑复查**：clippy 清零（移除 TagVec 未用 `id` 字段、watcher `tx` 标注 `#[allow(dead_code)]` 并注释用途、4 处冗余闭包）；watcher 扫描入队带当前 epoch（C-10.4）；启动只处理变更文件（C-10.3）；单测 6 项通过。

**④ 清理与 .gitignore**：

- 删除开发期无用缓存/杂物：`.npm-cache`（npm 失败尝试的日志缓存）、`node_modules`（0.8MB 残缺安装）、`dev-log.txt`、`npm-trace.txt`。保留 `.cargo-home`（离线 vendored 依赖，删了无法离线编译）、`target/`（构建产物）。
- `.gitignore` 扩充：新增 `.cargo-home/`、`.cargo/`、`models-download/`、`src-tauri/resources/`（运行时下载模型不入库）、`*.part`、`*.bundled.tmp`、`*.log`、`dev-log.txt`、`npm-trace.txt`、`test_imgs/`、系统/编辑器杂项（.DS_Store/Thumbs.db/.idea/.vscode）。
- **UI 滚动修复（C-10.5 附）**：设置页与目录页内容超出窗口时无法滚动/被压缩——`.settings` 与 `.folder-list` 补 `flex:1; min-height:0; overflow-y:auto`（flex 列子项默认 `min-height:auto` 阻止收缩导致溢出被裁剪），并加 `.settings > * { flex-shrink:0 }` 防止子项（如调试日志面板）被压缩——容器滚动而非内容挤压。

## C-10.6 · 2025-07 — 跨平台审计 + 默认语言改英文

**平台兼容性审计**（代码全部跨平台，仅打包层有注意事项）：

- **路径**：存储键统一 `normalize_storage_path`（dunce + 正斜杠 + 小写）✓；应用数据/模型目录全部走 tauri `path()` 解析器（Windows `%APPDATA%`/`%LOCALAPPDATA%`、macOS `~/Library/Application Support`）✓；`LOCALAPPDATA` 仅出现在测试辅助函数（`#[ignore]` 且可 `TIOL_MODEL_DIR` 覆盖）✓。
- **代码**：`reveal_in_folder` 三平台分支（explorer /select、open -R、xdg-open）✓；GPU 提供链按平台 cfg（CUDA+DirectML / CoreML / 空）✓；`mlx` 模式非 Apple 平台回退 CPU ✓；`windows_subsystem` cfg 门控 ✓；sqlite bundled（源码编译）✓；reqwest rustls ✓。
- **打包注意事项（文档级）**：① macOS 需要 `icon.icns`——当前只有小尺寸 icon.png/icon.ico，macOS 打包前须 `npx tauri icon <1024px png>` 生成全套；② macOS/Linux 需要各自平台的 onnxruntime 动态库（仓库仅内置 win-x64 onnxruntime.dll）；③ Windows 安装包需要 NSIS/WiX（本机未装，联网下载被墙，可用 `cargo build --release` + 手动拷贝 DLL 组装便携版）；④ `src-tauri/.cargo/config.toml`（本地代理）已被 .gitignore 排除，不会污染 macOS 构建。

**默认语言改英文**：`i18n.js DEFAULT_LANG = "en-US"`（SUPPORTED 顺序同步调整）；index.html 两处内联状态栏占位文本改英文（启动即被 i18n 覆盖，仅防闪烁）；已有用户持久化的 `language` 设置不受影响。


## C-10 · 2025-07 — 标记管理完善：删除连带清理 / 清除标记 / 进度浮窗 / 卡片刷新

**需求**：① 删除自定义标记时同时删除照片上的对应标记 ② 清除缓存旁加"清除标记"按钮（带警告确认）③ 后台有标记任务时右上角显示进度小浮窗（进度条 + 剩余数量，新任务加入实时更新）④ 标签更新后切回照片页卡片标签立即刷新。

**实现**：

- **删除连带清理**（`db` + `main.rs`）：`delete_custom_tag` 先查名称（`get_custom_tag_name`），删除定义后 `remove_tag_everywhere(name)`——删除 `tags` 行，FK ON DELETE CASCADE 自动清掉所有照片上的该标签（卡片/标签搜索/计数同步消失）。
- **清除标记**：设置页"清除缓存"旁新增"清除标记"按钮 → 应用内**确认对话框**（`#confirm-overlay` 通用组件，复用 dialog 样式）→ `clear_all_tags` 命令清空 `file_tags` + `tags` + `custom_tags`（完整重置），随后刷新设置列表与照片页。
- **进度浮窗**：`queue.rs` 的进度事件由 `ai-progress`（每 5 条、DB 统计）改为 **`ai-queue-status`**（每任务，300ms 节流，队列清空必发）：`{done, remaining}`，`remaining = rx.len()`（**新任务入队实时反映在剩余数上**）。前端右上角固定浮窗（所有视图可见）：剩余张数 + 进度条（done/(done+remaining)），remaining=0 自动隐藏；设置页模型状态行继续复用同一事件。
- **卡片标签刷新**：① 切到照片视图时若未在搜索则重新 `loadPhotos()`（拉取最新 tags）② 监听 `ai-queue-status` 且队列清空时，若照片视图可见且未在搜索，自动刷新。
- i18n：`dialog.*`、`tagging.badge/remaining`、`settings.clearTags/clearTagsConfirm`（中英同步）。

**修复（C-10.1）**：

- **浮窗永远显示、卡在"剩余 1 张"**：`.tagging-badge { display:flex }` 压过 UA 样式 `[hidden]{display:none}`（作者样式优先级更高）→ `hidden` 属性失效，徽标永不隐藏、停在最后一次计数。修复：`.tagging-badge[hidden] { display:none; }`（`.dialog-overlay[hidden]` 早有此规则，新组件漏了）。
- **调试模式输出打标置信度**：`process_one` 现在对**每张照片**输出 `tag <id> <path>: tag1=0.122, ...`（top5 带分数）；无匹配时输出 `no match (best 0.043, tags [human:0.07, ...]) -> unknown`——调试面板可直接看到打标依据与阈值对比。

**修复（C-10.2）**：

- **多标签语义**：新增标签时，**已带其他标签的照片也要参与匹配**（一张图可有多个标签）。`get_files_needing_tags` 改为 `get_files_missing_tags`：`ai_processed=3` 且**缺少任一当前定义标签**的文件才入队（含仅 unknown 的）。SQL：`(该文件命中的当前标签数) < (当前标签总数)`。新标签加入后，缺它的照片（无论已有几个标签）都会被重打标，`process_one` 会全量重算所有标签的匹配（INSERT OR REPLACE，幂等）。新增单测 `missing_tags_multilabel_semantics`（7 项全过）。
- **设置页标签计数自动刷新**：`ai-queue-status` 队列清空时，前端现在同时调用 `renderTags()` 刷新设置页每个标签的匹配照片数（无需手动切换界面）。

**优化（C-10.3）**：

- **新增标签只查新标签**：`AITask` 增加 `tag: Option<TagVec>`——`add_custom_tag` 后经 `retag_new_tag` 对"尚未带该标签"的照片（`get_files_without_tag`）入队**单标签任务**：只算新标签的余弦、只写新标签的匹配，**完全不动已有标签**，也不写 unknown（照片可能已有其他标签）。
- **启动不再全库重打标**：移除启动时的 `get_files_missing_tags` 全量入队——启动/扫描只处理 scanner 检测到**变更/新增**的文件（size+mtime 比对，`ai_processed` 归 0 者）。存量照片只在"新标签加入（单标签检查）"或"文件本身变更"时再处理。**说明**：md5 内容比对未实现——size+mtime 是标准廉价启发式（逐文件内容哈希会显著拖慢大库启动）；如需可后续加。
- **读库复用 embedding**：新增 `db.get_embedding`，重打标时直接读存储向量，不再重复跑视觉编码器（原代码 has_embedding 后仍会重 embed，约 300ms/张的浪费）。
- **默认阈值 0.06**（前端默认值 + 校验提示同步；实测 0.06 在描述性/单字标签上均更均衡）。

**修复（C-10.4）**：

- **watcher 扫描丢弃 AI 任务（真实 bug）**：文件监控的防抖扫描调用 `scan_folder` 后**丢弃了返回的 pending 文件 ID**——会话期间新增/修改的照片永远不会被嵌入/打标（直到下次启动），且不通知前端刷新（列表陈旧）。修复：`FileWatcher::start` 接收 AI 队列 + `AIControl` + `AppHandle`，扫描后把变更文件**入队**（携带当前 epoch）并 `emit scan-complete` 刷新前端；`rebuild_watcher` 相应接收 `AppHandle`（add/remove/scan_folders 命令签名增加 AppHandle 参数）。日志：`watcher: enqueued N changed files for AI`。

**测试**：`cargo check` 通过（清理了已无调用方的 `count_pending_ai`）；前端语法/JSON 校验通过；待应用内实测。

## C-09 · 2025-07 — 纯 SigLIP 架构迁移（MIGRATE1.md V3.0）

**需求**：按 MIGRATE1.md 从双模型（SigLIP + DeepDanbooru）迁移为**纯 SigLIP**——移除 DeepDanbooru 模型/代码/测试/配置，改由 SigLIP 文本编码器对**用户自定义标签**做零样本匹配打标签；语义搜索/标签搜索/手动标签编辑不变；清理遗留文件；前端 i18n 一致。

### 重大技术发现（本次迁移的核心修复）

1. **输出选择错误（元凶）**：onnx-community 的 SigLIP 导出有**两个输出**——`last_hidden_state` 与 `pooler_output`。旧代码取第一个输出（raw hidden states）：图像做 mean-pool、文本取 EOS 行——**都在投影前，不在对比学习空间**。实测图像-文本余弦全部糊在 0.01-0.08（grass 图竟 food 最高），零样本打标签完全不可用。
   **修复**：`extract_pooled()` 取 `pooler_output`（投影后对齐嵌入）→ 匹配分数跃升至合理区间（描述性标签匹配 0.09-0.13，噪声 <0.08，排序正确）。语义搜索质量同步提升。
2. **文本池化**：pooler_output 本身 = EOS token → 投影头（HF SiglipTextTransformer.head），无需手工池化。
3. **图像预处理**：SigLIP 继承 CLIP 归一化——像素 `/255 → (v-0.5)/0.5 ∈ [-1,1]`（旧代码只做 /255）。
4. **嵌入空间版本门**：旧库中按错误管道算的 embedding 与新空间不兼容 → 新增 `embed_version` 设置（当前 `pooler-v1`），不匹配时启动即 `reindex_embeddings()`（清 embedding + 自动标签 source=1，保留手动标签，ai_processed 归 0 全量重处理）。

### 自定义标签（替代 DeepDanbooru）

- `add_custom_tag(name, threshold)` **签名变更**：由参考图均值向量改为**文本向量**（`embed_text(name)` 存入 `custom_tags.embedding`），threshold ∈ [0.01, 0.5]（默认 **0.08**，按 pooler 尺度校准——旧 schema 默认 0.25 已不适用）。
- `get_custom_tags` 增加 `photo_count`（每标签命中照片数）。
- 标签向量缓存 `tag_cache`（AppState 共享，std RwLock）：启动时引擎就绪后重建；增删标签即时重建 + **立即重入队未标记照片**（`retag_untagged`）。
- `process_one` 统一管道：嵌入（已有则跳过）→ 与缓存标签向量点积（余弦）→ `> 该标签阈值` 写入 `file_tags`（source=1, confidence=点积）→ 无命中且标签列表非空 → `unknown`（source=1, conf=1.0，防止重复检测）→ `ai_processed=3`。标签列表为空时跳过自动打标（不刷 unknown）。
- **unknown 哨兵语义（实测后修正）**：unknown 只是哨兵，**绝不阻止重新打标**——重打标前先 `clear_unknown_tag()` 清除；`get_files_needing_tags()` 把"仅含 unknown"的文件也纳入重入队（否则用户先加标签 A 全库变 unknown，再加标签 B 就永远匹配不到了——首次实测踩中此坑）。
- 使用提示（实测）：**描述性标签效果好于单字**（"a cup of coffee" 0.122 vs "food" 0.064），已写进测试与前端占位文案。

### 移除清单（代码 + 文件）

| 项 | 说明 |
|---|---|
| `engine.rs` Tagger/predict_tags/阈值常量/深丹测试 | 删除，替换为 `TagVec` + `cosine()` |
| `queue.rs` tagger 参数/空闲卸载/`tagger_broken` | 删除（无独立打标模型） |
| `model_lock.rs` deepdanbooru.onnx（bundled）+ tags.txt 条目、`bundled` 字段 | 回退为 3 个 SigLIP 文件 |
| `downloader.rs` `ensure_bundled`/resource_dir 参数 | 回退 |
| `resources/`（154MB int8）、`scripts/download-deepdanbooru.js`、`download-models.js` 的深丹条目、`tauri.conf.json` resources | 删除 |
| 本机 `%LOCALAPPDATA%\...\models\deepdanbooru.onnx` + `tags.txt`（154MB+） | 删除 |

### 前端

- 设置页新增**自定义标签**区块：名称输入 + 阈值（0.01-0.5，默认 0.08）+ 添加；标签列表（名称/阈值/命中数/删除）。i18n：`settings.tags*`（中英同步）。
- 其余（搜索模式下拉、卡片标签、✎ 手动标签、置信度徽标、推理进度）保持不变。

### 测试与验证

- `siglip_tag_match_sanity`（ignored，真实模型）：描述性标签在测试图上 top-1 正确（coffee=0.122）。
- `preprocess_experiment`（ignored，诊断用）：输出多图×多标签余弦，供调阈值参考。
- 全量单测 6 通过；启动日志验证：`embedding pipeline changed (pooler-v1) — re-indexing all photos` → 12 文件重嵌入 → `tag cache: N tags embedded`；无 deepdanbooru 残留（grep 归零，仅文档提及）。

### 与 C-08 的关系

C-08 的 DeepDanbooru 方案（含 0.6 阈值校准、154MB 打包资源）被 C-09 **完全取代**，保留为历史记录。MIGRATE1.md §4.1 建议的 naflex 模型（~137MB）为**可选后续优化**（需重新下载+锁定哈希），本次未实施。

## C-08 · 2025-07 — DeepDanbooru 自动打标签（恢复）

**需求**：恢复 DeepDanbooru 打标签：① 编辑描述改为编辑标签（逗号分隔）② 后台监控恢复，但 SigLIP 与 DeepDanbooru 不持续运行——仅在发现未被 tag 的照片时处理 ③ 模型未识别出元素时打 `unknown` 标签防止下次启动重复检测 ④ 搜索框旁下拉框切换语义/标签搜索 ⑤ 中英双语 + 文档。

**模型**（关键决策）：

- 公开源**没有 int8/fp16 的 DeepDanbooru**（fp32 为 643MB）。方案：下载 Neus/Onnx_DeepDanbooru fp32 → 本机用 Python onnxruntime **动态量化 int8（154.4MB）**，作为 **Tauri 打包资源**分发（`bundle.resources` + 启动时复制进模型目录），避免用户下载 643MB。
- 该模型与经典 DeepDanbooru 不同：**输入 [1,512,512,3] NHWC、输出 9176 类**（非 224²/11827）；同源 `tags.txt`（9176 行）保证标签顺序一致。int8 CPU 推理约 150ms/张（实测）。
- 量化产物哈希已写入锁文件：`deepdanbooru.onnx` 161941374B / `fd750f41...`（bundled）；`tags.txt` 124634B / `a4020357...`（可下载）。
- 复现量化：`scripts/download-deepdanbooru.js`（fp32 断点续传）+ `quantize_dynamic(weight_type=QInt8)`。

**后端**：

- `engine.rs`：新增 `Tagger`（`deepdanbooru.onnx` + `tags.txt`，CPU EP，`predict_tags` 返回置信度 > **0.6** 的标签，sigmoid 排序）。**阈值校准（重要）**：该 9176 类模型的 logits 被压缩——真实标签得分 0.60~0.73，而垃圾标签全部挤在 0.50~0.55（经典 DeepDanbooru 的 0.5 阈值会输出数千个垃圾标签，实测 7 张图共打出 49,373 个标签）。阈值 0.6 一刀切干净，已加回归断言（每张 <100 标签）。
- `queue.rs`：`process_one` = ① SigLIP 嵌入（已有 embedding 则跳过，避免历史文件重复向量化）② DeepDanbooru 打标签（**懒加载**；模型缺失/损坏时跳过打标签但照常完成）③ `ai_processed=3`。**空闲 5 分钟自动卸载 tagger**（约 155MB 内存释放），新任务到达时重新加载——模型不持续驻留（用户要求）。SigLIP 保持常驻但仅按需推理（搜索/新文件）。
- 打标结果为空 → 写入 `unknown` 标签（source=1，confidence 1.0）→ 下次启动不再检测。
- `main.rs`：启动时除 `ai_processed=0` 外，**追加入队"已处理但无标签"的历史文件**（embed-only 时代的存量，仅打标签不重复嵌入），等待 bundled 模型就位（轮询 ≤15s）。
- `db`：`FileRecord.tags`（GROUP_CONCAT 子查询，全查询共用 `FILE_COLS`/`FILE_COLS_F` 常量）；新增 `get_file_tags`（含 source）、`replace_manual_tags`（source=0 替换）、`get_untagged_done_files`、`has_embedding`；`FileTag` 结构。
- `downloader.rs`：`ModelFileInfo.bundled` 支持——bundled 条目从 `resource_dir` 复制而非下载（校验哈希后原子改名）。

**前端**：

- 搜索框右侧新增**模式下拉框**（语义搜索 / 标签搜索），`runSearch` 按模式调用 `search(mode: semantic|tag)`。
- ✎ 对话框改为**编辑标签**（预填手动标签 source=0，逗号分隔输入）；保存后原地刷新卡片标签行。
- 卡片元信息显示**标签列表**（替代原描述显示；`description` 列保留但不再展示）。
- i18n：`search.mode.*`、`search.tag.error`、`card.edit.title/placeholder`（中英同步）。

**测试**：`deepdanbooru_predicts_sane_tags`（真实模型冒烟：4 张测试图标签合理、全在词表内、阈值生效；需 `ORT_DYLIB_PATH` + `TIOL_MODEL_DIR`）。

**验证**：真实照片输出如 `food, plate, drinking_glass`、`grass, tree, nature`、`blurry, depth_of_field` —— 识别正确；int8 模型 154MB 打包，CPU 150ms/张，适合普通笔记本。

## C-07 · 2025-07 — 调试模式：卡片显示 AI 置信度

**需求**：开启调试模式时，thumbnail 卡片显示 AI 识别的置信度，方便测试效果。

**实现**：

- 后端（`db/mod.rs`、`search/mod.rs`）：`FileRecord` 新增可选字段 `score: Option<f32>`（`#[serde(default, skip_serializing_if = "Option::is_none")]`，非语义搜索路径为 `None`，序列化时省略）；`semantic_search` 把每条的相似度点积 zip 回结果（`get_files_by_ids` 保持请求顺序）。命令签名不变，前端零侵入。
- 前端（`app.js`、`styles.css`）：新增 `debugMode` 实时标志（启动时预读 `debug` 设置 + 设置页开关切换时同步 + 立即重渲染卡片）；`buildCard` 在 `debugMode && p.score != null` 时于缩略图左上角加 `AI 0.xxx` 徽标（`.card__score`，黑色半透明底 + 绿色等宽字体，pointer-events: none）。
- 徽标只出现在**语义搜索**结果卡片上（普通浏览 `score` 为 `None`）。

## C-06 · 2025-07 — 修复：预览 / 清缓存 / 缩略图全挂

**症状**：点击图片打不开右侧预览；设置里清除缓存报错；缩略图完全不渲染（点击也无用）。

**根因**（仪器化确认，`report_js_event` 上报抓到）：

1. **唯一真凶（前端）**：`setThumb` 重构为 `enqueueThumb` 时，队列项改为 `{img, photo}`，但 `thumbQueue.findIndex((q) => q.p.path === ...)` 仍引用旧字段 `q.p`（undefined）→ 队列非空后每次入队都抛 `TypeError: Cannot read properties of undefined (reading 'path')`。连锁反应：
   - 初始加载循环在第 2 张卡中断（`renderPhotos` 抛错）→ 缩略图几乎不入队；
   - 观察器回调抛错 → 其余卡片永远不被请求；
   - 点击处理器 `setThumb()` 先于 `preview.open()` 抛错 → **预览打不开**；
   - 清缓存成功后的 `renderPhotos(currentPhotos)` 抛错 → catch → **弹 alert 报错**（并非 remove_dir_all 失败）。
2. **本轮新引入（后端）**：`generate_thumbnail` 原子改名时临时文件名为 `<hash>.tmp{pid}_{seq}`，无图片扩展名，`image` crate 按扩展名判格式 → `save` 全部失败（`The file extension ."tmp..." was not recognized`）。改为 `save_with_format(..., ImageFormat::Jpeg)`。

**修复**：

- `src/app.js`
  - `enqueueThumb`：`q.p.path` → `q.photo.path`。
  - 新增前端错误上报：`window error / unhandledrejection` → `report_js_event` 命令写入日志缓冲（调试面板 + stderr 可见），缩略图失败原因也上报（`thumb-fail`）；初始加载循环逐卡片 try/catch（单卡失败不拖垮整屏，失败卡不标记 `_initial` 留待重试）；循环后无条件 pump；点击处理器 `setThumb` 包 try/catch，保证 `preview.open()` 一定执行。
- `src-tauri/src/utils/mod.rs`
  - `generate_thumbnail` 唯一临时文件 + 原子 rename，且**显式指定 JPEG 格式**。
- `src-tauri/src/main.rs`
  - `clear_cache` 改为最多 5 次重试 + 递增退避（250ms×n），容忍瞬时文件锁。
  - 新增 `report_js_event(kind, message)` 命令。

**教训**：重构后字段名不一致不会编译报错（JS 动态类型）——前端错误上报（webview 错误 → 后端日志）是唯一可靠的调试通道，已内置。

## C-05 · 2025-07 — 语义搜索「人」崩溃修复

**症状**：语义搜索输入单字（如「人」）panic：`index out of bounds: len 1, index 1`；第二次查询再 panic `PoisonError`。

**根因**：int8 文本模型导出后**只有 1 个输入 `input_ids`**（`attention_mask` 被融合进图），代码硬编码取 `inputs()[1]` → 越界 panic → session Mutex 被毒化 → 后续搜索全部连锁崩溃。

**修复**（`src-tauri/src/ai/engine.rs`）：

- `embed_text` 按 session **实际声明的输入名动态构造**输入：第一个给 `input_ids`，仅当声明了第二个输入才补 `attention_mask`。
- 所有 session 锁改为毒化安全：`lock().unwrap_or_else(|e| e.into_inner())`（`embed_image` 同样处理）。
- 空 token 序列返回错误而非喂空张量。
- 新增回归测试 `ai::engine::tests::text_embed_never_panics`（真实模型，默认 `#[ignore]`；运行需 `ORT_DYLIB_PATH` 指向 `target\debug\onnxruntime.dll`，因为测试 exe 在 `deps\` 找不到 DLL）。

## C-04 · 2025-07 — 缩略图首行不渲染修复

**症状**：只有最后几张渲染，第一行不渲染（点击后才出）。

**根因**：初始加载循环引用 `card._img`，但 `buildCard` 只设置了 `thumb._img`（挂在子元素上）→ 循环是**死代码**，第一屏从未入队；只有不可靠的 IntersectionObserver 初始回调"碰巧"送到的卡片（最后几张）渲染了。另：`setThumb` 每次入队即 pump，倒序迭代导致服务顺序从底部行开始。

**修复**（`src/app.js`）：`buildCard` 补 `card._img = img`；初始循环改为**先入队后标记 `_initial`**（顺序关键）；整屏入队后**统一 pump 一次**（队列 [0..n] 自上而下）；`setThumb` 拆为 `enqueueThumb`（纯入队，返回是否新入队）+ 包装（观察器/点击路径保持即时 pump）。

## C-03 · 2025-07 — Apple MLX 选项

- 设置「推理后端」新增第 4 选项 **Apple MLX**（i18n：`settings.aiMlx`）。
- 后端 `"mlx"` 模式（`engine.rs` `apple_accel_providers()`）：ort 2.0.0-rc.13 **无 MLX 执行器**，macOS 用 CoreML（Apple 原生加速器），先做真实冒烟探测；不可用（如 Windows）→ 回退 CPU 并打日志，状态栏显示真实后端。

## C-02 · 2025-07 — 队列失效 / 搜索框精简 / 推理进度 / 调试模式

- **删除目录清空 AI 队列**（`ai/queue.rs`）：`AIControl` 纪元（epoch）计数；`remove_folder` 时 `invalidate()` 纪元 +1，消费者跳过旧纪元任务（不入队即丢弃），新任务不受影响（无竞态，优于粗暴 drain）。前端缩略图队列在 `renderPhotos` 时本就清空。
- **删除中间描述搜索框**：`#desc-search-input` 及其接线全部移除，保留文件名 + 语义搜索（后端 `search_description` 命令保留，无副作用）。
- **推理进度**：消费者每处理 5 张发 `ai-progress` 事件 `{done, remaining}`，设置页模型状态行显示「推理中 X/Y」（`settings.aiProgress`）。
- **调试模式**（设置页开关，持久化 `debug` 设置）：开启后全局日志级别提到 Info，显示深色日志面板，每秒轮询 `get_logs`（`src-tauri/src/logbuf.rs` 500 行环形缓冲，由 env_logger 自定义格式同时写 stdout 与缓冲）。

## C-01 · 早期关键改动（摘要）

- **SigLIP 语义搜索**：vision/text int8 模型（768 维）+ tokenizer，全本地推理；DeepDanbooru 自动打标签按产品决策延后。
- **推理后端选择**：auto（探测 CUDA→DirectML→CoreML→CPU，真实冒烟推理验证）/ gpu / cpu / mlx。
- **模型锁与下载**（ADD.md §4）：硬编码 URL/大小/SHA256，校验失败即删重下，断点续传（.part）+ 原子改名，镜像链 hf-mirror→openi→huggingface，磁盘空间预检。
- **ort 2.0.0-rc.13 API 要点**：`Session::builder()` → `with_execution_providers()`（消费 self）→ `commit_from_file(&mut self)`；`run(&mut self, ...)` 需要 Mutex 包裹；`inputs!` 返回 Vec（键可为运行时 String）；`Tensor::from_array((Vec<i64> 形状, Vec<T>))`；`try_extract_tensor` 返回 `(&Shape, &[T])`。
- **构建环境**：crates.io 直连被墙 → `scripts/cargo-proxy.js`（127.0.0.1:8013 → ustc 稀疏索引）+ `vendor-crates.js` 离线打包；`CARGO_HOME=E:\ImageManager\.cargo-home`；GUI 运行需 `danger-full-access` 权限。
- **调试辅助**：`node:sqlite`（Node ≥22）可直接查 `%APPDATA%\com.tiol.desktop\db.sqlite`（只读）。
