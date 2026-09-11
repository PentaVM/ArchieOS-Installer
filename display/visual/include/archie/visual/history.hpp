#pragma once

// What the scene looked like a moment ago, so an operator can go back and see it.
//
// The question a timeline answers is "what was true at 14:02", and the failure mode is
// answering a slightly different question convincingly. Two rules follow from that.
//
// **Seek returns what was in force, not what was nearest.** If the ring holds entries
// at 14:00 and 14:05 and the operator asks for 14:02, the answer is the 14:00 entry —
// that is the state the machine was actually in. Returning 14:05 because it is closer
// in time would show them a future they are asking to look behind.
//
// **A time before the recording began has no answer.** Not the earliest entry: the
// display genuinely does not know what was happening then, and handing back the oldest
// state would claim it held for a period nobody observed.
//
// Memory is bounded, and what is stored is deliberately thin — per-node severity,
// freshness and presence, not geometry or camera. A timeline is for reading what
// changed; re-flying the camera through history would need the whole scene and buy
// nothing an operator asked for.
//
// Sampling is change-driven with a floor. A still scene ticked sixty times a second
// must not fill the ring with sixty identical entries a second, and a change must not
// have to wait out an interval to be recorded — so an entry is kept when the state
// differs from the last one, or when enough time has passed to be worth a keyframe.

#include <cstddef>
#include <cstdint>
#include <deque>
#include <string>
#include <utility>
#include <vector>

#include "archie/visual/budget.hpp"
#include "archie/visual/freshness.hpp"
#include "archie/visual/graph.hpp"
#include "archie/visual/scene.hpp"

namespace archie::visual {

/// One node as history remembers it. Everything an operator would read off a timeline,
/// and nothing that only matters to a renderer.
struct NodeSample {
    std::string id;
    int severity = 0;
    Freshness freshness = Freshness::Live;
    NodePresence presence = NodePresence::Present;

    friend bool operator==(const NodeSample& a, const NodeSample& b) noexcept {
        return a.id == b.id && a.severity == b.severity && a.freshness == b.freshness &&
               a.presence == b.presence;
    }
};

struct HistoryEntry {
    std::int64_t at_ms = 0;
    RenderTier tier = RenderTier::Normal;
    TierReason tier_reason = TierReason::TelemetryUnavailable;
    SceneMode mode = SceneMode::Cerebra;
    int total_nodes = 0;
    int aggregated = 0;
    std::vector<NodeSample> nodes;

    /// Whether this entry differs from `other` in any way worth recording.
    [[nodiscard]] bool differs_from(const HistoryEntry& other) const noexcept;
};

struct HistoryPolicy {
    std::size_t capacity = 600;

    /// The longest gap between entries in a scene that is not changing. A keyframe at
    /// this interval keeps the timeline from having long stretches with nothing in them,
    /// which would make a scrub through a quiet hour look like a jump.
    std::int64_t keyframe_interval_ms = 5'000;
};

/// A bounded, monotonic timeline of scene state.
class History final {
public:
    History() = default;
    explicit History(HistoryPolicy policy)
        : m_policy(policy.capacity == 0 ? HistoryPolicy{1, policy.keyframe_interval_ms}
                                        : policy) {}

    /// Offer an entry. Returns whether it was kept.
    ///
    /// Entries at or before the newest are refused rather than inserted out of order: a
    /// timeline that is not monotonic cannot be seeked, and silently reordering would
    /// make the answer to "what was true at 14:02" depend on the order things arrived.
    bool record(HistoryEntry entry);

    /// The state in force at `when_ms`, or null if that is before the recording began.
    [[nodiscard]] const HistoryEntry* at_or_before(std::int64_t when_ms) const noexcept;

    [[nodiscard]] const HistoryEntry* newest() const noexcept;
    [[nodiscard]] const HistoryEntry* oldest() const noexcept;

    /// The interval the timeline can answer for. Both zero when empty.
    [[nodiscard]] std::pair<std::int64_t, std::int64_t> span() const noexcept;

    [[nodiscard]] std::size_t size() const noexcept { return m_entries.size(); }
    [[nodiscard]] std::size_t capacity() const noexcept { return m_policy.capacity; }

    /// Entries dropped to stay inside capacity. The interface should say when a scrub
    /// cannot reach as far back as the operator expects.
    [[nodiscard]] int evicted() const noexcept { return m_evicted; }

    /// Entries refused for arriving out of order. Non-zero means a clock moved
    /// backwards, which is worth surfacing rather than absorbing.
    [[nodiscard]] int out_of_order() const noexcept { return m_out_of_order; }

private:
    HistoryPolicy m_policy{};
    std::deque<HistoryEntry> m_entries{};
    int m_evicted = 0;
    int m_out_of_order = 0;
};

}  // namespace archie::visual
