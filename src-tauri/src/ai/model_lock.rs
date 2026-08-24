//! Model lockfile (ADD.md §4.1 / "模型锁定"): the exact files, sizes and
//! SHA256 hashes the app is pinned to. Updated ONLY with an app release —
//! never at runtime. Values were verified against hf-mirror.com downloads.
//!
//! Pure SigLIP architecture (MIGRATE1.md V3.0, change C-09): only the SigLIP
//! vision/text encoders + tokenizer. Auto-tagging uses user-defined tags
//! matched via text embeddings — no DeepDanbooru model.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelFileInfo {
    pub url: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

pub const MODEL_LOCK: &[(&str, ModelFileInfo)] = &[
    (
        "vision_model_int8.onnx",
        ModelFileInfo {
            url: "https://hf-mirror.com/onnx-community/siglip2-base-patch16-224-ONNX/resolve/main/onnx/vision_model_int8.onnx",
            size: 94553333,
            sha256: "0dd31785a2713f1113ef2272472165c69d580473dae38d7b47568ac587795e70",
        },
    ),
    (
        "text_model_int8.onnx",
        ModelFileInfo {
            url: "https://hf-mirror.com/onnx-community/siglip2-base-patch16-224-ONNX/resolve/main/onnx/text_model_int8.onnx",
            size: 283438275,
            sha256: "3a0603d3a00c05a80a6ded4743c16aaac7b1e62cdcc7e362e7ce418659b96400",
        },
    ),
    (
        "tokenizer.json",
        ModelFileInfo {
            url: "https://hf-mirror.com/onnx-community/siglip2-base-patch16-224-ONNX/resolve/main/tokenizer.json",
            size: 34363039,
            sha256: "cb9140fae3ac5122c972d37adf83e1248471a38147ad76f8215c8872c6fd8322",
        },
    ),
];

/// Status of the model directory (ADD.md §4.1).
#[derive(Debug, Clone, PartialEq)]
pub enum ModelStatus {
    /// All files present and verified — models may be loaded.
    /// Payload: active inference backend label ("cpu" / "gpu" / "coreml").
    Locked(String),
    /// Download failed after retries; AI features degraded.
    Degraded(String),
}
