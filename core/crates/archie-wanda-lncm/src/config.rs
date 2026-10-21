//! Model shape. One file describes a checkpoint completely; nothing about the architecture is
//! decided in code that a config cannot change, so a larger or smaller expert is a new config,
//! not a new build.

use crate::{LncmError, Result};
use serde::{Deserialize, Serialize};

pub const CONFIG_FORMAT: u16 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransformerConfig {
    pub d_model: usize,
    pub layers: usize,
    pub heads: usize,
    pub d_ff: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VqConfig {
    pub d_model: usize,
    pub encoder_layers: usize,
    pub decoder_layers: usize,
    pub heads: usize,
    pub d_ff: usize,
    /// Weight of the commitment term that keeps the encoder near its codes.
    pub commitment: f32,
    /// Share of training steps whose reconstruction uses only the first levels, so the coarse
    /// levels learn to carry the gist on their own.
    pub nested_dropout: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoraTarget {
    Q,
    K,
    V,
    O,
    FfIn,
    FfOut,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoraConfig {
    pub rank: usize,
    pub alpha: f32,
    pub targets: Vec<LoraTarget>,
}

impl LoraConfig {
    pub fn scale(&self) -> f64 {
        f64::from(self.alpha) / self.rank as f64
    }
    pub fn applies(&self, target: LoraTarget) -> bool {
        self.targets.contains(&target)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LncmConfig {
    pub format: u16,
    /// Must equal the codec's vocabulary; checked when a checkpoint is opened.
    pub vocab_size: usize,
    /// Longest prompt, in tokens.
    pub prompt_len: usize,
    /// Length of every generated target, in tokens (end marker and padding included).
    pub target_len: usize,
    /// Positions per level, coarse to fine. Strictly increasing, each divides `target_len`,
    /// and the last equals it.
    pub levels: Vec<usize>,
    pub codebook_size: usize,
    pub code_dim: usize,
    pub vq: VqConfig,
    pub generator: TransformerConfig,
    pub max_options: usize,
    pub option_len: usize,
    /// Per level, the share of conditioning codes replaced by random ones while training,
    /// so generation recovers from its own mistakes at coarser levels.
    pub corruption: Vec<f32>,
    /// α in the level weighting `w_k = ℓ_k^-α / Σ ℓ_j^(1-α)`: 0 weighs every code equally,
    /// 1 weighs every level equally.
    pub level_alpha: f32,
    pub lora: LoraConfig,
    pub dropout: f32,
    /// Fitted after training on development data; divides the decision logits.
    pub decision_temperature: f32,
}

impl LncmConfig {
    /// For tests: trains in seconds on a CPU.
    pub fn tiny(vocab_size: usize) -> Self {
        Self {
            format: CONFIG_FORMAT,
            vocab_size,
            prompt_len: 192,
            target_len: 32,
            levels: vec![2, 8, 32],
            codebook_size: 32,
            code_dim: 16,
            vq: VqConfig {
                d_model: 32,
                encoder_layers: 1,
                decoder_layers: 1,
                heads: 2,
                d_ff: 64,
                commitment: 0.25,
                nested_dropout: 0.2,
            },
            generator: TransformerConfig {
                d_model: 32,
                layers: 2,
                heads: 2,
                d_ff: 64,
            },
            max_options: 4,
            option_len: 24,
            corruption: vec![0.0, 0.05, 0.1],
            level_alpha: 0.5,
            lora: LoraConfig {
                rank: 4,
                alpha: 8.0,
                targets: vec![LoraTarget::Q, LoraTarget::V, LoraTarget::O],
            },
            dropout: 0.0,
            decision_temperature: 1.0,
        }
    }

    /// About 20 M parameters: fits an 8 GB consumer GPU for training and a laptop CPU for use.
    pub fn small(vocab_size: usize) -> Self {
        Self {
            prompt_len: 768,
            target_len: 256,
            levels: vec![4, 16, 64, 256],
            codebook_size: 1024,
            code_dim: 64,
            vq: VqConfig {
                d_model: 256,
                encoder_layers: 3,
                decoder_layers: 3,
                heads: 4,
                d_ff: 1024,
                commitment: 0.25,
                nested_dropout: 0.2,
            },
            generator: TransformerConfig {
                d_model: 384,
                layers: 6,
                heads: 6,
                d_ff: 1536,
            },
            max_options: 8,
            option_len: 48,
            corruption: vec![0.0, 0.05, 0.1, 0.15],
            lora: LoraConfig {
                rank: 16,
                alpha: 32.0,
                targets: vec![
                    LoraTarget::Q,
                    LoraTarget::K,
                    LoraTarget::V,
                    LoraTarget::O,
                    LoraTarget::FfIn,
                    LoraTarget::FfOut,
                ],
            },
            dropout: 0.1,
            ..Self::tiny(vocab_size)
        }
    }

    /// About 120 M parameters, the scale of the paper's generators plus the tokenizer.
    pub fn base(vocab_size: usize) -> Self {
        Self {
            codebook_size: 4096,
            code_dim: 96,
            vq: VqConfig {
                d_model: 512,
                encoder_layers: 4,
                decoder_layers: 4,
                heads: 8,
                d_ff: 2048,
                ..Self::small(vocab_size).vq
            },
            generator: TransformerConfig {
                d_model: 768,
                layers: 12,
                heads: 12,
                d_ff: 3072,
            },
            ..Self::small(vocab_size)
        }
    }

    /// About 350 M parameters. Needs a data-centre GPU to train.
    pub fn large(vocab_size: usize) -> Self {
        Self {
            prompt_len: 1024,
            codebook_size: 8192,
            code_dim: 128,
            vq: VqConfig {
                d_model: 768,
                encoder_layers: 6,
                decoder_layers: 6,
                heads: 12,
                d_ff: 3072,
                ..Self::base(vocab_size).vq
            },
            generator: TransformerConfig {
                d_model: 1024,
                layers: 20,
                heads: 16,
                d_ff: 4096,
            },
            ..Self::base(vocab_size)
        }
    }

    pub fn preset(name: &str, vocab_size: usize) -> Result<Self> {
        match name {
            "tiny" => Ok(Self::tiny(vocab_size)),
            "small" => Ok(Self::small(vocab_size)),
            "base" => Ok(Self::base(vocab_size)),
            "large" => Ok(Self::large(vocab_size)),
            _ => Err(LncmError::Config(format!("unknown preset {name}"))),
        }
    }

    pub fn validate(&self) -> Result<()> {
        let fail = |why: &str| Err(LncmError::Config(why.to_string()));
        if self.format != CONFIG_FORMAT {
            return fail("unknown config format");
        }
        if !(272..=(256 + crate::codec::MAX_MERGES + 16)).contains(&self.vocab_size) {
            return fail("vocabulary size out of range");
        }
        if !(32..=8192).contains(&self.prompt_len) {
            return fail("prompt_len must be 32..=8192");
        }
        if !(8..=2048).contains(&self.target_len) {
            return fail("target_len must be 8..=2048");
        }
        if self.levels.is_empty()
            || self.levels.len() > 8
            || self.levels.windows(2).any(|w| w[0] >= w[1])
            || self
                .levels
                .iter()
                .any(|&l| l == 0 || !self.target_len.is_multiple_of(l))
            || self.levels.last() != Some(&self.target_len)
        {
            return fail("levels must increase, divide target_len and end at it");
        }
        if self.corruption.len() != self.levels.len()
            || self.corruption.iter().any(|p| !(0.0..=0.5).contains(p))
        {
            return fail("one corruption share in 0..=0.5 per level");
        }
        if !(2..=65_536).contains(&self.codebook_size) || !(4..=1024).contains(&self.code_dim) {
            return fail("codebook_size 2..=65536 and code_dim 4..=1024");
        }
        for (name, t) in [
            ("generator", &self.generator),
            (
                "vq",
                &TransformerConfig {
                    d_model: self.vq.d_model,
                    layers: self.vq.encoder_layers.max(self.vq.decoder_layers),
                    heads: self.vq.heads,
                    d_ff: self.vq.d_ff,
                },
            ),
        ] {
            if t.d_model == 0
                || t.heads == 0
                || t.d_model % t.heads != 0
                || t.layers == 0
                || t.layers > 64
                || t.d_ff == 0
                || t.d_model > 8192
                || t.d_ff > 32_768
            {
                return fail(&format!("{name} transformer shape is invalid"));
            }
        }
        if self.vq.encoder_layers == 0 || self.vq.decoder_layers == 0 {
            return fail("the tokenizer needs an encoder and a decoder");
        }
        if !(0.0..=10.0).contains(&self.vq.commitment)
            || !(0.0..1.0).contains(&self.vq.nested_dropout)
        {
            return fail("commitment 0..=10 and nested_dropout 0..1");
        }
        if !(2..=16).contains(&self.max_options) || !(2..=512).contains(&self.option_len) {
            return fail("max_options 2..=16 and option_len 2..=512");
        }
        if !(0.0..=1.0).contains(&self.level_alpha) {
            return fail("level_alpha must be 0..=1");
        }
        if !(1..=256).contains(&self.lora.rank)
            || !(f32::MIN_POSITIVE..=1024.0).contains(&self.lora.alpha)
            || self.lora.targets.is_empty()
        {
            return fail("lora rank 1..=256, alpha in (0, 1024] and at least one target");
        }
        if !(0.0..0.5).contains(&self.dropout) {
            return fail("dropout must be 0..0.5");
        }
        if !(0.05..=20.0).contains(&self.decision_temperature) {
            return fail("decision_temperature must be 0.05..=20");
        }
        // The prompt has to hold its fixed markers, a question and every option.
        if self.prompt_len < 16 + self.max_options * self.option_len.min(16) {
            return fail("prompt_len too small for the options");
        }
        Ok(())
    }

    /// Weight of each level in the generator loss (eq. 10 of the NCM paper).
    pub fn level_weights(&self) -> Vec<f64> {
        let alpha = f64::from(self.level_alpha);
        let denominator: f64 = self
            .levels
            .iter()
            .map(|&l| (l as f64).powf(1.0 - alpha))
            .sum();
        self.levels
            .iter()
            .map(|&l| (l as f64).powf(-alpha) / denominator)
            .collect()
    }

    /// Approximate parameter count, for logs and model cards.
    pub fn parameters(&self) -> usize {
        let block = |d: usize, ff: usize| 4 * d * d + 2 * d * ff + 2 * d;
        let g = &self.generator;
        let v = &self.vq;
        let generator = g.layers * block(g.d_model, g.d_ff)
            + self.prompt_len * g.d_model
            + self.levels.iter().sum::<usize>() * g.d_model
            + v.d_model * g.d_model
            + self.code_dim * g.d_model
            + self.levels.len() * g.d_model * self.codebook_size;
        let tokenizer = (v.encoder_layers + v.decoder_layers) * block(v.d_model, v.d_ff)
            + 2 * self.vocab_size * v.d_model
            + 2 * self.target_len * v.d_model
            + 2 * v.d_model * self.code_dim
            + self.levels.len()
                * (self.codebook_size * self.code_dim + self.code_dim * self.code_dim);
        generator + tokenizer
    }

    pub fn to_toml(&self) -> Result<String> {
        toml::to_string(self).map_err(|e| LncmError::Config(e.to_string()))
    }

    pub fn from_toml(text: &str) -> Result<Self> {
        let config: Self = toml::from_str(text).map_err(|e| LncmError::Config(e.to_string()))?;
        config.validate()?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_validate_and_round_trip() {
        for name in ["tiny", "small", "base", "large"] {
            let config = LncmConfig::preset(name, 8192).unwrap();
            config.validate().unwrap();
            let back = LncmConfig::from_toml(&config.to_toml().unwrap()).unwrap();
            assert_eq!(back, config);
        }
        let small = LncmConfig::small(8192).parameters();
        let base = LncmConfig::base(8192).parameters();
        let large = LncmConfig::large(8192).parameters();
        assert!((10_000_000..40_000_000).contains(&small), "{small}");
        assert!((80_000_000..200_000_000).contains(&base), "{base}");
        assert!((250_000_000..500_000_000).contains(&large), "{large}");
    }

    #[test]
    fn level_weights_follow_equation_ten() {
        let mut config = LncmConfig::tiny(272);
        config.level_alpha = 1.0;
        let w = config.level_weights();
        // α = 1: every level weighs the same in total (w_k · ℓ_k is constant).
        let totals: Vec<f64> = w
            .iter()
            .zip(&config.levels)
            .map(|(w, &l)| w * l as f64)
            .collect();
        assert!(totals.windows(2).all(|t| (t[0] - t[1]).abs() < 1e-12));
        config.level_alpha = 0.0;
        let w = config.level_weights();
        assert!(w.windows(2).all(|t| (t[0] - t[1]).abs() < 1e-12));
    }

    #[test]
    fn invalid_shapes_are_refused() {
        let mut config = LncmConfig::tiny(272);
        config.levels = vec![8, 2, 32];
        assert!(config.validate().is_err());
        let mut config = LncmConfig::tiny(272);
        config.levels = vec![2, 8, 16];
        assert!(config.validate().is_err());
        let mut config = LncmConfig::tiny(272);
        config.generator.heads = 3;
        assert!(config.validate().is_err());
        let mut config = LncmConfig::tiny(272);
        config.corruption = vec![0.1];
        assert!(config.validate().is_err());
        let mut config = LncmConfig::tiny(100);
        config.vocab_size = 100;
        assert!(config.validate().is_err());
    }
}
