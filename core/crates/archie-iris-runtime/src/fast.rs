//! Element-wise kernels spread over every CPU core.
//!
//! candle runs GELU and softmax on one thread on the CPU, which made them most of a frame's
//! time once the matrix products were fast. These are the same formulas, in the same f32
//! arithmetic, split by rows across rayon's pool. Any other device or dtype goes to candle.

use crate::nn::Result;
use candle_core::{DType, Device, Tensor, D};
use rayon::prelude::*;

const SQRT_TWO_OVER_PI: f32 = 0.797_884_6;

fn on_cpu_f32(x: &Tensor) -> bool {
    matches!(x.device(), Device::Cpu) && x.dtype() == DType::F32
}

fn map(x: &Tensor, f: impl Fn(f32) -> f32 + Sync) -> Result<Tensor> {
    let mut values: Vec<f32> = x.flatten_all()?.to_vec1()?;
    values
        .par_chunks_mut(16 * 1024)
        .for_each(|chunk| chunk.iter_mut().for_each(|v| *v = f(*v)));
    Ok(Tensor::from_vec(values, x.shape(), x.device())?)
}

/// Exact GELU, `0.5 x (1 + erf(x / sqrt 2))`.
pub fn gelu_erf(x: &Tensor) -> Result<Tensor> {
    if !on_cpu_f32(x) {
        return Ok(x.gelu_erf()?);
    }
    map(x, |v| {
        (candle_core::cpu::erf::erf_f32(v * std::f32::consts::FRAC_1_SQRT_2) + 1.0) * 0.5 * v
    })
}

/// GELU with the tanh approximation (`gelu_pytorch_tanh`).
pub fn gelu_tanh(x: &Tensor) -> Result<Tensor> {
    if !on_cpu_f32(x) {
        return Ok(x.gelu()?);
    }
    map(x, |v| {
        0.5 * v * (1.0 + (SQRT_TWO_OVER_PI * v * (1.0 + 0.044715 * v * v)).tanh())
    })
}

/// Softmax over the last dimension, rows in parallel.
pub fn softmax_last_dim(x: &Tensor) -> Result<Tensor> {
    if !on_cpu_f32(x) {
        return Ok(candle_nn::ops::softmax_last_dim(x)?);
    }
    let width = x.dim(D::Minus1)?;
    let mut values: Vec<f32> = x.flatten_all()?.to_vec1()?;
    values.par_chunks_mut(width).for_each(|row| {
        let max = row.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let mut total = 0.0;
        for v in row.iter_mut() {
            *v = (*v - max).exp();
            total += *v;
        }
        for v in row.iter_mut() {
            *v /= total;
        }
    });
    Ok(Tensor::from_vec(values, x.shape(), x.device())?)
}

/// Rotary embedding over `[B, H, L, W]` with `[L, W]` tables: each pair `(a, b)` becomes
/// `(a * cos - b * sin, b * cos + a * sin)`, multiplied and added in the order candle's
/// broadcast ops use, so the result is theirs to the bit.
pub fn rope(x: &Tensor, cos: &[f32], sin: &[f32]) -> Result<Tensor> {
    let (_, _, length, width) = x.dims4()?;
    let plane = length * width;
    let mut values: Vec<f32> = x.flatten_all()?.to_vec1()?;
    values.par_chunks_mut(plane).for_each(|block| {
        for (row, chunk) in block.chunks_exact_mut(width).enumerate() {
            let c = &cos[row * width..(row + 1) * width];
            let s = &sin[row * width..(row + 1) * width];
            for pair in 0..width / 2 {
                let (i, j) = (2 * pair, 2 * pair + 1);
                let (a, b) = (chunk[i], chunk[j]);
                chunk[i] = a * c[i] + (-b) * s[i];
                chunk[j] = b * c[j] + a * s[j];
            }
        }
    });
    Ok(Tensor::from_vec(values, x.shape(), x.device())?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_parallel_kernels_agree_with_candle() {
        let x = Tensor::randn(0f32, 2.0, (37, 129), &Device::Cpu).unwrap();
        let close = |a: Tensor, b: Tensor| {
            let d: f32 = (a - b)
                .unwrap()
                .abs()
                .unwrap()
                .max_all()
                .unwrap()
                .to_scalar()
                .unwrap();
            assert!(d < 1e-6, "{d}");
        };
        close(gelu_erf(&x).unwrap(), x.gelu_erf().unwrap());
        close(gelu_tanh(&x).unwrap(), x.gelu().unwrap());
        close(
            softmax_last_dim(&x).unwrap(),
            candle_nn::ops::softmax_last_dim(&x).unwrap(),
        );
    }
}
