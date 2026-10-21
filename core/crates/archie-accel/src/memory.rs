//! VRAM: who holds how much, and which MoE experts stay resident.
//!
//! The 3070 has 8 GiB for the desktop, IRIS, the LLM and Wanda's LNCM (§2.4 of the plan:
//! about 7.8 to 8.3 GiB if everything sits on it at once). The ledger refuses a reservation
//! past the budget Policy set, so a model falls back (fewer LLM layers on the GPU, the LNCM
//! on the CPU) instead of the driver failing an allocation mid-frame.
//!
//! Experts are 57 % of IRIS's weights. With room, all stay resident (exact). Under
//! pressure, each layer keeps its most used experts for the current mix of sources; a
//! non-resident expert behaves as one at capacity 0, the overflow path the model was
//! trained with. That changes results, so it only happens when Policy's envelope allows it
//! and a quality floor holds, and it is measured by A/B before anyone relies on it.

use crate::Error;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VramLedger {
    budget: u64,
    held: BTreeMap<String, u64>,
}

impl VramLedger {
    pub fn new(budget: u64) -> Self {
        Self {
            budget,
            held: BTreeMap::new(),
        }
    }

    pub fn used(&self) -> u64 {
        self.held.values().sum()
    }

    pub fn free(&self) -> u64 {
        self.budget.saturating_sub(self.used())
    }

    /// Sets what `holder` holds, if the total stays within the budget.
    pub fn reserve(&mut self, holder: &str, bytes: u64) -> Result<(), Error> {
        let others = self.used() - self.held.get(holder).copied().unwrap_or(0);
        if others + bytes > self.budget {
            return Err(Error::Memory(format!(
                "{holder} asks {bytes} bytes; {} of {} are free",
                self.budget.saturating_sub(others),
                self.budget
            )));
        }
        self.held.insert(holder.to_owned(), bytes);
        Ok(())
    }

    pub fn release(&mut self, holder: &str) {
        self.held.remove(holder);
    }

    pub fn holders(&self) -> &BTreeMap<String, u64> {
        &self.held
    }
}

/// Which experts stay on the GPU, per layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Residency {
    pub resident: Vec<Vec<bool>>,
    /// All experts resident: results are exact.
    pub exact: bool,
}

/// Residency for `usage[layer][expert]` (tokens routed, from recent frames) within
/// `budget_bytes` of expert weights. Everything fits → all resident. Otherwise, only if
/// `allow_partial`: each layer keeps the same count of its most used experts (ties to the
/// lower index), never fewer than `floor` of them; below the floor it refuses.
pub fn plan_residency(
    usage: &[Vec<u64>],
    expert_bytes: u64,
    budget_bytes: u64,
    allow_partial: bool,
    floor: f32,
) -> Result<Residency, Error> {
    let layers = usage.len();
    let experts = usage.first().map_or(0, Vec::len);
    if usage.iter().any(|layer| layer.len() != experts) {
        return Err(Error::Memory(
            "layers route over different expert counts".into(),
        ));
    }
    let total = (layers * experts) as u64 * expert_bytes;
    if total <= budget_bytes {
        return Ok(Residency {
            resident: vec![vec![true; experts]; layers],
            exact: true,
        });
    }
    if !allow_partial {
        return Err(Error::Memory(format!(
            "experts need {total} bytes, {budget_bytes} allowed, and partial residency is not"
        )));
    }
    let per_layer = (budget_bytes / expert_bytes.max(1) / layers.max(1) as u64) as usize;
    let minimum = (floor.clamp(0.0, 1.0) * experts as f32).ceil() as usize;
    if per_layer < minimum.max(1) {
        return Err(Error::Memory(format!(
            "{per_layer} experts per layer fit, the quality floor needs {minimum}"
        )));
    }
    let resident = usage
        .iter()
        .map(|layer| {
            let mut order: Vec<usize> = (0..experts).collect();
            order.sort_by(|&a, &b| layer[b].cmp(&layer[a]).then(a.cmp(&b)));
            let mut keep = vec![false; experts];
            for &expert in order.iter().take(per_layer.min(experts)) {
                keep[expert] = true;
            }
            keep
        })
        .collect();
    Ok(Residency {
        resident,
        exact: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1 << 30;
    const MIB: u64 = 1 << 20;

    #[test]
    fn the_ledger_refuses_past_the_budget_and_lets_a_holder_resize() {
        let mut ledger = VramLedger::new(8 * GIB);
        ledger.reserve("desktop", 1400 * MIB).unwrap();
        ledger.reserve("iris", 3 * GIB).unwrap();
        ledger.reserve("llm", 2 * GIB).unwrap();
        assert!(ledger.reserve("llm-kv", 2 * GIB).is_err());
        ledger.reserve("iris", 2400 * MIB).unwrap();
        assert_eq!(ledger.used(), 1400 * MIB + 2400 * MIB + 2 * GIB);
        ledger.release("llm");
        assert_eq!(ledger.holders().len(), 2);
    }

    #[test]
    fn residency_is_exact_with_room_and_keeps_the_most_used_otherwise() {
        let usage = vec![vec![5, 0, 9, 9, 1, 3], vec![0, 0, 0, 7, 7, 7]];
        let all = plan_residency(&usage, 4 * MIB, 48 * MIB, false, 0.5).unwrap();
        assert!(all.exact);
        assert!(plan_residency(&usage, 4 * MIB, 40 * MIB, false, 0.5).is_err());
        let partial = plan_residency(&usage, 4 * MIB, 32 * MIB, true, 0.5).unwrap();
        assert!(!partial.exact);
        // Four of six per layer: the most used, ties to the lower index.
        assert_eq!(
            partial.resident[0],
            vec![true, false, true, true, false, true]
        );
        assert_eq!(
            partial.resident[1],
            vec![true, false, false, true, true, true]
        );
        assert!(
            plan_residency(&usage, 4 * MIB, 16 * MIB, true, 0.5).is_err(),
            "below the floor"
        );
    }
}
