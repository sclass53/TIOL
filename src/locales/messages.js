// AUTO-GENERATED from locales/*.json — edit the JSON files, then run
// `node scripts/gen-messages.js` to refresh this module (C-11.11).
export const MESSAGES = {
  "zh-CN": {
    "app": {
      "title": "TIOL - AI 本地照片管理"
    },
    "nav": {
      "photos": "照片",
      "folders": "目录",
      "tags": "标签",
      "settings": "设置"
    },
    "search": {
      "name": {
        "placeholder": "搜索文件名"
      },
      "semantic": {
        "placeholder": "用一句话描述要找的照片… 例如：一张日落的照片",
        "error": "语义搜索失败",
        "unavailable": "AI 尚未就绪，无法语义搜索"
      },
      "tag": {
        "error": "标签搜索失败"
      },
      "mode": {
        "semantic": "语义搜索",
        "tag": "标签搜索"
      }
    },
    "photos": {
      "empty": "暂无照片 — 请添加目录或尝试搜索",
      "selectMode": "多选",
      "selectDone": "完成",
      "selectedCount": "已选 {count} 张",
      "addTagSelected": "添加标签",
      "tagsAdded": "已为 {count} 张照片添加「{tag}」",
      "filterColor": "颜色筛选",
      "filterBtn": "筛选",
      "filterClear": "清除",
      "filterEmpty": "没有符合颜色筛选的照片",
      "status": {
        "count": "{count} 张照片",
        "partial": "{shown} / {total} 张照片"
      }
    },
    "colors": {
      "red": "红色",
      "orange": "橙色",
      "yellow": "黄色",
      "green": "绿色",
      "blue": "蓝色",
      "purple": "紫色"
    },
    "folders": {
      "empty": "尚未添加目录",
      "add": "+ 添加目录",
      "refresh": "↻ 刷新",
      "remove": "移除",
      "count": "{count} 张",
      "status": {
        "count": "{count} 个文件夹"
      }
    },
    "card": {
      "edit": {
        "title": "编辑标签",
        "current": "当前标签",
        "suggest": "从已有标签添加",
        "noTags": "暂无标签",
        "noSuggest": "没有更多可添加的标签 — 可在下方输入新标签",
        "remove": "移除标签",
        "placeholder": "输入新标签名称后按回车",
        "save": "保存",
        "cancel": "取消"
      }
    },
    "menu": {
      "reveal": "在文件资源管理器中显示"
    },
    "preview": {
      "close": "关闭预览",
      "error": "无法预览此文件"
    },
    "dialog": {
      "confirmTitle": "确认操作",
      "ok": "确定",
      "cancel": "取消"
    },
    "tagging": {
      "badge": "正在标记中",
      "indexing": "正在索引",
      "remaining": "剩余 {count} 张",
      "indexingRemaining": "剩余 {count} 张"
    },
    "tags": {
      "title": "自定义标签",
      "hint": "点击「AI 标记」按全部当前标签为照片打标——新增标签与新增照片都会包含。",
      "pickTitle": "为选中的照片添加标签",
      "pickSearch": "搜索标签…",
      "pickNoMatch": "没有匹配的标签",
      "pickEmpty": "尚未定义标签 — 请先在标签页添加",
      "tagPlaceholder": "标签名称，如：飞机 / sunset / 长曝光",
      "tagThreshold": "匹配阈值",
      "tagCount": "{count} 张",
      "addTag": "+ 添加",
      "removeTag": "删除",
      "empty": "尚未定义标签 — 添加标签后点击「AI 标记」开始打标",
      "nameRequired": "请输入标签名称",
      "runButton": "AI 标记",
      "runStarted": "已为 {count} 张照片排队打标",
      "runNoTags": "尚未定义标签 — 请先添加标签",
      "clearAll": "清除标记",
      "clearAllConfirm": "将删除所有标签定义和所有照片上的标签（包括手动标签）。此操作不可撤销，确定继续吗？"
    },
    "settings": {
      "title": "设置",
      "language": "语言",
      "languageZh": "中文",
      "languageEn": "English",
      "hwDecode": "硬件加速解码",
      "hwDecodeHint": "更改后需重启应用生效",
      "restart": "重启应用",
      "on": "开",
      "off": "关",
      "cacheLabel": "缩略图缓存",
      "clearCache": "清除缓存",
      "cacheCleared": "缓存已清除",
      "modelStatus": "AI 模型",
      "modelLocked": "已就绪",
      "modelDownloading": "正在下载模型",
      "modelError": "模型异常（AI 功能不可用）",
      "aiProgress": "处理中 {done}/{remaining}",
      "aiProvider": "AI 引擎",
      "aiAuto": "自动",
      "aiGpu": "GPU",
      "aiCpu": "CPU",
      "aiCoreml": "Apple CoreML",
      "debug": "调试模式",
      "gpu": "GPU 渲染器：{renderer}",
      "gpuSoftware": "（软件渲染 — 硬件加速未生效）",
      "gpuUnknown": "GPU 渲染器：无法检测"
    }
  },
  "en-US": {
    "app": {
      "title": "TIOL - AI Local Photo Manager"
    },
    "nav": {
      "photos": "Photos",
      "folders": "Folders",
      "tags": "Tags",
      "settings": "Settings"
    },
    "search": {
      "name": {
        "placeholder": "Search for filenames"
      },
      "semantic": {
        "placeholder": "Describe the photo you're looking for… e.g. a sunset",
        "error": "Semantic search failed",
        "unavailable": "AI not ready for semantic search"
      },
      "tag": {
        "error": "Tag search failed"
      },
      "mode": {
        "semantic": "Semantic",
        "tag": "Tag"
      }
    },
    "photos": {
      "empty": "No photos — add a folder or try searching",
      "selectMode": "Select",
      "selectDone": "Done",
      "selectedCount": "{count} selected",
      "addTagSelected": "Add tag",
      "tagsAdded": "“{tag}” added to {count} photos",
      "filterColor": "Color filter",
      "filterBtn": "Filter",
      "filterClear": "Clear",
      "filterEmpty": "No photos match the color filter",
      "status": {
        "count": "{count} photos",
        "partial": "{shown} / {total} photos"
      }
    },
    "colors": {
      "red": "Red",
      "orange": "Orange",
      "yellow": "Yellow",
      "green": "Green",
      "blue": "Blue",
      "purple": "Purple"
    },
    "folders": {
      "empty": "No folders added yet",
      "add": "+ Add Folder",
      "refresh": "↻ Refresh",
      "remove": "Remove",
      "count": "{count} photos",
      "status": {
        "count": "{count} folders"
      }
    },
    "card": {
      "edit": {
        "title": "Edit tags",
        "current": "Current tags",
        "suggest": "Add from existing tags",
        "noTags": "No tags yet",
        "noSuggest": "No more tags to add — type a new one below",
        "remove": "Remove tag",
        "placeholder": "Type a new tag name and press Enter",
        "save": "Save",
        "cancel": "Cancel"
      }
    },
    "menu": {
      "reveal": "Show in File Explorer"
    },
    "preview": {
      "close": "Close preview",
      "error": "Cannot preview this file"
    },
    "dialog": {
      "confirmTitle": "Confirm action",
      "ok": "OK",
      "cancel": "Cancel"
    },
    "tagging": {
      "badge": "Tagging in progress",
      "indexing": "Indexing",
      "remaining": "{count} left",
      "indexingRemaining": "{count} left"
    },
    "tags": {
      "title": "Custom tags",
      "hint": "Click “AI Tagging” to tag photos with all current tags — new tags and new photos are included.",
      "pickTitle": "Add tag to selected photos",
      "pickSearch": "Search tags…",
      "pickNoMatch": "No matching tags",
      "pickEmpty": "No tags defined — add one in the Tags tab first",
      "tagPlaceholder": "Tag name, e.g. plane / sunset / long-exposure",
      "tagThreshold": "Threshold",
      "tagCount": "{count} photos",
      "addTag": "+ Add",
      "removeTag": "Remove",
      "empty": "No tags defined — add a tag, then click “AI Tagging” to start",
      "nameRequired": "Please enter a tag name",
      "runButton": "AI Tagging",
      "runStarted": "{count} photos queued for AI tagging",
      "runNoTags": "No tags defined yet — add a tag first",
      "clearAll": "Clear tags",
      "clearAllConfirm": "This will delete every tag definition and all tags on all photos (including manual tags). This cannot be undone. Continue?"
    },
    "settings": {
      "title": "Settings",
      "language": "Language",
      "languageZh": "中文",
      "languageEn": "English",
      "hwDecode": "Hardware decoding",
      "hwDecodeHint": "Restart the app for the change to take effect",
      "restart": "Restart App",
      "on": "On",
      "off": "Off",
      "cacheLabel": "Thumbnail cache",
      "clearCache": "Clear cache",
      "cacheCleared": "Cache cleared",
      "modelStatus": "AI models",
      "modelLocked": "Ready",
      "modelDownloading": "Downloading models",
      "modelError": "Model error (AI unavailable)",
      "aiProgress": "Processing {done}/{remaining}",
      "aiProvider": "AI engine",
      "aiAuto": "Auto",
      "aiGpu": "GPU",
      "aiCpu": "CPU",
      "aiCoreml": "Apple CoreML",
      "debug": "Debug mode",
      "gpu": "GPU renderer: {renderer}",
      "gpuSoftware": "(software rendering — hardware acceleration inactive)",
      "gpuUnknown": "GPU renderer: unable to detect"
    }
  }
};
