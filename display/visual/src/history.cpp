#include "archie/visual/history.hpp"

#include <algorithm>

namespace archie::visual {

bool HistoryEntry::differs_from(const HistoryEntry& other) const noexcept {
    // Deliberately not comparing `at_ms`: every entry differs in its timestamp, so
    // including it would make this always true and defeat the whole point of asking.
    return tier != other.tier || tier_reason != other.tier_reason || mode != other.mode ||
           total_nodes != other.total_nodes || aggregated != other.aggregated ||
           nodes != other.nodes;
}

bool History::record(HistoryEntry entry) {
    if (!m_entries.empty()) {
        const HistoryEntry& newest_entry = m_entries.back();

        if (entry.at_ms <= newest_entry.at_ms) {
            // A timeline that is not monotonic cannot be seeked: `at_or_before` relies on
            // the entries being ordered, and inserting here would make the answer to
            // "what was true at 14:02" depend on the order things happened to arrive.
            ++m_out_of_order;
            return false;
        }

        const bool changed = entry.differs_from(newest_entry);
        const bool due = entry.at_ms - newest_entry.at_ms >= m_policy.keyframe_interval_ms;
        if (!changed && !due) {
            // A still scene ticked sixty times a second must not fill the ring with sixty
            // identical entries a second. Nothing is lost: the newest entry still
            // describes the state, and it remains in force until something replaces it.
            return false;
        }
    }

    if (m_entries.size() >= m_policy.capacity) {
        m_entries.pop_front();
        ++m_evicted;
    }
    m_entries.push_back(std::move(entry));
    return true;
}

const HistoryEntry* History::at_or_before(std::int64_t when_ms) const noexcept {
    if (m_entries.empty() || when_ms < m_entries.front().at_ms) {
        // Before anything was recorded. Handing back the oldest entry would claim that
        // state held for a period nobody observed.
        return nullptr;
    }

    // The last entry whose timestamp is at or before the moment asked about — the state
    // actually in force then, rather than whichever entry is nearest in time.
    const auto after = std::upper_bound(
        m_entries.begin(), m_entries.end(), when_ms,
        [](std::int64_t moment, const HistoryEntry& candidate) {
            return moment < candidate.at_ms;
        });
    return &*(after - 1);
}

const HistoryEntry* History::newest() const noexcept {
    return m_entries.empty() ? nullptr : &m_entries.back();
}

const HistoryEntry* History::oldest() const noexcept {
    return m_entries.empty() ? nullptr : &m_entries.front();
}

std::pair<std::int64_t, std::int64_t> History::span() const noexcept {
    if (m_entries.empty()) {
        return {0, 0};
    }
    return {m_entries.front().at_ms, m_entries.back().at_ms};
}

}  // namespace archie::visual
