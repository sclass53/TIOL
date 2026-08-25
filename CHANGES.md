# TIOL — 变更记录 (CHANGES)

> 记录影响行为的关键改动与修复，供后续开发参考。环境注意事项见 BUILD.md / LIMITS.md / ADD.md。
> 改动编号规则：**C-NN**，按时间倒序（最新在最上）；引用改动时直接写编号。

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
