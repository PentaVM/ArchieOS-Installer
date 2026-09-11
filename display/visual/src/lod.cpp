#include "archie/visual/lod.hpp"

#include <algorithm>
#include <limits>

namespace archie::visual {
namespace {

/// Importance, on a scale where one whole unit is one severity level.
///
/// Magnitude is folded in underneath as a tie-break rather than added raw, because the
/// two are not comparable: a magnitude of 7 is not seven times as urgent as severity 1,
/// and letting an unbounded provider number outrank severity would mean a large quiet
/// reading pushing a small critical one off the screen. Mapping it through
/// `m / (1 + m)` puts it strictly inside [0, 1), so it can order nodes of equal
/// severity and can never reach past them.
double importance(const LodCandidate& candidate) noexcept {
    const double magnitude = candidate.magnitude > 0.0F ? static_cast<double>(candidate.magnitude) : 0.0;
    return static_cast<double>(candidate.severity) + magnitude / (1.0 + magnitude);
}

struct Ranked {
    const LodCandidate* candidate = nullptr;
    double score = 0.0;
};

}  // namespace

DetailLevel detail_for(float distance, bool focused, const LodPolicy& policy) noexcept {
    if (focused) {
        // The operator asked for this one. It gets full detail even while the camera is
        // still on its way, because the panel beside it is already filling in.
        return DetailLevel::Inspect;
    }
    if (distance <= policy.inspect_within) {
        return DetailLevel::Inspect;
    }
    if (distance <= policy.full_within) {
        return DetailLevel::Full;
    }
    if (distance <= policy.simplified_within) {
        return DetailLevel::Simplified;
    }
    return DetailLevel::Silhouette;
}

LodPlan LodPlanner::plan(const std::vector<LodCandidate>& candidates, int budget,
                         const std::string& focused) {
    LodPlan plan{};

    const int cap = budget > 0 ? budget : 0;
    if (cap == 0) {
        // Headless, or a tier with nothing to spend. Everything is aggregated rather
        // than forgotten, and the incumbency record is cleared so that whatever comes
        // back does so on merit rather than on a claim from before the lights went out.
        plan.aggregated = static_cast<int>(candidates.size());
        m_visible.clear();
        return plan;
    }

    std::vector<Ranked> ranked;
    ranked.reserve(candidates.size());
    for (const LodCandidate& candidate : candidates) {
        double score = importance(candidate);

        if (!focused.empty() && candidate.id == focused) {
            // Never evicted, for any reason. Dropping the node an operator is reading
            // because something elsewhere got worse is the display deciding what they
            // should be looking at.
            score = std::numeric_limits<double>::infinity();
        } else if (m_visible.count(candidate.id) != 0) {
            score += m_policy.incumbency_margin;
        }

        ranked.push_back(Ranked{&candidate, score});
    }

    // Deterministic: score first, then id. Ties broken by anything else — insertion
    // order, a hash, pointer value — would let the visible set depend on payload
    // ordering, which is the flicker this class exists to prevent wearing a disguise.
    std::stable_sort(ranked.begin(), ranked.end(), [](const Ranked& a, const Ranked& b) {
        if (a.score != b.score) {
            return a.score > b.score;
        }
        return a.candidate->id < b.candidate->id;
    });

    const std::size_t take = std::min(static_cast<std::size_t>(cap), ranked.size());
    std::set<std::string> now_visible;
    plan.visible.reserve(take);

    for (std::size_t i = 0; i < take; ++i) {
        const LodCandidate& candidate = *ranked[i].candidate;
        const bool is_focused = !focused.empty() && candidate.id == focused;
        plan.visible.push_back(
            VisibleNode{candidate.id, detail_for(candidate.distance, is_focused, m_policy)});
        now_visible.insert(candidate.id);
    }

    plan.aggregated = static_cast<int>(ranked.size() - take);
    m_visible = std::move(now_visible);
    return plan;
}

}  // namespace archie::visual
