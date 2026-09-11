// Provenance — where a statement came from, and how many sources that really is.
//
// RFC: docs/RFC-008-SENTRY-WORLD-MODEL.md §Provenance
//
// The corroboration floor in `epistemic.hpp` takes a count of independent sources. Until
// this header existed, that count was an integer a caller passed in — which made the
// most important rule in the fusion engine a matter of trust in whoever was calling it.
//
// Independence is a property of the *provenance chain*, not of the number of documents
// in front of you. Four newspapers carrying one wire story are four records and one
// source. A satellite product and an analysis derived from that same product are two
// artifacts and one observation. Counting rows finds four and two; walking to the roots
// finds one and one, which is the answer that matters.
//
// This is the MizarVision lesson stated in code: fusing genuinely independent public
// observations is powerful, and fusing one observation with three copies of itself is
// self-deception wearing the same shape.

#pragma once

#include <cstddef>
#include <string>
#include <string_view>
#include <unordered_map>
#include <unordered_set>
#include <vector>

#include "sentry/world/time.hpp"

namespace sentry::world {

// A stable identifier for one record inside Sentry.
using RecordId = std::string;

// How this record came to exist.
enum class Derivation {
    // Fetched directly from a provider. This is a root.
    Fetched,
    // Structurally decoded from a fetched record.
    Parsed,
    // Mapped onto Sentry's types.
    Normalized,
    // Computed from one or more other records by an analytic.
    Derived,
    // Republished from another source. A syndicated wire story, a mirrored dataset, an
    // aggregator relaying someone else's feed.
    //
    // This is the one that makes independence counting non-trivial: a syndicated record
    // has its own id, its own fetch time and its own provider, and is not an independent
    // source of anything.
    Syndicated,
};

[[nodiscard]] std::string_view name(Derivation derivation) noexcept;

// One node in the provenance graph.
struct Provenance {
    RecordId record_id{};
    // Which provider handed us this record. For a syndicated copy this is the outlet
    // that republished it, not the wire service that wrote it.
    std::string provider_id{};
    Derivation derivation = Derivation::Fetched;
    // Records this one was derived or copied from. Empty for a root.
    std::vector<RecordId> parents{};
    // Where a syndicated record says it came from, when it says. An outlet crediting
    // "Reuters" gives us the root without our having to fetch Reuters.
    std::string attributed_origin{};
    TemporalExtent time{};
    // Which analytic produced this, so a regression traces to a version.
    std::string agent_version{};

    [[nodiscard]] bool is_root() const noexcept {
        return parents.empty() && derivation == Derivation::Fetched;
    }
};

// The provenance graph for a set of records.
//
// Deliberately not a general graph library. It answers one question — how many genuinely
// independent sources stand behind this statement — and refusing to grow past that keeps
// the answer auditable.
class ProvenanceGraph {
public:
    // Add a record. Later additions with the same id replace earlier ones, so a
    // re-normalization does not create a phantom second source.
    void add(Provenance record);

    [[nodiscard]] const Provenance* find(const RecordId& id) const noexcept;
    [[nodiscard]] std::size_t size() const noexcept { return records_.size(); }

    // The distinct originating sources behind `id`, walking every parent to its root.
    //
    // A syndicated record resolves to what it credits when it credits anything: an
    // outlet republishing Reuters counts as Reuters, so ten outlets carrying the same
    // wire story collapse to one. When it credits nothing, it resolves to the outlet
    // itself — which is the conservative answer, since an uncredited copy might be
    // original reporting and refusing to count it at all would lose real evidence.
    [[nodiscard]] std::vector<std::string> independent_sources(const RecordId& id) const;

    // How many. This is the number `epistemic::Transition` needs, and computing it here
    // rather than accepting it from a caller is the whole point of this file.
    [[nodiscard]] int independent_source_count(const RecordId& id) const {
        return static_cast<int>(independent_sources(id).size());
    }

    // The same question for several records at once — the shape fusion actually asks,
    // when it has a cluster of observations and wants to know how many sources back the
    // cluster as a whole.
    [[nodiscard]] std::vector<std::string> independent_sources(
        const std::vector<RecordId>& ids) const;

    [[nodiscard]] int independent_source_count(const std::vector<RecordId>& ids) const {
        return static_cast<int>(independent_sources(ids).size());
    }

private:
    // Accumulates roots for one record into `out`, guarding against cycles.
    void collect_roots(const RecordId& id,
                       std::unordered_set<RecordId>& visited,
                       std::unordered_set<std::string>& out) const;

    std::unordered_map<RecordId, Provenance> records_{};
};

}  // namespace sentry::world
