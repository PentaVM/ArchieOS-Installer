//! IRIS on the node, in Rust.
//!
//! Reads a released IRIS Beta weights file (`archie-iris-beta1-weights-v1`, the `.pt`
//! PyTorch wrote) directly, without Python, and runs what `iris_serve` serves: detection of
//! the phrases asked for, "everything" mode over the everyday vocabulary, instance
//! embeddings, and acting with a memory per session. [`server`] exposes the same loopback
//! HTTP API, so `archie-voice live` and Arena talk to it unchanged.
//!
//! IRIS proposes; it never acts. Every action it suggests still goes through Cerebra, the
//! policy and the Stage gate (`archie-iris-bridge`), exactly as with the Python server.
//!
//! What a node needs besides the release: the SigLIP2 text tower (`model.safetensors` and
//! `tokenizer.json` of `google/siglip2-base-patch16-384`), which the release does not carry,
//! or a table of pre-encoded phrases. Not ported: LoRA skill adapters, REM recording, and
//! the mask, keypoint and reading heads, which nothing served reads.

pub mod act;
pub mod detect;
pub mod fast;
pub mod model;
pub mod nn;
pub mod phrases;
pub mod preprocess;
pub mod roi;
pub mod server;
pub mod siglip;
pub mod vjepa;
pub mod weights;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("weights: {0}")]
    Weights(String),
    #[error("the weights have no tensor {0}")]
    MissingTensor(String),
    #[error("image: {0}")]
    Image(String),
    #[error("phrase: {0}")]
    Phrase(String),
    #[error("refused: {0}")]
    Refused(String),
    #[error(transparent)]
    Tensor(#[from] candle_core::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// The device inference runs on: the first CUDA GPU when built with `cuda` and one is
/// present, otherwise the CPU. bf16 on a GPU that computes in it natively, float32 elsewhere
/// (as `iris_infer.gpu_dtype` decides; `IRIS_PRECISION` overrides).
pub fn choose_device(requested: &str) -> Result<(candle_core::Device, candle_core::DType), Error> {
    use candle_core::{DType, Device};
    let device = match requested {
        "cpu" => Device::Cpu,
        "cuda" => Device::new_cuda(0)?,
        _ => Device::cuda_if_available(0)?,
    };
    let precision = std::env::var("IRIS_PRECISION")
        .unwrap_or_else(|_| "auto".into())
        .to_lowercase();
    let dtype = match precision.as_str() {
        "bf16" | "bfloat16" => DType::BF16,
        "float32" | "fp32" => DType::F32,
        _ if device.is_cuda() && cuda_has_native_bf16() => DType::BF16,
        _ => DType::F32,
    };
    Ok((device, dtype))
}

/// Ampere (compute capability 8) and later compute in bf16; candle does not report the
/// capability, so `IRIS_PRECISION` is the way to say a card is older.
fn cuda_has_native_bf16() -> bool {
    std::env::var("IRIS_CUDA_CAPABILITY")
        .ok()
        .and_then(|v| {
            v.split('.')
                .next()
                .and_then(|major| major.parse::<u32>().ok())
        })
        .is_none_or(|major| major >= 8)
}
