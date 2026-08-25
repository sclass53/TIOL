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

/// Model files for the CURRENT platform (C-11.14):
/// - Windows/Linux: int8 (small, 412MB total) — x64 MLAS has ConvInteger.
/// - macOS (Apple Silicon): fp16 — the official ONNX Runtime's ARM64 CPU EP
///   LACKS ConvInteger kernels ("Could not find an implementation for
///   ConvInteger(10)"), so the int8 models cannot run there at all.
///   fp16 convs are universally supported (CPU NEON / CoreML).
pub fn model_lock() -> &'static [(&'static str, ModelFileInfo)] {
    #[cfg(target_os = "macos")]
    {
        &[
            (
                "vision_model_fp16.onnx",
                ModelFileInfo {
                    url: "https://hf-mirror.com/onnx-community/siglip2-base-patch16-224-ONNX/resolve/main/onnx/vision_model_fp16.onnx",
                    size: 186039516,
                    sha256: "a1959f7bd3993a607e48839f6d01e25b876fe76afda301b028b78eef68aabd95",
                },
            ),
            (
                "text_model_fp16.onnx",
                ModelFileInfo {
                    url: "https://hf-mirror.com/onnx-community/siglip2-base-patch16-224-ONNX/resolve/main/onnx/text_model_fp16.onnx",
                    size: 564862230,
                    sha256: "711da56ada0a4aa11c7dd3320df741081a3cae4f0ae1b5e5c6d5b294738d0eb0",
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
        ]
    }
    #[cfg(not(target_os = "macos"))]
    {
        &[
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
        ]
    }
}

/// Status of the model directory (ADD.md §4.1).
#[derive(Debug, Clone, PartialEq)]
pub enum ModelStatus {
    /// All files present and verified — models may be loaded.
    /// Payload: active inference backend label ("cpu" / "gpu" / "coreml").
    Locked(String),
    /// Download failed after retries; AI features degraded.
    Degraded(String),
}
