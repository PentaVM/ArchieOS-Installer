#pragma once

// Things that happened, in a queue that cannot grow without bound.
//
// Events here are *derived*, never invented. Every one of them comes out of a
// `GraphDelta` — the same comparison the scene graph already ran to decide what
// changed — so there is no second opinion about whether something happened, and nothing
// in this file can manufacture a pulse for which no state change exists. An empty delta
// produces an empty queue, and that is a tested property rather than a convention.
//
// Two failure modes shape the rest.
//
// The first is unbounded growth. A node that loses its provider can change state on
// every poll, and a hundred nodes doing that fills memory with pulses nobody will ever
// watch. So the queue has a hard capacity, drops the *oldest* when full — the newest
// events are the ones an operator is actually looking for — and counts what it dropped.
// It never silently discards: a batch that lost events says so, so the interface can
// admit to a gap rather than presenting a partial history as a complete one.
//
// The second is the storm. A node whose severity walks 1 -> 2 -> 5 between two frames
// did not have three things happen to it; it had one, and the honest summary is "rose
// from 1 to 5". Coalescing per node and per kind keeps that true, and it means a
// flapping subsystem costs one pulse rather than one per poll. A node that ends a frame
// exactly where it started did not change at all, and its event is dropped outright —
// otherwise the display would flash for a value that is identical to the one already
// on screen.

#include <cstddef>
#include <cstdint>
#include <deque>
#include <string>
#include <vector>

#include "archie/visual/freshness.hpp"
#include "archie/visual/graph.hpp"

namespace archie::visual {

enum class EventKind {
    /// A node the scene has not seen before.
    Appeared,
    /// Missing from the payload; drawn dimmed while its grace runs.
    Departed,
    /// Missing, then back, inside its grace. Not an arrival.
    Returned,
    /// Grace expired. The renderer may release the geometry.
    Removed,
    /// Severity moved. Carries both ends, so the pulse can say which way.
    SeverityChanged,
    /// The data behind a node got older or fresher.
    FreshnessChanged,
};

struct VisualEvent {
    EventKind kind = EventKind::Appeared;
    std::string node_id;
    std::int64_t at_ms = 0;

    /// Meaningful for `SeverityChanged`.
    int severity_from = 0;
    int severity_to = 0;

    /// Meaningful for `FreshnessChanged`.
    Freshness freshness_from = Freshness::Live;
    Freshness freshness_to = Freshness::Live;

    /// Whether this reads as the situation getting worse. Used to pick the pulse's
    /// colour without the renderer having to re-derive the judgement.
    [[nodiscard]] bool is_deterioration() const noexcept;
};

/// One handover to the renderer.
struct EventBatch {
    std::vector<VisualEvent> events;

    /// How many events were discarded to keep the queue inside its capacity. Non-zero
    /// means the interface is showing an incomplete history and should say so.
    int dropped = 0;

    [[nodiscard]] bool empty() const noexcept { return events.empty() && dropped == 0; }
};

/// A bounded, coalescing queue of derived events.
class EventQueue final {
public:
    EventQueue() = default;
    explicit EventQueue(std::size_t capacity) : m_capacity(capacity == 0 ? 1 : capacity) {}

    /// Derive events from one reconciliation. The only way to put anything in.
    void observe(const GraphDelta& delta, std::int64_t now_ms);

    /// Take everything pending and reset the drop counter.
    EventBatch drain();

    [[nodiscard]] std::size_t size() const noexcept { return m_pending.size(); }
    [[nodiscard]] std::size_t capacity() const noexcept { return m_capacity; }
    [[nodiscard]] int dropped() const noexcept { return m_dropped; }

private:
    void push(VisualEvent event);

    std::size_t m_capacity = 256;
    std::deque<VisualEvent> m_pending{};
    int m_dropped = 0;
};

}  // namespace archie::visual
