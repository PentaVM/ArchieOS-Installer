#include "sentry/world/provenance.hpp"

#include <algorithm>

namespace sentry::world {

std::string_view name(Derivation derivation) noexcept {
    switch (derivation) {
        case Derivation::Fetched:    return "fetched";
        case Derivation::Parsed:     return "parsed";
        case Derivation::Normalized: return "normalized";
        case Derivation::Derived:    return "derived";
        case Derivation::Syndicated: return "syndicated";
    }
    return "unknown";
}

void ProvenanceGraph::add(Provenance record) {
    auto id = record.record_id;
    records_.insert_or_assign(std::move(id), std::move(record));
}

const Provenance* ProvenanceGraph::find(const RecordId& id) const noexcept {
    const auto it = records_.find(id);
    return it == records_.end() ? nullptr : &it->second;
}

void ProvenanceGraph::collect_roots(const RecordId& id,
                                    std::unordered_set<RecordId>& visited,
                                    std::unordered_set<std::string>& out) const {
    // Provenance should be acyclic, but it is assembled from provider metadata and a
    // provider that credits itself in a loop must not hang the fusion engine.
    if (!visited.insert(id).second) {
        return;
    }

    const auto* record = find(id);
    if (record == nullptr) {
        // A parent we have no record for. It is still a distinct origin — dropping it
        // would silently reduce the source count, which errs in the dangerous direction:
        // fewer counted sources means *harder* to corroborate, but an unknown parent
        // could equally be a duplicate of one we already have. Counting it as itself
        // keeps the answer honest about what we actually know.
        out.insert(id);
        return;
    }

    if (record->derivation == Derivation::Syndicated) {
        // A copy is not a source. Resolve to what it credits; if it credits nothing,
        // fall through to counting the outlet itself — an uncredited copy might be
        // original reporting, and refusing to count it would discard real evidence.
        if (!record->attributed_origin.empty()) {
            out.insert(record->attributed_origin);
            return;
        }
        if (record->parents.empty()) {
            out.insert(record->provider_id);
            return;
        }
    }

    if (record->parents.empty()) {
        // A root. The provider is the source, unless it named an upstream it is relaying.
        out.insert(record->attributed_origin.empty() ? record->provider_id
                                                     : record->attributed_origin);
        return;
    }

    for (const auto& parent : record->parents) {
        collect_roots(parent, visited, out);
    }
}

std::vector<std::string> ProvenanceGraph::independent_sources(const RecordId& id) const {
    return independent_sources(std::vector<RecordId>{id});
}

std::vector<std::string> ProvenanceGraph::independent_sources(
    const std::vector<RecordId>& ids) const {
    std::unordered_set<std::string> roots;
    // One visited set across the whole batch, so two records sharing an ancestor do not
    // each get to count it. That sharing is exactly what fusion is trying to detect.
    std::unordered_set<RecordId> visited;
    for (const auto& id : ids) {
        collect_roots(id, visited, roots);
    }

    std::vector<std::string> sorted(roots.begin(), roots.end());
    // Sorted so the answer is stable across runs: an unordered_set's iteration order is
    // an implementation detail, and a provenance explanation shown to an operator that
    // reorders itself between refreshes looks like the data changed.
    std::ranges::sort(sorted);
    return sorted;
}

}  // namespace sentry::world
