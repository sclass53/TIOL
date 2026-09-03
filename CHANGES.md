# TIOL — 变更记录 (CHANGES)

> 记录影响行为的关键改动与修复，供后续开发参考。环境注意事项见 BUILD.md / LIMITS.md / ADD.md。
> 改动编号规则：**C-NN**，按时间倒序（最新在最上）；引用改动时直接写编号。

## C-19.23 · 2025-08 — 同名 RAW+JPEG 只索引 JPEG / 搜索结果补显 RAW 孪生 / 索引提速 / 「隐藏重复 RAW」

**需求**：① 同一照片（同名）RAW 与 JPEG 并存时只 index JPEG（减少索引量——RAW 解码预览是全库最慢路径）；② 搜索时两个仍都显示（勾选「隐藏重复 RAW」才隐藏）；③ index 太慢要查因提速；④ 菜单文案「不显示重复 RAW」改「隐藏重复 RAW」。配对范围用户确认：**全库同名配对**（RAW/JPEG 常分目录存放）。

**实现**：

1. **索引跳过 RAW 孪生**（AI 队列层）：`db::find_jpeg_twin(file_id)`——任意目录、同 stem（NOCASE）、jpg/jpeg 扩展的其它文件。`queue::process_one` 开头：RAW 且有 JPEG 孪生 → 直接跳过（不嵌入、不改 `ai_processed`——JPEG 后来消失时 RAW 下次启动自动恢复待索引，自愈）。手动「AI Tagging」对跳过 RAW 改为**复制孪生 JPEG 的 AI 标签**（source=1，滤 unknown，不动手动标签）——RAW 无需自身嵌入也能收敛出标签。
2. **搜索结果补显 RAW 孪生**：`db::raw_twins_of(hits)`（全库 filename 一次扫描内存分组）→ `semantic_search` 命中后把同 stem RAW 附加到结果尾部，**携带 JPEG 的 score**（顺序/徽章一致）——未勾选「隐藏重复 RAW」时一对两个都显示；勾选时前端既有 `filterDupRaws` 原样剔除（两处各司其职，前端零改动）。tag/文件名搜索不扩展（RAW 无标签/名字本就各自命中）。
3. **索引提速**（两处）：
   - **缩略图缓存直嵌**：`engine::embed_dynamic(img)`（拆自 embed_image：解码与归一化分离）；`queue::embed_task_image`——文件缩略图（key = hash(display path, mtime)，与前端/prewarm 同键）存在时从 360px 小图解码嵌入（毫秒级），miss/损坏回退全尺寸解码（行为不变）。
   - **去掉每任务 100ms 固定空等**：原「空闲节流」对每个任务无条件 sleep——690 文件纯延迟 ~69s；改为仅队列空时 sleep（防 busy-spin，不拖累吞吐）。
4. **文案**：`menu.hideDupRaw` zh→「隐藏重复 RAW」、en→「Hide duplicate RAW」（菜单/消息同步，i18n 183 keys 全绿）。

## C-19.22 · 2025-08 — 启动卡死修复（TDZ）/ RAW 预览解码 panic 防护 / 语义搜索体验修正

**需求（跟进 19.21 后测试）**：① 启动后按钮可点但逻辑全死（疑似死锁/卡死）；② RAW 内嵌预览解码大量线程 panic（`copy_from_slice` 长度不匹配）刷屏；③ 语义搜索输入区分大小写；④ 模型加载完成前的搜索显示「无结果」；⑤ gallery 图标在废片/重复页点击须回正常视图。

**实现**：

1. **启动卡死（根因 TDZ）**：`syncHideRawCheck()` 在模块顶层被调用，但其读取的 `hideDupRaw` 由更靠后的 `let` 声明（模块级读 localStorage 初始化）→ TDZ `ReferenceError` 中止整个模块初始化，**后续所有事件绑定全部丢失**（症状 = 按钮样式正常但逻辑全死）。修复：`hideDupRaw` 状态块上移到菜单处理器与 `syncHideRawCheck()` 调用之前。
2. **RAW 预览解码 panic 防护**（C-19.21 RAW 扫描的隐患）：`image` 0.24.9 的 JPEG 解码器对部分 RAW 全尺寸预览段（索尼 ARW 等）不返回 `Err` 而是直接 panic（`copy_from_slice`：源 393216 ≠ 目标 196608），未捕获时 panic 刷屏并干扰 worker。修复：候选段解码包 `std::panic::catch_unwind`，且调用期间临时静音 panic hook——坏段仅被跳过（自动回退到更小的内嵌缩略图），同文件其它候选与整个任务不受影响，日志恢复干净。
3. **语义搜索小写归一化**：嵌入模型区分大小写，照片/废片搜索框提交前统一 `toLowerCase()`。
4. **引擎未就绪自动重试**：后端答「引擎未就绪」时前端显示 loading 文案并每 2s 自动重试（上限 20 次），引擎加载完成后自动出结果，无需手动重搜。
5. **gallery 图标语义**：iconbar gallery 图标点击一律 `switchView("photos")`——废片/重复页同样能一键回正常视图（侧栏相机图标负责族内刷新，分工见 C-19.20）。

## C-19.21 · 2025-08 — 切页搜索失效修复 / RAW 支持 / 「不显示重复 RAW」视图筛选

**需求**：① 主页切到别的页再切回，搜索失效（须改动字符串再回车才能重搜）；② 支持各类相机 RAW；③ 「视图」菜单分割线下新增勾选「不显示重复 RAW」：视图内同名 JPEG 与 RAW 并存时只显示 JPEG，对三个视图有效。

**实现**：

