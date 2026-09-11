#include "archie/visual/events.hpp"

#include <algorithm>

namespace archie::visual {

bool VisualEvent::is_deterioration() const noexcept {
    switch (kind) {
        case EventKind::SeverityChanged:
            return severity_to > severity_from;
        case EventKind::FreshnessChanged:
            // The enum is ordered from most to least trustworthy, so a larger value is
            // a worse one.
            return static_cast<int>(freshness_to) > static_cast<int>(freshness_from);
        case EventKind::Departed:
        case EventKind::Removed:
            return true;
        case EventKind::Appeared:
        case EventKind::Returned:
            return false;
    }
    return false;
}

void EventQueue::push(VisualEvent event) {
    // Coalesce against a pending event for the same node and the same kind. A node
    // whose severity walks 1 -> 2 -> 5 between two frames had one thing happen to it,
    // and the truthful summary keeps the original starting point.
    const auto existing = std::find_if(m_pending.begin(), m_pending.end(),
                                       [&](const VisualEvent& pending) {
                                           return pending.kind == event.kind &&
                                                  pending.node_id == event.node_id;
                                       });

    if (existing != m_pending.end()) {
        existing->at_ms = event.at_ms;

        // Only the field this kind of event is about is carried forward. The starting
        // value is deliberately left alone: it is what makes the merged event a true
        // summary of the whole run rather than of its last step.
        switch (existing->kind) {
            case EventKind::SeverityChanged:
                existing->severity_to = event.severity_to;
                if (existing->severity_from == existing->severity_to) {
                    // Back where it started, so nothing changed. Flashing for it would
                    // pulse the display over a number identical to the one on screen.
                    m_pending.erase(existing);
                }
                break;
            case EventKind::FreshnessChanged:
                existing->freshness_to = event.freshness_to;
                if (existing->freshness_from == existing->freshness_to) {
                    m_pending.erase(existing);
                }
                break;
            case EventKind::Appeared:
            case EventKind::Departed:
            case EventKind::Returned:
            case EventKind::Removed:
                // Presence events carry no value; the newer timestamp is the whole
                // update.
                break;
        }
        return;
    }

    if (m_pending.size() >= m_capacity) {
        // Drop the oldest. The newest events are the ones an operator is looking for,
        // and a queue that dropped those instead would go blind exactly when the
        // machine got busy.
        m_pending.pop_front();
        ++m_dropped;
    }

    m_pending.push_back(std::move(event));
}

void EventQueue::observe(const GraphDelta& delta, std::int64_t now_ms) {
    for (const std::string& id : delta.added) {
        VisualEvent event{};
        event.kind = EventKind::Appeared;
        event.node_id = id;
        event.at_ms = now_ms;
        push(std::move(event));
    }

    for (const std::string& id : delta.returned) {
        VisualEvent event{};
        event.kind = EventKind::Returned;
        event.node_id = id;
        event.at_ms = now_ms;
        push(std::move(event));
    }

    for (const NodeChange& change : delta.updated) {
        // Only genuine severity and freshness movement earns a pulse. A node whose
        // label or dependency list changed has changed, and the renderer will redraw
        // it, but flashing the screen for it would spend the operator's attention on
        // something that does not need it.
        if (change.severity_from != change.severity_to) {
            VisualEvent event{};
            event.kind = EventKind::SeverityChanged;
            event.node_id = change.id;
            event.at_ms = now_ms;
            event.severity_from = change.severity_from;
            event.severity_to = change.severity_to;
            push(std::move(event));
        }
        if (change.freshness_from != change.freshness_to) {
            VisualEvent event{};
            event.kind = EventKind::FreshnessChanged;
            event.node_id = change.id;
            event.at_ms = now_ms;
            event.freshness_from = change.freshness_from;
            event.freshness_to = change.freshness_to;
            push(std::move(event));
        }
    }

    for (const std::string& id : delta.departing) {
        VisualEvent event{};
        event.kind = EventKind::Departed;
        event.node_id = id;
        event.at_ms = now_ms;
        push(std::move(event));
    }

    for (const std::string& id : delta.removed) {
        VisualEvent event{};
        event.kind = EventKind::Removed;
        event.node_id = id;
        event.at_ms = now_ms;
        push(std::move(event));
    }
}

EventBatch EventQueue::drain() {
    EventBatch batch{};
    batch.events.assign(m_pending.begin(), m_pending.end());
    batch.dropped = m_dropped;
    m_pending.clear();
    m_dropped = 0;
    return batch;
}

}  // namespace archie::visual
