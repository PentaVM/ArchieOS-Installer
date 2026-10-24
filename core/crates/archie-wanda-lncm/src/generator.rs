//! The next-concept generator and the heads of state 1.
//!
//! One transformer reads a prompt and, when it generates, one block of slots per level. The
//! prompt attends only to itself; each level's block attends to the prompt and to itself
//! (prefix-LM, as in the NCM paper). A level's slots start from the sum of the coarser levels'
//! codes resized to that level, so in training every level is predicted in a single pass from
//! the true (lightly corrupted) coarser codes, and in use the levels are produced in order, one
//! pass each, with the prompt's keys and values computed once.
//!
//! The heads read the prompt only:
//! - `gate`: is the question for Wanda;
//! - `kind`: is it a yes/no question (so state 1 can answer it at once);
//! - `change`: does a chunk change the social state (so the reader can skip it);
//! - `decision`: one score per option, read from that option's own tokens in context, and one
//!   for "cannot know", read from the whole prompt.

use crate::config::LncmConfig;
use crate::nn::{self, Block, Ctx, Lin, RmsNorm, MASKED};
use crate::vq::VqVae;
use candle_core::{DType, Device, Result, Tensor, D};
use candle_nn::VarBuilder;
use rand::Rng;

#[derive(Clone)]
pub struct Generator {
    levels: Vec<usize>,
    target_len: usize,
    prompt_len: usize,
    code_dim: usize,
    max_options: usize,
    tok_proj: Lin,
    pos: Tensor,
    level_in: Lin,
    level_pos: Vec<Tensor>,
    level_emb: Tensor,
    blocks: Vec<Block>,
    norm: RmsNorm,
    code_heads: Vec<Lin>,
    gate: Lin,
    kind: Lin,
    change: Lin,
    option: Lin,
    abstain: Lin,
}

/// A padded batch of prompts.
pub struct PromptBatch {
    /// `(B, P)` ids, padded with the codec's pad id.
    pub ids: Tensor,
    pub lengths: Vec<usize>,
    pub option_spans: Vec<Vec<(usize, usize)>>,
}

impl PromptBatch {
    pub fn new(
        rows: &[Vec<u32>],
        spans: Vec<Vec<(usize, usize)>>,
        pad: u32,
        device: &Device,
    ) -> Result<Self> {
        let width = rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
        let lengths = rows.iter().map(Vec::len).collect();
        let padded: Vec<Vec<u32>> = rows
            .iter()
            .map(|r| {
                let mut r = r.clone();
                r.resize(width, pad);
                r
            })
            .collect();
        Ok(Self {
            ids: nn::ids_tensor(&padded, device)?,
            lengths,
            option_spans: spans,
        })
    }
    pub fn width(&self) -> Result<usize> {
        self.ids.dim(1)
    }
}

/// Hidden states after the final norm.
pub struct Hidden {
    pub prompt: Tensor,
    pub levels: Vec<Tensor>,
}

/// Raw head outputs (logits) for a batch.
pub struct Heads {
    /// `(B,)`
    pub gate: Tensor,
    /// `(B, 2)`: open, yes/no.
    pub kind: Tensor,
    /// `(B,)`
    pub change: Tensor,
    /// `(B, max_options + 1)`; absent options are masked; the last column is "cannot know".
    pub decision: Tensor,
}

/// The prompt's keys and values per layer, computed once for every level that reads them.
pub struct PromptCache {
    kv: Vec<(Tensor, Tensor)>,
    hidden: Tensor,
    len: usize,
}