1. **切页搜索失效**：`cameraClick()` 从其他页回照片页时无条件 `loadPhotos()`，全量列表覆盖搜索结果（输入框还有字但结果没了）。新增 `hasActiveQuery()` / `refreshPhotosView()`——有查询时改跑 `runSearch()` 恢复结果；`btnGalleryView`、`appMenuViewGallery` 同步接入。废片页同类问题用 `refreshRejectsView()`（有废片搜索词时跑 `runRejectSearch`）。
2. **RAW 支持**：
   - `utils::RAW_EXTS`（nef/nrw/pef/ptx/arw/srf/sr2/crw/cr2/cr3/dng/raf/orf/rw2/raw/srw）+ `is_raw_ext/is_raw_path`；scanner `is_allowed` 并入该表。
   - `utils::decode_image(path)`：先 `image::open`，失败且是 RAW 时字节扫描内嵌 JPEG 预览（SOI FF D8 FF → 下一个 EOI FF D9 为一段，逐段解码取面积最大者；误报段解码失败即跳过，跨品牌通用，免逐格式解析 IFD）。缩略图 `generate_thumbnail` 与 AI `preprocess_image` 统一走它——RAW 获得缩略图、嵌入与语义搜索（内容=预览 JPEG）。
   - EXIF：kamadak-exif 对 TIFF 系 RAW（NEF/ARW/CR2/DNG…）可直接读镜头/焦距；CRW 等非 TIFF 容器优雅降级为无 EXIF（既有行为）。
3. **不显示重复 RAW**：
   - 视图菜单分割线（`.titlebar__menu-sep`）+ 勾选项（`.titlebar__menu-check`，✓ 用 accent 色），状态存 `localStorage("tiol-hide-dup-raw")`。
   - 纯前端筛选 `filterDupRaws(list)`：按 `路径目录/去扩展名`（归一化斜杠+小写，路径是 display 混合斜杠）分组，组内同时有 JPEG（jpg/jpeg）与 RAW 侧车时隐藏 RAW 的 id；`loadPhotos` / `runSearch scoped()` / `loadRejects` / `runRejectSearch` / `loadDuplicates`（组内过滤，空组丢弃）五处接入，搜索结果同样生效。切换勾选只重渲当前可见视图，其余视图下次进入自然生效。
   - i18n `menu.hideDupRaw`（中英）。

## C-19.20 · 2025-08 — 相机族视图统一（gallery 图标 / 废片重复迁移）/ 视图菜单 / 工具栏分组

**需求**：① 废片、重复照片与照片同属「相机」界面：相机图标在三个视图下保持高亮（蓝条不消失），点击侧栏相机只刷新当前视图不切页；② iconbar 顶部新增 gallery-view（四宫格）图标回到正常视图（默认亮），三个视图图标相邻，分隔线下是工具；③ 顶部菜单新增「视图」（文件与帮助之间）：照片/废片/重复照片，当前项高亮，点击即切换；④ 工具栏按钮紧凑；⑤ 图标确认全部内联（icns 不上传 GitHub 不影响构建）；⑥ 预览显示分辨率；⑦ 重复卡片路径对混合斜杠（`e:/img\xx.jpg`）显示修正。

**实现**：

