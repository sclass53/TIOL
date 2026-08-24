//! AI inference engine — SigLIP dual encoder (image + text) for semantic
//! search (ADD.md §4/§6). DeepDanbooru auto-tagging is deferred (per product
//! decision) and can be re-added later. Built against ort 2.0.0-rc.13.

use crate::error::{AppError, Result};
use ndarray::Dimension;
use ort::session::Session;
use ort::value::Tensor;
use std::path::Path;
use std::sync::Mutex;

pub struct AIEngine {
    vision: Mutex<Session>,
    text: Mutex<Session>,
    tokenizer: tokenizers::Tokenizer,
}

/// L2-normalize a vector in place.
fn normalize(v: &mut [f32]) {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 1e-8 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

/// Extract the model's `pooler_output` (the projected, alignment-space
/// embedding — the FIRST output is raw last_hidden_state which is NOT in
/// contrastive space; using it crushed image-text similarity, C-09).
fn extract_pooled(outputs: &ort::session::SessionOutputs) -> Result<(Vec<usize>, Vec<f32>)> {
    let out = outputs
        .iter()
        .find(|(name, _)| name.to_lowercase().contains("pooler"))
        .map(|(_, v)| v)
        .or_else(|| outputs.iter().next().map(|(_, v)| v))
        .ok_or_else(|| AppError::Ai("model produced no outputs".into()))?;
    if let Ok((shape, data)) = out.try_extract_tensor::<f32>() {
        let dims = shape.to_ixdyn().slice().to_vec();
        return Ok((dims, data.to_vec()));
    }
    let (shape, data) = out
        .try_extract_tensor::<half::f16>()
        .map_err(|e| AppError::Ai(format!("unsupported output dtype: {e}")))?;
    let dims = shape.to_ixdyn().slice().to_vec();
    Ok((dims, data.iter().map(|h| f32::from(*h)).collect()))
}

/// Build a session with the given execution providers.
fn build_session(
    model_path: &Path,
    providers: &[ort::ep::ExecutionProviderDispatch],
) -> Result<Session> {
    Session::builder()
        .map_err(|e| AppError::Ai(e.to_string()))?
        .with_execution_providers(providers)
        .map_err(|e| AppError::Ai(e.to_string()))?
        .commit_from_file(model_path)
        .map_err(|e| AppError::Ai(e.to_string()))
}

/// GPU providers available on this platform (in priority order).
fn platform_gpu_providers() -> Vec<ort::ep::ExecutionProviderDispatch> {
    #[cfg(target_os = "windows")]
    {
        vec![
            ort::ep::CUDA::default().build(),
            ort::ep::DirectML::default().build(),
        ]
    }
    #[cfg(target_os = "macos")]
    {
        vec![ort::ep::CoreML::default().build()]
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        Vec::new()
    }
}

/// Apple accelerator providers for the "mlx" mode (Apple MLX). ort 2.0.0-rc.13
/// ships no MLX execution provider yet, so CoreML — Apple's other native
/// accelerator — is used on macOS; empty on every other platform (caller
/// falls back to CPU with an honest label).
fn apple_accel_providers() -> Vec<ort::ep::ExecutionProviderDispatch> {
    #[cfg(target_os = "macos")]
    {
        vec![ort::ep::CoreML::default().build()]
    }
    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
}

/// Auto-detect the best backend: probe each GPU provider with a real session
/// build AND a smoke inference (a provider can register successfully yet be
/// unusable at runtime, e.g. missing CUDA DLLs). First working one wins.
fn detect_backend(probe_model: &Path) -> (Vec<ort::ep::ExecutionProviderDispatch>, &'static str) {
    #[cfg(target_os = "windows")]
    let candidates: [(&'static str, ort::ep::ExecutionProviderDispatch); 2] = [
        ("cuda", ort::ep::CUDA::default().build()),
        ("directml", ort::ep::DirectML::default().build()),
    ];
    #[cfg(target_os = "macos")]
    let candidates: [(&'static str, ort::ep::ExecutionProviderDispatch); 1] =
        [("coreml", ort::ep::CoreML::default().build())];
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let candidates: [(&'static str, ort::ep::ExecutionProviderDispatch); 0] = [];

    for (label, provider) in candidates {
        if probe_works(probe_model, &provider) {
            return (vec![provider], label);
        }
    }
    (vec![ort::ep::CPU::default().build()], "cpu")
}

/// Build a session with the provider and run one dummy inference
/// (vision input [1, 3, 224, 224]).
fn probe_works(model_path: &Path, provider: &ort::ep::ExecutionProviderDispatch) -> bool {
    let session = match build_session(model_path, std::slice::from_ref(provider)) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let data = vec![0f32; 3 * 224 * 224];
    let tensor = match Tensor::from_array((vec![1i64, 3, 224, 224], data)) {
        Ok(t) => t,
        Err(_) => return false,
    };
    let mut session = session;
    let name = session.inputs()[0].name().to_string();
    {
        let outputs = session.run(ort::inputs![name => tensor]);
        outputs.is_ok()
    }
}

impl AIEngine {
    /// Build sessions from a verified model directory (all-or-nothing).
    /// `mode`: "auto" (detect), "gpu" (GPU only), "cpu" (CPU only).
    /// Returns the engine plus the active backend label.
    pub fn load(model_dir: &Path, mode: &str) -> Result<(Self, String)> {
        let vision_path = model_dir.join("vision_model_int8.onnx");
        let (providers, label) = match mode {
            "gpu" => (platform_gpu_providers(), "gpu"),
            "cpu" => (vec![ort::ep::CPU::default().build()], "cpu"),
            "mlx" => {
                let providers = apple_accel_providers();
                if providers.is_empty() || !probe_works(&vision_path, &providers[0]) {
                    log::warn!(
                        "Apple MLX backend unavailable on this machine — falling back to CPU"
                    );
                    (vec![ort::ep::CPU::default().build()], "cpu")
                } else {
                    (providers, "mlx")
                }
            }
            _ => detect_backend(&vision_path),
        };

        let vision = build_session(&vision_path, &providers)
            .map_err(|e| AppError::Ai(format!("vision: {e}")))?;
        let text = build_session(&model_dir.join("text_model_int8.onnx"), &providers)
            .map_err(|e| AppError::Ai(format!("text: {e}")))?;
        let tokenizer = tokenizers::Tokenizer::from_file(model_dir.join("tokenizer.json"))
            .map_err(|e| AppError::Ai(format!("tokenizer: {e}")))?;

        Ok((
            Self {
                vision: Mutex::new(vision),
                text: Mutex::new(text),
                tokenizer,
            },
            label.to_string(),
        ))
    }

    /// Image embedding (SigLIP vision encoder), normalized.
    pub fn embed_image(&self, image_path: &Path) -> Result<Vec<f32>> {
        let data = Self::preprocess_image(image_path)?;
        let tensor = Tensor::from_array((vec![1i64, 3, 224, 224], data))
            .map_err(|e| AppError::Ai(e.to_string()))?;
        let mut vision = self.vision.lock().unwrap_or_else(|e| e.into_inner());
        let name = vision
            .inputs()
            .first()
            .ok_or_else(|| AppError::Ai("vision model has no inputs".to_string()))?
            .name()
            .to_string();
        let outputs = vision
            .run(ort::inputs![name => tensor])
            .map_err(|e| AppError::Ai(format!("vision run: {e}")))?;
        // pooler_output is the projected embedding (alignment space).
        let (dims, values) = extract_pooled(&outputs)?;
        let n = dims.last().copied().unwrap_or(0);
        let mut v = values[..n.min(values.len())].to_vec();
        normalize(&mut v);
        Ok(v)
    }

    /// Text embedding (SigLIP text encoder), normalized. Never falls back —
    /// failures are errors (ADD.md §4.3 / §6.2).
    pub fn embed_text(&self, text: &str) -> Result<Vec<f32>> {
        const MAX_LEN: usize = 64;
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| AppError::Ai(format!("tokenize: {e}")))?;
        let ids: Vec<i64> = encoding
            .get_ids()
            .iter()
            .take(MAX_LEN)
            .map(|t| *t as i64)
            .collect();
        if ids.is_empty() {
            return Err(AppError::Ai("empty token sequence".to_string()));
        }
        let seq = ids.len() as i64;
        let ids_t = Tensor::from_array((vec![1i64, seq], ids))
            .map_err(|e| AppError::Ai(e.to_string()))?;
        let mask_t = Tensor::from_array((vec![1i64, seq], vec![1i64; seq as usize]))
            .map_err(|e| AppError::Ai(e.to_string()))?;

        // Build inputs from the session's declared input names instead of
        // assuming a fixed count: the int8 text export fuses attention_mask
        // into the graph (single `input_ids` input), other exports declare
        // both. Poison-safe lock so one failed run can't brick later searches.
        let mut text_sess = self.text.lock().unwrap_or_else(|e| e.into_inner());
        let names: Vec<String> = text_sess
            .inputs()
            .iter()
            .map(|i| i.name().to_string())
            .collect();
        let in0 = names
            .first()
            .cloned()
            .ok_or_else(|| AppError::Ai("text model has no inputs".to_string()))?;
        let mut ins: Vec<(String, ort::session::SessionInputValue<'_>)> =
            vec![(in0, ids_t.into())];
        if names.len() > 1 {
            ins.push((names[1].clone(), mask_t.into()));
        }
        let outputs = text_sess
            .run(ins)
            .map_err(|e| AppError::Ai(format!("text run: {e}")))?;
        // pooler_output = EOS-token hidden state passed through the
        // projection head — the true contrastive-space embedding.
        let (dims, values) = extract_pooled(&outputs)?;
        let n = dims.last().copied().unwrap_or(0);
        if n == 0 {
            return Err(AppError::Ai("text model produced no embedding".to_string()));
        }
        let mut v: Vec<f32> = values[..n.min(values.len())].to_vec();
        normalize(&mut v);
        Ok(v)
    }

    /// Decode an image to a normalized flat [1, 3, 224, 224] f32 tensor.
    /// CLIP-style normalization (SigLIP inherits it): pixels /255 -> [-1, 1].
    fn preprocess_image(image_path: &Path) -> Result<Vec<f32>> {
        let img = image::open(image_path)
            .map_err(|e| AppError::Ai(format!("open {}: {e}", image_path.display())))?;
        let img = img.resize_exact(224, 224, image::imageops::FilterType::Triangle);
        let rgb = img.to_rgb8();
        let mut data = vec![0f32; 3 * 224 * 224];
        for y in 0usize..224 {
            for x in 0usize..224 {
                let px = rgb.get_pixel(x as u32, y as u32);
                data[y * 224 + x] = (px[0] as f32 / 255.0 - 0.5) / 0.5;
                data[224 * 224 + y * 224 + x] = (px[1] as f32 / 255.0 - 0.5) / 0.5;
                data[2 * 224 * 224 + y * 224 + x] = (px[2] as f32 / 255.0 - 0.5) / 0.5;
            }
        }
        Ok(data)
    }
}

// ---------------------------------------------------------------------------
// User-defined tag matching (MIGRATE1.md V3.0, change C-09): tags are plain
// text embedded by the SigLIP text encoder; photos are tagged by cosine
// similarity between their image embedding and the cached tag vectors.
// TagVec lives here so both the queue (matching) and main.rs (cache) share it.
// ---------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct TagVec {
    pub name: String,
    pub threshold: f64,
    pub vec: Vec<f32>, // L2-normalized text embedding
}

/// Cosine similarity between two L2-normalized vectors (plain dot product).
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
    }
    dot
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Locate the model dir (env override or default LocalAppData location).
    fn model_dir() -> std::path::PathBuf {
        if let Ok(dir) = std::env::var("TIOL_MODEL_DIR") {
            return std::path::PathBuf::from(dir);
        }
        let base = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".to_string());
        std::path::PathBuf::from(base).join("com.tiol.desktop").join("models")
    }

    /// Real-model smoke test: text encoder must handle single-token CJK,
    /// multi-token sentences, and empty-ish input without panicking or
    /// poisoning the session mutex (regression for the `人` crash).
    /// NOTE: run with `ORT_DYLIB_PATH` pointing at the vendored
    /// `target/debug/onnxruntime.dll` (test exes in `deps\` can't find it).
    #[test]
    #[ignore = "requires downloaded SigLIP models"]
    fn text_embed_never_panics() {
        let (engine, backend) = AIEngine::load(&model_dir(), "cpu").expect("engine load");
        assert_eq!(backend, "cpu");
        let mut dim: Option<usize> = None;
        for q in ["人", "一张日落的照片", "a photo of a sunset", "cat", ""] {
            let v = engine
                .embed_text(q)
                .unwrap_or_else(|e| panic!("embed_text({q:?}) failed: {e}"));
            assert_eq!(v.len(), 768, "unexpected dim for {q:?}");
            assert!(v.iter().any(|x| x.abs() > 1e-6), "degenerate vector for {q:?}");
            dim = Some(v.len());
        }
        let _ = dim;
    }

    /// Temporary experiment: compare image preprocessing variants (raw /255
    /// vs CLIP-style [-1,1] normalization) on text-image similarity scale.
    #[test]
    #[ignore = "experiment"]
    fn preprocess_experiment() {
        let (engine, _backend) = AIEngine::load(&model_dir(), "cpu").expect("engine load");
        let dir = std::env::var("TIOL_TEST_IMAGES")
            .unwrap_or_else(|_| "E:\\ImageManager\\test_imgs".to_string());
        let image = std::fs::read_dir(&dir)
            .expect("test images dir")
            .flatten()
            .find(|e| {
                e.path()
                    .extension()
                    .and_then(|x| x.to_str())
                    .map(|x| matches!(x.to_lowercase().as_str(), "jpg" | "jpeg" | "png" | "webp"))
                    .unwrap_or(false)
            })
            .expect("no image");
        let mut tags: Vec<(String, Vec<f32>)> = Vec::new();
        for tag in [
            "human",
            "a person",
            "人",
            "a cup of coffee",
            "a plate of food on a table",
            "a photo of grass and trees",
            "a blurry photo of a person",
            "a building in a city",
        ] {
            tags.push((tag.to_string(), engine.embed_text(tag).expect("t")));
        }
        for entry in std::fs::read_dir(&dir).expect("test images dir").flatten() {
            let p = entry.path();
            let is_img = p
                .extension()
                .and_then(|x| x.to_str())
                .map(|x| matches!(x.to_lowercase().as_str(), "jpg" | "jpeg" | "png" | "webp"))
                .unwrap_or(false);
            if !is_img {
                continue;
            }
            let base = AIEngine::preprocess_image(&p).expect("pre");
            let tensor = Tensor::from_array((vec![1i64, 3, 224, 224], base)).expect("t");
            let mut vision = engine.vision.lock().unwrap();
            let name = vision.inputs()[0].name().to_string();
            let out = vision.run(ort::inputs![name => tensor]).expect("run");
            let (dims, values) = extract_pooled(&out).expect("extract");
            let n = dims.last().copied().unwrap_or(0);
            let mut v = values[..n.min(values.len())].to_vec();
            normalize(&mut v);
            let mut scores: Vec<(String, f32)> = tags
                .iter()
                .map(|(t, tv)| (t.clone(), cosine(&v, tv)))
                .collect();
            scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            let mean: f32 = scores.iter().map(|(_, s)| *s).sum::<f32>() / scores.len() as f32;
            let var: f32 = scores.iter().map(|(_, s)| (s - mean) * (s - mean)).sum::<f32>() / scores.len() as f32;
            let sigma = var.sqrt();
            println!(
                "{}: mean={mean:.3} sigma={sigma:.3} | {}",
                p.file_name().unwrap_or_default().to_string_lossy(),
                scores
                    .iter()
                    .map(|(t, s)| format!("{t}={s:.3}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            );
        }
    }

    /// Real-model smoke test for SigLIP-based tag matching (MIGRATE1.md):
    /// embeds a few user-style tags + one image and sanity-checks that the
    /// matching tag scores above unrelated ones. Informational asserts.
    /// Same ORT_DYLIB_PATH note as above.
    #[test]
    #[ignore = "requires downloaded SigLIP models"]
    fn siglip_tag_match_sanity() {
        let (engine, _backend) = AIEngine::load(&model_dir(), "cpu").expect("engine load");
        let dir = std::env::var("TIOL_TEST_IMAGES")
            .unwrap_or_else(|_| "E:\\ImageManager\\test_imgs".to_string());
        // The first image in the test folder (DSC01354-ish: food/drink scene).
        let image = std::fs::read_dir(&dir)
            .expect("test images dir")
            .flatten()
            .find(|e| {
                e.path()
                    .extension()
                    .and_then(|x| x.to_str())
                    .map(|x| matches!(x.to_lowercase().as_str(), "jpg" | "jpeg" | "png" | "webp"))
                    .unwrap_or(false)
            })
            .expect("no image in test dir");
        let img_vec = engine
            .embed_image(&image.path())
            .expect("embed_image");
        // Descriptive tags separate better than single words (measured:
        // phrases score 0.09-0.13 on matches, single words ~0.06-0.08).
        let mut scores: Vec<(String, f32)> = Vec::new();
        for tag in [
            "a cup of coffee",
            "a plate of food on a table",
            "a photo of grass and trees",
            "a blurry photo of a person",
            "a building in a city",
        ] {
            let tv = engine.embed_text(tag).expect("embed_text");
            scores.push((tag.to_string(), cosine(&img_vec, &tv)));
        }
        scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        println!(
            "{} tag scores: {:?}",
            image.file_name().to_string_lossy(),
            scores
                .iter()
                .map(|(n, s)| format!("{n}={s:.3}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        // The top tag must be a real match (food/coffee-family image);
        // everything else is informational. Calibrated to the pooler-output
        // scale: descriptive matches score ~0.09-0.13, noise < 0.08.
        assert!(!scores.is_empty());
        assert!(scores[0].1 > 0.08, "top tag score too low: {:?}", scores[0]);
    }
}