impl PromptCache {
    /// The prompt's hidden states after the final norm `(1, P, d)`: the copy head's keys.
    pub fn hidden(&self) -> &Tensor {
        &self.hidden
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Sampling {
    /// 0 picks the most likely code everywhere.
    pub temperature: f32,
    /// Only the `top_k` most likely codes are drawn from; 0 means all.
    pub top_k: usize,
}

impl Sampling {
    pub const GREEDY: Sampling = Sampling {
        temperature: 0.0,
        top_k: 0,
    };
}

impl Generator {
    pub fn new(vb: &VarBuilder, config: &LncmConfig) -> Result<Self> {
        let vb = vb.pp("gen");
        let g = &config.generator;
        let d = g.d_model;
        let blocks = (0..g.layers)
            .map(|i| Block::new(&vb.pp(format!("blocks.{i}")), d, g.heads, g.d_ff, g.layers))
            .collect::<Result<Vec<_>>>()?;
        let level_pos = config
            .levels
            .iter()
            .enumerate()
            .map(|(k, &l)| nn::table(&vb, &format!("level_pos.{k}"), l, d))
            .collect::<Result<Vec<_>>>()?;
        let code_heads = (0..config.levels.len())
            .map(|k| {
                Lin::new(
                    &vb,
                    &format!("code_head.{k}"),
                    d,
                    config.codebook_size,
                    true,
                    0.02,
                    None,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            levels: config.levels.clone(),
            target_len: config.target_len,
            prompt_len: config.prompt_len,
            code_dim: config.code_dim,
            max_options: config.max_options,
            tok_proj: Lin::new(&vb, "tok_proj", config.vq.d_model, d, true, 0.02, None)?,
            pos: nn::table(&vb, "pos", config.prompt_len, d)?,
            level_in: Lin::new(&vb, "level_in", config.code_dim, d, true, 0.02, None)?,
            level_pos,
            level_emb: nn::table(&vb, "level_emb", config.levels.len(), d)?,
            blocks,
            norm: RmsNorm::new(&vb, "norm", d)?,
            code_heads,
            gate: Lin::new(&vb, "head.gate", d, 1, true, 0.02, None)?,
            kind: Lin::new(&vb, "head.kind", d, 2, true, 0.02, None)?,
            change: Lin::new(&vb, "head.change", d, 1, true, 0.02, None)?,
            option: Lin::new(&vb, "head.option", d, 1, true, 0.02, None)?,
            abstain: Lin::new(&vb, "head.abstain", d, 1, true, 0.02, None)?,
        })
    }

    pub fn attach_lora(
        &mut self,
        vb: &VarBuilder,
        config: &crate::config::LoraConfig,
    ) -> Result<()> {
        for block in &mut self.blocks {
            block.attach_lora(vb, config)?;
        }
        Ok(())
    }

    pub fn detach_lora(&mut self) {
        for block in &mut self.blocks {
            block.detach_lora();
        }
    }

    fn embed_prompt(&self, vq: &VqVae, ids: &Tensor) -> Result<Tensor> {
        let width = ids.dim(1)?;
        if width > self.prompt_len {
            candle_core::bail!("prompt of {width} tokens exceeds {}", self.prompt_len);
        }
        let tokens = vq.embed_tokens(ids)?.detach();
        self.tok_proj
            .forward(&tokens)?
            .broadcast_add(&self.pos.narrow(0, 0, width)?)
    }

    /// The inputs of each level's block, from the codes of the coarser levels. `codes[k]` is
    /// `(B, ℓ_k)`; the last level's codes are never read.
    pub fn level_inputs(&self, vq: &VqVae, codes: &[Tensor]) -> Result<Vec<Tensor>> {
        let b = codes
            .first()
            .ok_or_else(|| candle_core::Error::Msg("no levels".into()))?
            .dim(0)?;
        let device = codes[0].device();
        let mut acc = Tensor::zeros((b, self.target_len, self.code_dim), DType::F32, device)?;
        let mut inputs = Vec::with_capacity(self.levels.len());
        for (k, &l) in self.levels.iter().enumerate() {
            inputs.push(self.level_input(k, l, &acc)?);
            if k + 1 < self.levels.len() {
                acc = (acc + vq.code_embedding(k, &codes[k])?.detach())?;
            }
        }
        Ok(inputs)
    }

    fn level_input(&self, k: usize, l: usize, acc: &Tensor) -> Result<Tensor> {
        self.level_in
            .forward(&nn::downsample(acc, l)?)?
            .broadcast_add(&self.level_pos[k])?
            .broadcast_add(&self.level_emb.narrow(0, k, 1)?)
    }

    /// Training/eval forward over a padded batch. With `levels`, every level's block is
    /// appended after the prompt.
    pub fn forward(
        &self,
        vq: &VqVae,
        batch: &PromptBatch,
        levels: Option<&[Tensor]>,
        ctx: Ctx,
    ) -> Result<Hidden> {
        let device = batch.ids.device();
        let width = batch.width()?;
        let prompt = self.embed_prompt(vq, &batch.ids)?;
        let blocks: Vec<usize> = if levels.is_some() {
            self.levels.clone()
        } else {
            Vec::new()
        };
        let total = width + blocks.iter().sum::<usize>();
        let visible: Vec<Vec<(usize, usize)>> = batch
            .lengths
            .iter()
            .map(|&len| vec![(0, len.min(width)), (width, total)])
            .collect();
        let mut bias = nn::key_mask(&visible, total, device)?;
        let mut x = prompt;
        if let Some(levels) = levels {
            bias = bias.broadcast_add(&nn::prefix_block_mask(width, &blocks, device)?)?;
            let mut parts = vec![x];
            parts.extend(levels.iter().cloned());
            x = Tensor::cat(&parts, 1)?;
        }
        for block in &self.blocks {
            x = block.forward(&x, Some(&bias), ctx)?;
        }
        let x = self.norm.forward(&x)?;
        let prompt = x.narrow(1, 0, width)?;
        let mut out = Vec::with_capacity(blocks.len());
        let mut start = width;
        for &l in &blocks {
            out.push(x.narrow(1, start, l)?);
            start += l;
        }
        Ok(Hidden {
            prompt,
            levels: out,
        })
    }

    /// Code logits per level from the levels' hidden states.
    pub fn code_logits(&self, levels: &[Tensor]) -> Result<Vec<Tensor>> {
        levels
            .iter()
            .zip(&self.code_heads)
            .map(|(h, head)| head.forward(h))
            .collect()
    }

    /// Head logits from prompt hidden states `(B, P, d)`.
    pub fn heads(
        &self,
        prompt: &Tensor,
        lengths: &[usize],
        option_spans: &[Vec<(usize, usize)>],
    ) -> Result<Heads> {
        let (b, width, _) = prompt.dims3()?;
        let device = prompt.device();
        let whole: Vec<Vec<(usize, usize)>> =
            lengths.iter().map(|&l| vec![(0, l.min(width))]).collect();
        let pooled =
            nn::span_pool(prompt, &nn::span_weights(&whole, 1, width, device)?)?.squeeze(1)?;
        let options = nn::span_pool(
            prompt,
            &nn::span_weights(option_spans, self.max_options, width, device)?,
        )?;
        let option_scores = self.option.forward(&options)?.squeeze(D::Minus1)?;
        let abstain = self.abstain.forward(&pooled)?;
        let mut mask = vec![MASKED; b * (self.max_options + 1)];
        for (row, spans) in option_spans.iter().enumerate() {
            let present = spans.len().min(self.max_options);
            for slot in 0..present {
                mask[row * (self.max_options + 1) + slot] = 0.0;
            }
            mask[row * (self.max_options + 1) + self.max_options] = 0.0;
        }
        let mask = Tensor::from_vec(mask, (b, self.max_options + 1), device)?;
        let decision = Tensor::cat(&[option_scores, abstain], 1)?.broadcast_add(&mask)?;
        Ok(Heads {
            gate: self.gate.forward(&pooled)?.squeeze(D::Minus1)?,
            kind: self.kind.forward(&pooled)?,
            change: self.change.forward(&pooled)?.squeeze(D::Minus1)?,
            decision,
        })
    }

    /// Runs one unpadded prompt and keeps its keys and values.
    pub fn encode_prompt(&self, vq: &VqVae, ids: &[u32], device: &Device) -> Result<PromptCache> {
        let ids = Tensor::from_vec(ids.to_vec(), (1, ids.len()), device)?;
        let mut x = self.embed_prompt(vq, &ids)?;
        let mut kv = Vec::with_capacity(self.blocks.len());
        for block in &self.blocks {
            let (y, k, v) = block.forward_kv(&x)?;
            kv.push((k, v));
            x = y;
        }
        Ok(PromptCache {
            kv,
            hidden: self.norm.forward(&x)?,
            len: ids.dim(1)?,
        })
    }

    pub fn cached_heads(
        &self,
        cache: &PromptCache,
        option_spans: &[(usize, usize)],
    ) -> Result<Heads> {
        self.heads(&cache.hidden, &[cache.len], &[option_spans.to_vec()])
    }

    /// Generates one target, coarse to fine: one pass per level. Returns the target's token ids.
    pub fn generate<R: Rng>(
        &self,
        vq: &VqVae,
        cache: &PromptCache,
        sampling: Sampling,
        rng: &mut R,
    ) -> Result<Vec<u32>> {
        let device = cache.hidden.device();
        let mut acc = Tensor::zeros((1, self.target_len, self.code_dim), DType::F32, device)?;
        for (k, &l) in self.levels.iter().enumerate() {
            let mut x = self.level_input(k, l, &acc)?;
            for (block, (pk, pv)) in self.blocks.iter().zip(&cache.kv) {
                x = block.forward_with_prefix(&x, pk, pv)?;
            }
            let logits = self.code_heads[k].forward(&self.norm.forward(&x)?)?;
            let codes = pick(&logits, sampling, rng)?;
            let codes = Tensor::from_vec(codes, (1, l), device)?;
            acc = (acc + vq.code_embedding(k, &codes)?)?;
        }
        vq.tokens(&acc)
    }

    /// Generates one target per entry of `sampling` from the same prompt, all at once: the
    /// prompt's keys and values are shared and each level is a single pass for every row,
    /// instead of one pass per level and per candidate. Row `i` draws with `sampling[i]`.
    pub fn generate_many<R: Rng>(
        &self,
        vq: &VqVae,
        cache: &PromptCache,
        sampling: &[Sampling],
        rng: &mut R,
    ) -> Result<Vec<Vec<u32>>> {
        if sampling.len() <= 1 {
            return sampling
                .iter()
                .map(|&s| self.generate(vq, cache, s, rng))
                .collect();
        }
        Ok(self.generate_rows(vq, cache, sampling, rng)?.0)
    }

    /// [`Generator::generate_many`] for any number of rows, also returning the finest level's
    /// hidden states `(rows, L, d)`, which the copy head reads ([`crate::pointer`]). One row
    /// draws exactly as [`Generator::generate`] does.
    pub fn generate_rows<R: Rng>(
        &self,
        vq: &VqVae,
        cache: &PromptCache,
        sampling: &[Sampling],
        rng: &mut R,
    ) -> Result<(Vec<Vec<u32>>, Tensor)> {
        let rows = sampling.len().max(1);
        let device = cache.hidden.device();
        let kv = cache
            .kv
            .iter()
            .map(|(k, v)| Ok((k.repeat((rows, 1, 1, 1))?, v.repeat((rows, 1, 1, 1))?)))
            .collect::<Result<Vec<_>>>()?;
        let mut acc = Tensor::zeros((rows, self.target_len, self.code_dim), DType::F32, device)?;
        let mut last = None;
        for (k, &l) in self.levels.iter().enumerate() {
            let mut x = self.level_input(k, l, &acc)?;
            for (block, (pk, pv)) in self.blocks.iter().zip(&kv) {
                x = block.forward_with_prefix(&x, pk, pv)?;
            }
            let hidden = self.norm.forward(&x)?;
            let logits = self.code_heads[k].forward(&hidden)?;
            last = Some(hidden);
            let codes = if sampling.iter().all(|s| s.temperature <= 0.0) {
                logits.argmax(D::Minus1)?.flatten_all()?.to_vec1::<u32>()?
            } else {
                let mut codes = Vec::with_capacity(rows * l);
                for (i, &s) in sampling.iter().enumerate() {
                    codes.extend(pick(&logits.narrow(0, i, 1)?, s, rng)?);
                }
                codes
            };
            let codes = Tensor::from_vec(codes, (rows, l), device)?;
            acc = (acc + vq.code_embedding(k, &codes)?)?;
        }
        let ids = (0..rows)
            .map(|i| vq.tokens(&acc.narrow(0, i, 1)?))
            .collect::<Result<Vec<_>>>()?;
        let last = last.ok_or_else(|| candle_core::Error::Msg("no levels".into()))?;
        Ok((ids, last))
    }

    pub fn d_model(&self) -> usize {
        self.pos.dim(1).unwrap_or(0)
    }

    pub fn levels(&self) -> &[usize] {
        &self.levels
    }
    pub fn max_options(&self) -> usize {
        self.max_options
    }
}

/// Picks one code per position from `(1, ℓ, V)` logits.
fn pick<R: Rng>(logits: &Tensor, sampling: Sampling, rng: &mut R) -> Result<Vec<u32>> {
    if sampling.temperature <= 0.0 {
        return logits.argmax(D::Minus1)?.flatten_all()?.to_vec1::<u32>();
    }
    let rows = logits.squeeze(0)?.to_dtype(DType::F32)?.to_vec2::<f32>()?;
    let temperature = sampling.temperature;
    Ok(rows
        .into_iter()
        .map(|row| sample_row(&row, temperature, sampling.top_k, rng))
        .collect())
}

fn sample_row<R: Rng>(row: &[f32], temperature: f32, top_k: usize, rng: &mut R) -> u32 {
    let mut order: Vec<usize> = (0..row.len()).collect();
    if top_k > 0 && top_k < row.len() {
        order.select_nth_unstable_by(top_k - 1, |&a, &b| row[b].total_cmp(&row[a]));
        order.truncate(top_k);
    }
    let max = order
        .iter()
        .map(|&i| row[i])
        .fold(f32::NEG_INFINITY, f32::max);
    let weights: Vec<f64> = order
        .iter()
        .map(|&i| f64::from((row[i] - max) / temperature).exp())
        .collect();
    let total: f64 = weights.iter().sum();
    let mut target = rng.gen::<f64>() * total;
    for (&i, w) in order.iter().zip(&weights) {
        target -= w;
        if target <= 0.0 {
            return i as u32;
        }
    }
    order[order.len() - 1] as u32
}

/// `Σ_k w_k Σ_i CE(logits_k_i, codes_k_i)`, averaged over the batch (eq. 10).
pub fn generation_loss(logits: &[Tensor], codes: &[Tensor], weights: &[f64]) -> Result<Tensor> {
    let mut total: Option<Tensor> = None;
    for ((logit, code), &w) in logits.iter().zip(codes).zip(weights) {
        let (b, l, v) = logit.dims3()?;
        let ce = candle_nn::loss::cross_entropy(&logit.reshape((b * l, v))?, &code.flatten_all()?)?;
        let term = (ce * (w * l as f64))?;
        total = Some(match total {
            None => term,
            Some(t) => (t + term)?,
        });
    }
    total.ok_or_else(|| candle_core::Error::Msg("no levels".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use candle_nn::VarMap;
    use rand::SeedableRng;

    fn build() -> (LncmConfig, VarMap, VqVae, Generator) {
        let config = LncmConfig::tiny(272);
        let map = VarMap::new();
        let vb = VarBuilder::from_varmap(&map, DType::F32, &Device::Cpu);
        let vq = VqVae::new(&vb, &config).unwrap();
        let gen = Generator::new(&vb, &config).unwrap();
        (config, map, vq, gen)
    }

    #[test]
    fn heads_mask_absent_options_and_keep_abstain() {
        let (config, _map, vq, gen) = build();
        let rows = vec![vec![1u32, 2, 3, 4, 5, 6], vec![7u32, 8, 9]];
        let spans = vec![vec![(1, 3), (3, 5)], vec![(0, 2)]];
        let batch = PromptBatch::new(&rows, spans.clone(), 0, &Device::Cpu).unwrap();
        let hidden = gen.forward(&vq, &batch, None, Ctx::EVAL).unwrap();
        let heads = gen.heads(&hidden.prompt, &batch.lengths, &spans).unwrap();
        let d = heads.decision.to_vec2::<f32>().unwrap();
        assert_eq!(d[0].len(), config.max_options + 1);
        assert!(d[0][2] <= MASKED / 2.0 && d[0][3] <= MASKED / 2.0);
        assert!(d[1][1] <= MASKED / 2.0);
        assert!(d[0][config.max_options] > MASKED / 2.0);
        assert_eq!(heads.gate.dims(), &[2]);
        assert_eq!(heads.kind.dims(), &[2, 2]);
    }

    #[test]
    fn padding_does_not_change_a_prompt_and_cache_matches_batch() {
        let (_config, _map, vq, gen) = build();
        let alone = PromptBatch::new(&[vec![5u32, 6, 7]], vec![vec![]], 0, &Device::Cpu).unwrap();
        let padded = PromptBatch::new(
            &[vec![5u32, 6, 7], vec![1, 1, 1, 1, 1, 1]],
            vec![vec![], vec![]],
            0,
            &Device::Cpu,
        )
        .unwrap();
        let a = gen.forward(&vq, &alone, None, Ctx::EVAL).unwrap();
        let b = gen.forward(&vq, &padded, None, Ctx::EVAL).unwrap();
        let ha = gen
            .heads(&a.prompt, &alone.lengths, &alone.option_spans)
            .unwrap();
        let hb = gen
            .heads(&b.prompt, &padded.lengths, &padded.option_spans)
            .unwrap();
        let ga = ha.gate.to_vec1::<f32>().unwrap()[0];
        let gb = hb.gate.to_vec1::<f32>().unwrap()[0];
        assert!((ga - gb).abs() < 1e-5, "{ga} vs {gb}");
        let cache = gen.encode_prompt(&vq, &[5, 6, 7], &Device::Cpu).unwrap();
        let hc = gen.cached_heads(&cache, &[]).unwrap();
        assert!((hc.gate.to_vec1::<f32>().unwrap()[0] - ga).abs() < 1e-5);
    }

    #[test]
    fn generation_with_cache_matches_teacher_forced_pass() {
        let (config, _map, vq, gen) = build();
        let prompt = vec![3u32, 4, 5, 6];
        let cache = gen.encode_prompt(&vq, &prompt, &Device::Cpu).unwrap();
        let mut rng = rand::rngs::StdRng::seed_from_u64(1);
        let tokens = gen
            .generate(&vq, &cache, Sampling::GREEDY, &mut rng)
            .unwrap();
        assert_eq!(tokens.len(), config.target_len);
        // Feeding the generated codes back teacher-forced must reproduce the same argmax codes.
        let ids = Tensor::from_vec(tokens.clone(), (1, config.target_len), &Device::Cpu).unwrap();
        let _ = vq.codes_for(&ids).unwrap();
        let batch = PromptBatch::new(&[prompt], vec![vec![]], 0, &Device::Cpu).unwrap();
        // Rebuild the greedy codes level by level the way generate does, then compare one pass.
        let mut codes = Vec::new();
        let mut acc = Tensor::zeros(
            (1, config.target_len, config.code_dim),
            DType::F32,
            &Device::Cpu,
        )
        .unwrap();
        for (k, &l) in config.levels.iter().enumerate() {
            let mut x = gen.level_input(k, l, &acc).unwrap();
            for (block, (pk, pv)) in gen.blocks.iter().zip(&cache.kv) {
                x = block.forward_with_prefix(&x, pk, pv).unwrap();
            }
            let c = gen.code_heads[k]
                .forward(&gen.norm.forward(&x).unwrap())
                .unwrap()
                .argmax(D::Minus1)
                .unwrap();
            acc = (acc + vq.code_embedding(k, &c).unwrap()).unwrap();
            codes.push(c);
        }
        let inputs = gen.level_inputs(&vq, &codes).unwrap();
        let hidden = gen.forward(&vq, &batch, Some(&inputs), Ctx::EVAL).unwrap();
        let logits = gen.code_logits(&hidden.levels).unwrap();
        for (k, l) in logits.iter().enumerate() {
            let forced = l.argmax(D::Minus1).unwrap().to_vec2::<u32>().unwrap();
            assert_eq!(forced, codes[k].to_vec2::<u32>().unwrap(), "level {k}");
        }
    }

    #[test]
    fn batched_candidates_match_one_at_a_time() {
        let (_config, _map, vq, gen) = build();
        let cache = gen
            .encode_prompt(&vq, &[3u32, 4, 5, 6, 7], &Device::Cpu)
            .unwrap();
        let mut rng = rand::rngs::StdRng::seed_from_u64(3);
        let alone = gen
            .generate(&vq, &cache, Sampling::GREEDY, &mut rng)
            .unwrap();
        let many = gen
            .generate_many(&vq, &cache, &[Sampling::GREEDY; 3], &mut rng)
            .unwrap();
        assert_eq!(many.len(), 3);
        assert!(
            many.iter().all(|row| *row == alone),
            "greedy rows equal the single pass"
        );
        let sampled = Sampling {
            temperature: 1.0,
            top_k: 4,
        };
        let mixed = gen
            .generate_many(&vq, &cache, &[Sampling::GREEDY, sampled, sampled], &mut rng)
            .unwrap();
        assert_eq!(mixed[0], alone);
        assert!(mixed.iter().all(|row| row.len() == alone.len()));
    }

    #[test]
    fn sampling_respects_top_k_and_greedy() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(7);
        let row = [0.0f32, 10.0, 9.5, -3.0];
        for _ in 0..50 {
            let i = sample_row(&row, 1.0, 2, &mut rng);
            assert!(i == 1 || i == 2);
        }
        let logits = Tensor::new(&[[[0.0f32, 2.0, 1.0], [3.0, 0.0, 0.0]]], &Device::Cpu).unwrap();
        assert_eq!(
            pick(&logits, Sampling::GREEDY, &mut rng).unwrap(),
            vec![1, 0]
        );
    }
}