1. **相机族语义**：`switchView` 的 `navPhotos` 高亮条件 = `isPhotos || isRejects || isDup`；`updateSidebarIndicator` 无 active nav 时回退定位到照片按钮。侧栏相机 = `cameraClick()`（族内刷新当前视图，族外切回照片页）；iconbar gallery = 无条件回正常页（两者语义分离）。boot 补 `btnGalleryView` scoped 亮起（默认视图）。
2. **iconbar 分组与紧凑**：顺序 gallery → 废片 → 重复 | `.iconbar__sep` 分隔线 | tree/tag/colors/eraser；按钮 30px/间距 2px/分隔线 margin 2px。废片入口自 sidebar 迁入 iconbar（`btnRejectsView`，侧栏导航由 5 减为 4，`els.navRejects` 置 null 并判空）。视图激活图标 `iconbar__btn--scoped` 蓝色（gallery/rejects/dups 三键随 `switchView` 同步）。
3. **视图菜单**：`btn-app-menu-view`（菜单互斥关闭）；三项点击切换对应视图（photos=常规回页、rejects=加载+条件重渲+分析、dups=重扫）；菜单打开时当前视图项 `titlebar__menu-item--active`（accent 蓝+加粗）；i18n `menu.view`。
4. **拖选/删除迁移**（C-19.19 延续）：`gridCards(grid)` 统一取卡（dup 卡片嵌 `.dup-group` 行内用 `querySelectorAll`）；`updateSelectionBar` 删除按钮在废片或重复页显示，delete handler 按视图重扫；dup 卡片高度 172px 含目录路径行（取最后 `/` 或 `\` 分隔符）。
5. **预览分辨率**：full 图 `onload` 后用 `naturalWidth/Height` 追加「分辨率：W×H px」行（零后端改动），i18n `preview.resolution`。
6. **图标内联确认**：sidebar/iconbar 全部内联 SVG；`src/` 无 `icns/` 引用；标题栏图标用仓库内 `src/icon.png`——icns 目录不入库不影响 CI 构建与图标显示。
7. **侧栏宽度可拖拽（跟进）**：把手做成 sidepanel 与 main 之间的**独立 flex 拖拽条**（`.panel-resizer`，面板打开时显示、hover 变蓝）——首版把手放在面板（overflow 容器）内部悬出不可点，已废弃；拖动实时改 `--sp-w` 变量（120–560px），拖动中禁过渡（`.sidepanel--resizing`），宽度 localStorage 持久化（`tiol-sidepanel-w`），开关面板/切页时同步显示隐藏。
8. **死代码清理（文档/检查）**：`pagePhotoSub`（未定义引用）、`get_all_file_briefs`（被 `get_all_files_full` 取代）、`DupFile` 序列化结构（改返 FileRecord）、启动填充诊断日志（fill/watchdog reportJs）、`sidepanel__resize`/`sideResize`（拖拽把手旧方案）、`galleryClick` 旧名、`bannerSlide` keyframes 等均无残留；i18n 181 keys 双语对齐、messages.js 同步、`getElementById` 145 个引用与 HTML 一一对应（唯一 missing 的 `onboarding-root` 为引导流程动态创建，非死代码）；cargo 无代码级 warning（仅 exFAT 硬链接环境提示）。

## C-19.19 · 2025-08 — 废片入口移入工具栏 / 重复视图工具化（蓝色提醒 + 拖选 + 删除 + 树联动刷新）

**需求**：① 重复视图是"工具视图"，进入后 copy 图标变蓝提醒（同树图标 scoped 样式）；② 在重复视图里点文件夹树选文件夹不刷新（需重新点重复图标）——应自动刷新；③ 重复视图多选支持拖选；④ 重复视图多选要有废片页的「删除照片」；⑤ 废片入口从左侧 sidebar 移到 iconbar（copy 图标下方，全部工具集中一处）——注意动画与逻辑适配；⑥ 更新 todo。

**实现**：

1. **激活提醒**：`switchView` 里 `btnDupView`/`btnRejectsView` 按当前视图 toggle `iconbar__btn--scoped`（accent 蓝，声明在 `--active` 之后覆盖）——用户在重复/废片页时工具栏图标呈蓝。
2. **树联动刷新**：树面板「全部」与节点点击后，若 `view-duplicates` 可见则 `loadDuplicates()`（作用域变化即时生效）；`scan-complete` 后同样刷新可见的重复视图；语言切换分支补 `renderDupGroups()`。
3. **拖选**：`onGridMouseDown` 的卡片遍历抽象为 `gridCards(grid)`——普通网格取 `children`，`dup-grid` 用 `querySelectorAll(".card")`（卡片嵌在 `.dup-group` 行内，原遍历会漏）；`dupGrid` 绑定 `mousedown`。live-highlight / 落点选中 / 清除共用同一辅助。
4. **删除照片**：`updateSelectionBar` 的「删除文件」显示条件由「仅废片页」改为「废片页或重复页」；delete handler 在重复视图走 `loadDuplicates()` 重扫重渲（组内删光则该行消失）。
5. **废片入口迁移**：sidebar 移除 nav-rejects（五按钮 → 四），iconbar 底部新增 trash 按钮（copy 下方，复用 trash-alt.svg）；原 nav click 逻辑原样搬到 `btnRejectsView`（switchView + loadRejects + renderRejectConds + ensureRejectAnalysis）。`els.navRejects` 置 null，switchView 对其判空；sidebar 指示条在无 nav 激活（rejects/duplicates 页）时 `opacity:0` 隐藏（`updateSidebarIndicator`）。
6. **i18n**：无新增 key（trash 复用 `nav.rejects` title）。

## C-19.18 · 2025-08 — 重复照片视图（像素级检测）

**需求**：iconbar 新增 copy 图标进入「重复照片」视图：原理 = 直接对比图片像素，一模一样者为重复；一行一组连续照片（过多溢出到下一行）；卡片不能打星/编辑标签（高度相应缩小）；多选照常；顶部保留筛选/星数/多选按钮；文件夹树选中时只在该文件夹内检测；筛选后按行显示、无照片的行隐藏；卡片底部显示路径便于抉择。

**实现**：

1. **后端 `find_duplicates(scope)`**：遍历**全库**（新增 `db.get_all_files_full`，`SELECT FILE_COLS` 无 LIMIT，返回完整 FileRecord——含 folder_id/colors/rating/lens 等，供筛选与路径展示）→ 对每张图的**缩略图文件做 SHA-256**。技巧：缩略图编码确定性 ⇒ 像素相同的图片（含重编码/不同格式/不同文件大小）缩略图字节必然一致，读取小文件毫秒级完成；缺失缩略图现场生成。`scope`（`{folder_id, path}`）镜像树面板的 `folderScope`——只对作用域内文件分组比较。
2. **视图**：新 `view-duplicates` section（滚动容器机制同 photos/rejects，`currentGrid=dupGrid`）；数据扁平后写入 `currentPhotos` 使 Ctrl+A/多选簿记照常。顶部 = 状态 bubble + actions bubble（筛选/星数/多选）——筛选/星数打开**共享面板**，变更走 `refreshCurrentView()`（新增 dup 分支：`renderDupGroups`）；星数按钮高亮同步。
3. **分组布局**：每组一个 `.dup-group`（flex-wrap 行），组间 `margin-top` 分隔；`renderDupGroups` 先对组内逐张 `applyFilters`（颜色/镜头/焦段/星数），**整组过滤光则整行不渲染**。
4. **瘦身卡片**：`.card--dup`——无星行/无编辑按钮，高度 172px；meta 两行：文件名 + **目录路径**（去掉文件名的 path，短格式 + title 全文），选重复副本时一眼可辨。
5. **多选**：视图自带「多选」按钮走全局 `setSelectMode`（按钮文本/高亮、`selection-bar` 复用）；`setSelectMode` 的卡片遍历改为 `querySelectorAll(".card")`（dup 卡片嵌在 `.dup-group` 内，原 `children` 遍历会漏）；Ctrl+A 在 dup 页同样全选。离开视图自动退出多选（switchView `leavingGrid` 纳入 duplicates）。
6. **刷新**：每次进入视图重新 `find_duplicates`（毫秒级）；文件夹树选定新文件夹后进入 = 自动作用域过滤（`loadDuplicates` 传 `folderScope`）。
7. **其他 handler 统一**：色点/镜头/焦段/清除筛选的渲染调用改为 `refreshCurrentView()`——photos/rejects/duplicates 三视图行为一致。
8. **i18n**：`iconbar.duplicates` + `duplicates.analyzing/summary/none/error`。

## C-19.17 · 2025-08 — 文件夹树面板视觉优化（选中背景 margin / 三角位置 / 省略号 / 叶子照片数 pill）

**需求**：① 选中项背景顶到面板两边，加左右 margin；② 折叠三角形位置略偏下；③ 过长文件名要省略号；④ 叶子文件夹右侧用 pill 显示照片数量。

**实现**：

1. **选中背景 margin**（styles.css）：`.sidepanel__item` 由 `width:100%` 改 `width:auto` + `margin: 0 6px 2px`——选中/hover 的胶囊背景不再顶到面板边缘；tags/colors 面板同步受益（同一基类）。
2. **三角形**（styles.css）：`.sidepanel__tri` 改 flex 居中 + `padding-bottom: 3px`——「▶」字形基线偏低，整体上移一点；`rotate(90deg)` 展开动画不受影响（transform 未占用）。
3. **省略号**：名称包进 `.sidepanel__label` span（`flex:1; min-width:0; ellipsis`）——原来文字直接挂在 flex 容器上，`text-overflow` 不生效。
4. **照片数 pill**：后端 `FolderNode` 新增 `count` 字段——`get_folder_tree` 每个根目录一次性取全部文件路径（`db.get_folder_paths`），`fill_leaf_counts` 按前缀在内存计数（避免每节点一条 SQL LIKE）；前端 `count > 0` 时渲染 `.sidepanel__count` pill（title 复用 `folders.count` i18n）。数量随树缓存生命周期更新（markTreeDirty 失效后重取）。
5. **跟进修复（同日）**：① pill 恒为 0——`files.path` 存的是归一化 key（小写+正斜杠，ADD.md §9.1），而节点是显示路径，前缀永不匹配；`fill_leaf_counts` 先 `normalize_storage_path` 归一化节点路径再补 `/`（补必须在归一化之后：它会剥尾部分隔符，裸前缀会误匹配 `e:/img2`）。② `.sidepanel` 加 `padding: 8px 0`，「全部」不再贴顶。③ 树节点缩进 14px/级 → 10px/级。
6. **跟进二（同日）**：① 选中态更明显——`.sidepanel__item--active` 由灰底改为 accent 16% 半透明底（`color-mix`，`bg-elevated` 作不支持时的回退）+ accent 文字 + 600 字重，暗色（#0A84FF）/亮色（#007AFF）下均清晰。② 树筛选生效时（`folderScope != null`，含面板关闭后——筛选在面板外仍生效）树图标呈 accent 蓝：新增 `.iconbar__btn--scoped`（声明在 `--active` 之后以覆盖其 color），`syncTreeIcon()` 在「全部」与节点点击两处同步。③ 根节点 paddingLeft 基数 6px → 0（根节点行本身已有 16px 三角列，「全部」到第一层的视觉跳跃过长）；层级步进仍 10px。
7. **跟进三（同日）**：外层文件夹也显示数量——`fill_leaf_counts` 去掉「仅叶子」分支，所有节点统一按前缀计数，即**该节点（含子目录）的照片总数**，与点击节点的筛选结果一致（pill 数字 = 点击后网格照片数）；前端去掉 `!hasKids` 条件。

## C-19.16 · 2025-08 — 文件夹树缓存失效（修复添加新文件夹后树不显示）

**需求**：添加新文件夹后，左侧文件夹树面板不显示新文件夹（重启应用才出现）；删除的文件夹同理残留。

**根因**：树数据缓存 `treeCache`（app.js）只在树面板首次打开时经 `get_folder_tree` 填充，之后永不失效——面板再次打开时直接渲染旧缓存。后端 `get_folder_tree` 每次实时扫盘，数据本身无误，纯前端缓存问题。

**实现**：

1. **`markTreeDirty()`**：置 `treeCache = null`；若树面板当前打开（`sideMode === "tree"`）则立即 `renderSidePanel("tree")` 原地刷新（展开状态由 `treeExpanded` 保留，与点击节点后的重建路径相同）。
2. **失效点**：① `add_folder` 成功后；② `remove_folder` 成功后；③ `scan-complete` 事件（扫描可能发现缓存建立后才出现的子目录）。
3. **代价**：失效后下次打开面板会重新 `get_folder_tree`（一次目录遍历）；打开面板为低频操作，可接受。`treeCache` 原注释「per session」相应更新。

## C-19.15 · 2025-08 — 帮助菜单 / 图标栏 / 侧栏快捷面板 / 懒标签 / 橡皮擦 / 全局右键禁用 / Ctrl+A / 文件菜单导入

**需求**（多项小需求合并为一次大迭代）：① 顶部菜单新增「帮助 → 查看 GitHub 页面」；② 左侧新增窄图标栏（目录树/标签/多彩颜色图标），点击顺滑弹出左侧面板（挤压主区成双栏）；③ 面板内容：tag 单选点击即打、颜色单选点击即上色（含废片）、文件夹树（默认折叠、三角形展开、仅显示目录）；④ 图标栏只在照片/废片页出现；⑤ eraser 橡皮擦模式（点击清除全部标签与颜色，不弹面板）；⑥ Tags 页标签可勾选，AI Tagging 只处理勾选项；⑦ 全局禁用浏览器右键菜单（仅保留卡片自定义菜单）；⑧ Ctrl+A 全选当前视图；⑨ 文件菜单「导入文件夹」。

**实现**：

1. **帮助菜单**：标题栏「帮助 → 查看 GitHub 页面」经 `shell.open` 打开仓库 `sclass53/TIOL-Image-Manager`。
2. **图标栏（iconbar，36→30px）**：sidebar 与主区之间，folder-tree/tag/彩色环（conic-gradient）/eraser/copy 内联 SVG；**仅网格页显示**（`switchView` toggle `iconbar--hidden`，宽度+透明度过渡）；按钮高亮表示激活模式/面板。
3. **侧栏（sidepanel）**：面板在 main **左侧**滑出（width 过渡挤压主区）；`toggleSidePanel` 语义 = 点新图标**切换**面板（高亮转移）、点同一图标关闭；tags/colors/tree 各模式状态独立。
4. **懒打标（不重渲染）**：tag/颜色点击走 `updateCardInPlace`——就地更新标签文字行（必须 append 进 `.card__meta` 内，卡片固定高度裁剪外部元素）、色点、reject 徽章；照片因新状态掉出当前视图（废片条件/筛选）时**就地移除卡片**。颜色 toggle 语义：`toggle_color_tag` 返回操作后是否携带该色（all=true → 添加）。
5. **文件夹树**：后端 `get_folder_tree`（递归列子目录、跳过文件/隐藏目录、深度≤12、每节点带 root_id 与 count）；前端全部节点默认折叠，三角形 `▶ rotate(90deg)` 展开（**就地 toggle kids 容器，避免重建面板丢失过渡起点**）；选中节点 → `folderScope = {rootId, path}`：照片按根目录拉取 + **path 前缀过滤**（子目录同样可筛），搜索/筛选基于此作用域；树数据 session 缓存 + `markTreeDirty()` 失效。
6. **橡皮擦**：不弹面板；点击照片 → `clear_tags_from_files`（tags+colors）懒清除。
7. **AI Tagging 勾选**：Tags 页每标签勾选框（`selectedTagIds`）；`run_ai_tagging(tag_ids)` 后端把任务限定到勾选标签名（AITask.tag_names 过滤匹配向量）；未勾选提示。
8. **右键禁用**：全局 `contextmenu` preventDefault（输入框的复制粘贴菜单也禁用）；卡片右键仍弹自定义菜单。
9. **Ctrl+A**：任意视图进入多选并全选 `currentPhotos`（输入框除外，macOS Cmd+A）。
10. **文件菜单导入**：`importFolderFlow()` 与目录页按钮共用（dialog → `add_folder` → `markFoldersDirty`/`markTreeDirty` → 刷新列表）。
11. **i18n**：`menu.help/viewGithub/import`、`iconbar.*`、`sidepanel.*`、`tags.selectFirst` 等。
12. **动画开关扩展**：三角形旋转、侧栏滑动、图标栏与主 sidebar 的 width 过渡统一受「特效 > 动画」控制（`fx-anim-off`）。

## C-19.14 · 2025-08 — 搜索栏 bubble 化 + 星数多选筛选 + 全屏按钮 SVG

**需求**：① 最大化按钮偏小（字形差异）；② 顶部搜索栏拆成若干圆角 bubble：文件名搜索、语义/标签+搜索、筛选+星数+多选（带阴影，可开关）；③ 星数筛选改成 0~5 星各自勾选（可同时筛 1 星和 3 星）。

**实现**：

1. **全屏按钮**：`□`/`❐` 字形在 Segoe UI 渲染偏小——改为 inline SVG（13px 方框/双框），`updateWinMaxBtn` 按窗口状态切换单框/双框图标。
2. **搜索栏 bubble 化**（photos 页）：`.searchbar__bubble` 圆角 pill（radius 20px，`--bg-surface` 底 + 边框 + 阴影），三组：文件名 | 语义/标签模式+输入 | 筛选+星数+多选；bubble 内 input/select 背景透明。阴影挂 `fx-shadow-off`（设置里「特效 > 阴影」开关控制），浅色主题有对应弱阴影；`.searchbar` 原 border-bottom/阴影移除。
3. **星数多选**：photos/rejects 两个「星数」按钮共用同一面板（复用 `.color-filter` 样式与 fxIn 动画），面板内 0~5 星各一个勾选框（金色 ★×n，0=「无星」）+ 清除按钮；`activeRatings` Set 替代原 `minRating`（`{1,3}` = 1 星和 3 星照片都保留，无星照片需勾选 0 才显示）。空集 = 不过滤；按钮勾选实时重渲染当前视图（废片页仍叠加废片条件）。
4. **i18n**：`photos.ratingBtn / ratingNone`；删除已无引用的 `ratingAll / rating1..5`（原 select 移除）。

## C-19.13 · 2025-08 — 无边框窗口 + 内嵌标题栏（Windows/Linux）

**需求**：去掉 Windows 默认窗口标题栏，最小化/关闭等按钮直接嵌入窗口内部。

**实现**：

1. **无边框窗口**：`main.rs` 窗口构建时按平台覆盖 `decorations`——Windows/Linux `false`（配合前端自绘标题栏），macOS 保持 `true`（保留原生红绿灯，自绘交通灯工作量与收益不成比例）。
2. **内嵌标题栏**（`.titlebar`，36px 全宽）：左侧 `data-tauri-drag-region` 拖拽区（拖动移动窗口、双击最大化，Tauri 内置）+ 标题文字；右侧三个按钮：最小化（`─`）、最大化/还原（`□`/`❐`，窗口 resize 时同步图标与 i18n tooltip）、关闭（`✕`，hover 红色 `#e81123`）。
3. **窗口控制**：`window.__TAURI__.window.getCurrentWindow()`（withGlobalTauri，无新增依赖）——`minimize()/toggleMaximize()/close()`，`onResized` 监听同步最大化按钮状态。
4. **平台适配**：`navigator.userAgent` 检测 macOS → `body.platform-mac` 隐藏标题栏（CSS）。
5. **布局**：body 改纵向 flex，`.titlebar` 固定 36px，`.app` 弹性占满剩余空间。
6. **权限**：窗口控制属 `core:default`（capabilities 已有），无需新增。
7. **i18n**：`titlebar.minimize/maximize/restore/close`（中英同步）。

## C-19.12 · 2025-08 — 入场动画：淡入 + 略微下滑（可开关）

**需求**：每个图片 card、多选 pill、各类弹出式菜单出现时有动画——淡入且"略微向下滑动"（常见 web UI 入场效果），并复用设置里已有的「动画」开关控制。

**实现**：

1. **关键帧**：`fxIn`（无静态 transform 的元素：淡入 + `translateY(-8px → 0)`）、`fxInCenter`（保留 `translateX(-50%)` 居中的元素）、`fxFade`（遮罩纯淡入）。动画结束后回到静态样式，无跳变。
2. **覆盖范围**：照片卡片（0.3s + 组内 stagger `--i`，每张 8ms、上限 20 步）、多选 pill（主栏/次栏）、筛选面板、废片条件面板、对话框、对话框遮罩、右键菜单（0.15s 快）、预览面板、toast、更新横幅、打标徽章。
3. **stagger**：`renderChunk` 为每张卡设 `--i`（组内相对索引，滚动追加的批次从 0 重新计数），CSS `animation-delay: calc(min(var(--i),20) * 8ms)` 上限 160ms——首屏批量渲染呈瀑布式出现，滚动加载的批次同样生效。
4. **开关**：全部动画挂在 `body.fx-anim-off` 下（`animation: none`），即设置页「特效 > 动画」开关直接控制；update-banner 原有的 `bannerSlide` 动画被 `fxInCenter` 取代（原为纯下滑、无淡入，且与开关无关）。
5. **兼容**：`content-visibility: auto` 的离屏卡片不播放动画（时间推进、进入视口即终态），不影响滚动性能；卡片动画用 `fill: both` 保证延迟期间占位透明，布局/滚动高度计算不受影响。

## C-19.11 · 2025-08 — 设置页特效开关（动画 / 阴影）+ 多选 pill 跨页修复

**需求**：① 设置里 language 和主题拆成两行；② 下方新增一行「特效」，可开关「动画」和「阴影」（一行两个开关），默认开启；③ 废片页多选不显示底部 pill；④ 主页开多选后切到废片页，第一次点击"多选"是退出而非进入——切页时应退出多选；⑤ 废片页删除照片后一瞬间闪现全部照片；⑥ 废片删除后不应退出多选模式。

**实现**：

1. **布局修复**：language 与 theme 两个 `settings__row` 原本嵌套错乱（theme 嵌在 language 行内）——拆成两个独立行。
2. **特效行**：`settings__fx` 一行两个开关「动画」「阴影」，按钮文本「动画：开/关」「阴影：开/关」，点击切换 + `btn--active` 高亮。
3. **阴影开关**（`body.fx-shadow-off`）：关闭顶部搜索栏、照片卡片、多选栏、筛选面板、toast、更新横幅、打标徽章、新手气泡的 box-shadow。搜索栏/卡片此前无阴影，默认补上细微阴影（深色 `0 2px 8px` / `0 1px 4px`，浅色更淡），让开关有实际效果。
4. **动画开关**（`body.fx-anim-off`）：关闭侧边栏指示条滑动动画；未来新增动画统一挂到该类名下。
5. **持久化**：localStorage（`tiol-fx-anim` / `tiol-fx-shadow`，与主题一致），缺省即开启；boot 时应用，设置页 `renderSettings` 刷新状态显示。
6. **多选 pill 跨页修复（根因）**：两个 selection-bar 原本嵌在 `view-photos` section 内部——切到废片页时该 section `display:none`，fixed 定位的 pill 随祖先隐藏，废片页永远看不到。已移到 body 层级（main 之外），任何视图下都显示。
7. **切页退出多选**：`switchView` 记录切换前的可见视图，从照片/废片页切走时 `setSelectMode(false)`（重复点击当前页导航不退出）。`setSelectMode` 同步照片页/废片页两个"多选"按钮的文本与高亮；退出时同时清理两个 grid 的 `selecting` 类与卡片勾选状态（switchView 可能已切换 currentGrid）。
8. **废片页删除闪烁（根因）**：`loadPhotos()` 直接渲染到 `currentGrid`——从废片页触发刷新时（删除/scan-complete），全量照片被画进废片网格，随后 `loadRejects()` 才覆盖，造成"所有照片闪现"。修复：`loadPhotos()` 固定临时切换渲染到 photoGrid；删除 handler 与 scan-complete 中 `loadPhotos`/`loadRejects` 改为 `await` 串行执行。
9. **删除后保持多选**：删除 handler 不再 `setSelectMode(false)`；从 `selectedIds` 移除已删除 id，重渲染后 `applySelectionToGrid(rejectGrid)` 重新标记选中卡片并刷新计数——可连续删除多批。
10. **顺手修复**：废片页星数筛选分支缺 `applyRejectConds`（会显示全部照片）——已补。
11. **启动渲染不全修复（根因）**：`loadPhotos` 渲染后注册的 `requestAnimationFrame(fillGridIfNeeded)` 可能赶在 webview **首次布局之前**执行——`clientHeight=0` 使填充循环立即退出，之后没有任何触发源，网格停在首屏几行且无滚动条（滚动也无法自救），必须切页（布局已就绪后 switchView 重新触发）才恢复。修复：① `fillGridIfNeeded` 退出时若 `clientHeight===0` 且未渲染完，逐帧自我重试直到布局就绪；② boot 后 300ms/1.5s 定时兜底调用（幂等）；③ 窗口 resize 也触发填充（窗口放大同样会欠填）。
12. **i18n**：`settings.effects / fxAnim / fxShadow`（中英同步），messages.js 重新生成。

## C-19.10 · 2025-08 — 侧边栏指示条动画 + 滚动位置保持 + 多选栏双 pill

**需求**：① 左侧蓝条上下移动动画；② 强制刷新（添加/删除 tag 等）把页面滚回最上面——修复；③ 右侧三个按钮排序为「删除标签」「导出」「取消」，并单独拉出来开一个 pill。

**实现**：

1. **侧边栏指示条动画**：新增 `.sidebar__indicator`（3px 竖条，绝对定位在 sidebar 内），`transform: translateY` + `transition .25s` 实现上下滑动；`updateSidebarIndicator()` 读取 `.sidebar__btn--active` 的 offsetTop+6 定位，在切换视图、语言切换时调用。启动时 boot 流程直接同步定位（关过渡防滑入动画）——启动路径不经过 switchView，否则蓝条会停在左上角（相机图标上方）。
2. **滚动位置保持**：`renderPhotos(photos, opts)` 默认保留 scrollTop；需要回顶的场景显式传 `{scrollTop: 0}`（语义搜索两个分支、切换文件夹 `loadPhotos(f.id, {scrollTop:0})`）。
   - **坑（第一版失效）**：先清空网格再设 scrollTop 会被浏览器钳制为 0——必须**先渲染内容、再恢复滚动**；初始 chunk 短于目标位置时循环 `scrollToViewport()` 补渲染并重施目标值，直到位置真正落定。
   - 深滚动恢复时缩略图入队只覆盖恢复位置附近的视口窗口（`offsetTop` 过滤），避免顶部几百张卡先占满队列、可见区域饿死。
   - 添加/删除标签、X 标签、评分、颜色标签等强制重渲染不再把页面弹回顶部。
3. **多选栏双 pill**：主栏（数量、色点、添加标签、评分、删除文件——仅废片页显示）保持底部居中；新增 `.selection-bar--secondary` 锚定底部右侧（`left:auto; right:72px`，不贴右缘/滚动条），按钮顺序「删除标签」「导出」「取消」。对话框打开时两栏一起隐藏（`setSelectionBarVisible`）。
4. **X 徽章防遮挡**：废片 X 徽章固定在卡片右上角（`.card__reject` 红圈白 X，`transition: top .15s`）；进入多选模式时下移 `top:32px`（`.grid.selecting .card__reject`），不覆盖勾选框。颜色切换/清除标签后强制重渲染，徽章实时刷新。
5. **导出 / 删除文件**：多选栏新增「导出」（复制到所选目录，重名自动加 " (n)" 后缀）与「删除文件」（仅废片页显示 + 确认对话框）——后端 `export_files` / `delete_files` + `db::delete_file`。
6. **设置图标**：齿轮改用 `⚙️`（VS16 变体），emoji 尺寸下显示更合适。

## C-19.9 · 2025-08 — 提示浮层动态定位 + 各页面筛选独立 + 面板夹紧

**实现**：

1. **提示浮层动态定位**：`showSelectionHint` 创建的 toast 实时测量底栏位置（`getBoundingClientRect`），定位在底栏正上方——任何底栏高度都不会被遮挡（取代之前固定 bottom 值）。
2. **筛选面板夹紧视口**：`positionPanel(panel, btn)` 面板超出窗口时自动夹紧回可视范围内（全屏/窗口缩小时不跑出屏幕）。
3. **拖选框 fixed 定位**：`.selection-box` 改为 `position:fixed`，用视口坐标绘制，滚动/全屏时不再与光标偏移。
4. **各页面筛选独立**：`clearSharedFilters` 切换页面时清掉不适用该页的筛选状态；`lastGridView` 记住照片页网格视图。

## C-19.8 · 2025-08 — EXIF 异步回填（镜头/焦距）

**需求**：扫描时提取 EXIF 导致添加文件夹明显变慢。

**实现**：扫描流程不再提取 EXIF（添加文件夹恢复速度）；新增 `spawn_exif_backfill(db)` 后台线程，在启动 / 添加文件夹 / 扫描完成 / 文件监控扫描后增量补填镜头、焦距等 EXIF 列（只处理未提取的文件）；`exif_columns_roundtrip` 测试改为断言文件保持"待回填"状态。

## C-19.7 · 2025-08 — 启动首屏填充修复 + 首次安装新手教程

**需求**：① 启动时只显示 20+ 张照片，切换页面后才有全部（bug）；② 首次安装显示分步新手教程（仅第一次，更新不再显示）：1 左下角箭头（展开/收起菜单）→ 2 文件夹图标 → 3 添加文件夹 → 4 主页面按钮引导点击 → 5 右上角进度条（等待后可搜索）→ 6 垃圾桶（废片筛选）结束。

**实现**：

1. **首屏填充修复**：`fillGridIfNeeded`（视口填充循环）此前只在切换视图时触发——启动时 `loadPhotos()` 只渲染首屏 5 行后无人填充。修复：`loadPhotos()`/`loadRejects()` 渲染后各加 `requestAnimationFrame(fillGridIfNeeded)`（函数内部有视图可见性检查，安全）。
2. **新手教程**（`onboarding` 状态机，全 JS 动态创建）：启动延迟 0.9s 检查 DB 设置 `onboarding_done`——**缺失才显示**（第一个带教程的版本对每个安装都显示；之后更新的用户有标记不再显示）。
   - 高亮框（脉冲动画，`pointer-events:none` 不挡操作）+ 引导气泡（文本 + 下一步/跳过；等待步骤只有跳过）+ 跳过即写标记。
   - 步骤驱动：S1/S2/S5/S6 用「下一步」（S2 下一步自动切到目录页）；S3 等待「+ 添加目录」**添加成功**后自动前进（`onboardingAfterAdd` 钩子）；S4 等待用户点击照片页签（`onboardingOnPhotosClicked` 钩子）；S5 高亮**右上角**进度徽章区域（徽章可能隐藏，用固定区域高亮）；S6 完成 → 写 `onboarding_done=1`。
3. **i18n**：`onboarding.s1..s6/next/done/skip`（中英同步），messages.js 重新生成。

## C-19.6 · 2025-08 — 提示 popup 化 / 评分刷新 / 单次分析

**需求**：① 底部栏提示文本挤压按钮——提示改为**底栏上方的 popup**（独立浮层，不再嵌入底栏）；② 批量评分后星数不即时更新；③ 曝光分析出现多段执行。

**实现**：

1. **提示 popup**：`showSelectionHint` 改为动态创建 `toast selection-toast`（`bottom:104px`，2.5s 自动消失），彻底脱离底栏 DOM；底栏内 hint 元素移除——任何长文本都不会再挤压按钮。
2. **评分刷新**：原地 DOM 更新不可靠 → 评分成功后**强制重渲染当前网格**（照片页/废片页各自数据源），新卡片直接读 `p.rating`；多选状态保持。
3. **单次分析**：后端曝光循环**容错化**——`catch_unwind` 包单文件分析 + 更新失败记 warn 继续（一次调用必处理完全部剩余，日志统计 `(N failed, treated as normal)`）；前端恢复**一次性触发标志**（启动预热 + 首次进废片页各一次），**scan-complete 重置标志**允许新照片补算——反复进出页面不再触发多段分析。


## C-19.3 · 2025-08 — 废片条件检测逻辑（过曝/欠曝/闭眼）

**需求**：烂片条件**默认全勾选**并接入真实筛选逻辑：① 过曝 = 亮度 252–255 像素占比 > 20%；② 欠曝 = 亮度 <20 占比 >30% **且** 最亮像素亮度 <150；③ 闭眼 = 语义搜索"closed eyes"相似度 > 0.11；④ 模糊暂不实现（勾选但不过滤）。

**实现**：

1. **曝光检测**（新模块 `rejects.rs`）：`analyze_exposure`——解码 → 缩放到 256px（全局属性足够，大图快）→ luma 直方图统计；过曝/欠曝按上述规则；解码失败计为"都不是"（不重试）。单测 4 项（纯白/纯黑/中灰/黑白混合含高光点）。
2. **DB 缓存（增量）**：files 加 `overexposed/underexposed/eyes_closed INTEGER`（NULL=未检测）；`FileRecord` + FILE_COLS 加 3 列（serde skip None）；`get_files_missing_exposure`（分页）/`count_files_missing_exposure`/`update_reject_exposure`/`update_reject_eyes`——**只处理 NULL 的行**，新照片下次自动补算。
3. **闭眼检测**（main.rs `compute_reject_metrics`）：复用已存 embedding——`embed_text("closed eyes")` 一次 + 全库向量余弦（1034 张 ≈ 秒级），>0.11 → eyes_closed=1；每次全量重算（快）。
4. **曝光分析**：`spawn_blocking` 分页处理（每页 50），每 20 张发 `reject-analysis-progress {done,total}`，完成发 `reject-analysis-complete`——UI 状态栏显示"正在分析照片… N/M"，完成自动刷新列表。
5. **前端**：`activeRejectConds` 默认 `{blur,under,over,eyes}` 全勾；`applyRejectConds`（条件间**并集**，与其他筛选**交集**；条件激活而指标为 NULL 的照片不显示——C-15 规则）；`ensureRejectAnalysis` 每次会话只触发一次（进入废片页/勾选条件时），后端增量幂等；空结果显示"没有符合筛选条件的照片"。
6. i18n：`rejects.analyzing`（中英同步）。

**行为对照**：进入废片页 → 状态栏"正在分析照片…"（首次，含解码约 1–2 分钟）→ 完成后列表自动刷新为满足已勾选条件的照片 ✓；默认 4 项全勾（模糊不生效）✓；取消勾选条件即时重筛 ✓；新加照片再次进入自动补算 ✓。

## C-19 · 2025-08 — 废片筛选页面 + 多选批量评分

**需求**：① 新增第 5 页签「废片」（类似主页的照片网格）：搜索栏**去掉文件名搜索与 semantic/tag 模式下拉**，原下拉位置改为「废片条件」按钮（下拉面板可勾选 模糊/欠曝/过曝/闭眼——**仅 UI 状态，检测逻辑后续实现**）；保留 筛选（颜色/镜头/焦段）、内容搜索（语义）、星数筛选、多选。② 两个页面（照片/废片）的多选都支持**一键应用星数**（点击出星数选择框）；「删除标签」（一键清空）**不清星数**（现有实现已满足）。

**实现**：

1. **页面结构**：侧栏第 5 按钮 🗑️（nav-rejects，位于标签与设置之间）；`view-rejects` 搜索栏 = `[筛选][废片条件][星数下拉][内容搜索输入框][多选]` + `#reject-grid` 网格 + 状态栏；废片条件面板（`#reject-cond-panel`，仿 color-filter 下拉，4 个复选框 + 清除）；星数选择对话框 `#rate-overlay`（5 个大号 ★ 按钮）。
2. **渲染管线参数化**（关键重构）：`currentGrid`（photo-grid / reject-grid 二选一）——渲染、分块填充、滚动、缩略图、框选全部改操作当前网格；框选与滚动 handler 同时绑定两个网格（`e.currentTarget`）；缩略图 IntersectionObserver 的 root 由 photo-grid 改为 **viewport（null）**，两网格通用；`switchView` 增加 rejects 分支并切换 currentGrid；离开照片类视图退出多选。
3. **废片页数据与搜索**：`loadRejects()`（全量照片经共享 `applyFilters` 渲染到 reject-grid，颜色/镜头/焦段/星数筛选两页共享）；`#reject-search-input` 500ms 防抖 → 语义搜索（无 mode 下拉，固定 semantic）；星数下拉与主页**共享 minRating 并双向同步**；「筛选」按钮两页共用同一全局面板（按点击按钮定位）。
4. **废片条件（UI-only）**：勾选状态存 `activeRejectConds`，按钮高亮、可清除——**不改变照片列表**（检测逻辑留给后续版本）。
5. **批量评分**：后端新命令 `set_rating_files(file_ids, rating)`（校验 0–5，循环调用现有 db.set_rating）；selection-bar 新增「评分」按钮 → 星数选择框 → 应用后选中卡片 `p.rating` 原地刷新（`renderCardStars`）+ 提示「已为 N 张照片设置 X 星」，保持多选；两页共用（selection-bar 全局）。
6. **i18n**：`nav.rejects`、`rejects.cond/blur/under/over/eyes/clear`、`photos.rateSelected/rateTitle/rated`（中英同步），messages.js 重新生成。
7. 无 schema 变更（rating 字段 C-17 已有）。

**行为对照**：侧栏 5 页签切换正常 ✓；废片页无文件名框/无模式下拉，布局为 筛选|废片条件|星数|内容搜索|多选 ✓；废片条件勾选可交互但不影响结果 ✓；两页多选 → 评分 → 批量星数即时生效 ✓；删除标签后星数保留 ✓；星数下拉两页同步 ✓。

## C-18 · 2025-08 — 基于 SHA256 的自我更新检测

**需求**：工作流 push 到发布仓库（TIOL-site → tiol.netlify.app）时，在 version.json 中记录 Windows/macOS 可执行文件的 SHA256；本地启动时计算**自身 current_exe 的 SHA256** 与远程比对，实现检测更新——**更新判断完全不依赖烧录进 exe 的版本号**（tauri version 保持 0.1.0 不动），版本号仅用于展示。

**实现**：

1. **version.json 新格式**（`MNFilm-Industry/TIOL-site` 根目录，Netlify 托管）：在原有 version/tag/updated 基础上新增 `platforms.{windows,macos}.{sha256,url}`——sha256 为对应平台可执行文件哈希，url 为 Netlify 固定下载直链（`https://tiol.netlify.app/releases/latest/TIOL-{win64-portable,macos-arm64}.zip`）。
2. **CI**（build.yml `push-to-site-repo` job，替换原 version.json 步骤）：Windows 用 `unzip -p …/TIOL-win64-portable.zip tiol.exe | sha256sum` 流式计算 zip 内 exe 的哈希；macOS 取 `.app/Contents/MacOS` 下**非 dylib 的可执行文件**（通配符查找）计算；两平台都空才报错（单平台缺失留空，不阻塞）；bash heredoc 生成 JSON 后走原有 commit/push。
3. **后端**（新模块 `update.rs`）：`sha256_hex`（1MiB 分块流式计算，依赖已有 sha2/hex）；纯函数 `evaluate_update(remote_json, local_sha, platform)`（可单测：匹配→无更新、不匹配→有更新+版本/URL、缺平台字段/坏 JSON/空 sha→无更新）；命令 `check_update()`：**debug 构建直接跳过**（dev exe 哈希永不匹配）、`current_exe()` 哈希、reqwest GET version.json（**5 秒超时**），任何失败静默返回"无更新"（log::debug，绝不打扰用户）。
4. **前端**：启动后延迟 2 秒自动检查一次；有新版本 → 顶部**更新横幅**（slide-down 动画：「发现新版本 vX」+「下载更新」（`shell.open` 打开 Netlify 直链，capability 已有 `shell:allow-open`）+「稍后」）；设置页新增「更新/检查更新」行（手动触发，无更新 toast「已是最新版本」，离线轻提示）。不轮询、不常驻。
5. **i18n**：`update.label/check/available/download/later/upToDate/offline`（中英同步），messages.js 重新生成。
6. **单测**：`sha256_hex` 已知向量（sha256("hello")=2cf24dba…）+ `evaluate_update` 四种情形，预计 18 项全过。

**行为对照**：发布 v0.1.7 → version.json 含真实哈希 → 旧版用户启动 2 秒后见横幅 → 点下载打开直链 ✓；运行最新版无横幅 ✓；断网/离线/dev 模式均静默 ✓；版本号判断不依赖 exe 内烧录值 ✓。

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
