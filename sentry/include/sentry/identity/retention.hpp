// Retention and revocation — what happens to everything already derived when someone
// takes their consent back.
//
// RFC: docs/RFC-008-SENTRY-WORLD-MODEL.md §Consent
//
// Consent is easy to check at the moment of collection and easy to forget afterwards. The
// hard part is the second question: a subject withdraws their lease, and the system is
// already holding a face template, six months of location history, an attribution linking
// them to forty camera tracks, and a relationship edge. Which of those survive?
//
// A system that answers "all of them, the check already passed" has made consent a
// formality. A system that answers "none of them, delete everything" is also wrong, and
// wrong in a way that is harder to see: it deletes occupancy counts that never named
// anyone, it erases the record of an access grant the operator still needs to account
// for, and it drops a professional tie that came from a byline the subject's lease had
// nothing to do with.
//
// So the answer is per record, and it turns on one question: **was this derived from the
// consent being withdrawn, or from an independent authority?**
//
// # Revocation does not un-publish
//
// A reporter's byline is a fact about the public record. If that reporter later revokes a
// camera lease they had granted for an office visit, the byline does not become unknown —
// the two rest on different authorities, and collapsing them would let a consent
// withdrawal rewrite the public record. This is the same independence that
// `policy/consent.hpp` builds the permitted-operation intersection out of, applied
// backwards in time.
//
// # Deletion leaves a tombstone
//
// The record goes; the fact that it went does not. An operator who cannot show what was
// deleted and why cannot demonstrate that a revocation was honoured, and "trust us, it is
// gone" is not an answer to anybody. A tombstone carries no personal data — an opaque id,
// a time, and a reason — which is what makes keeping it compatible with deleting the
// thing it describes.

#pragma once

#include <cstdint>
#include <optional>
#include <string>
#include <string_view>

#include "sentry/identity/relationship.hpp"
#include "sentry/identity/subject.hpp"

namespace sentry::identity {

using sentry::world::TimePoint;

// A thing Sentry holds that was derived from, or about, a person.
enum class DerivedRecord {
    // A face or gait template computed from imagery.
    BiometricTemplate,
    // "Track 17 is Ana." The link between a sensor observation and a person.
    TrackAttribution,
    // Where the subject has been, from a location provider they authorized.
    LocationHistory,
    // An edge in the relationship graph.
    RelationEdge,
    // A count of bodies. Derived from imagery, attributable to nobody.
    AnonymousAggregate,
    // The operator's record that this person was granted access to a space or device.
    AccessGrantAudit,
};

[[nodiscard]] std::string_view name(DerivedRecord record) noexcept;

// What to do with a record when the consent behind it ends.
enum class RetentionDecision {
    // Keep it. It does not rest on the withdrawn lease.
    Keep,
    // Delete the record itself, leaving a tombstone.
    DeleteNow,
    // Strip the identity and keep what remains. A track that was "Ana at 14:02" becomes
    // "someone at 14:02" — which is what it was before consent made it more, and is still
    // a true statement about a sensor reading.
    Anonymise,
};

[[nodiscard]] std::string_view name(RetentionDecision decision) noexcept;

// Why a retention decision was reached. Carried so an operator can answer "why is this
// still here" without re-deriving the reasoning.
enum class RetentionBasis {
    // The record was produced under the lease that has been withdrawn.
    DerivedFromWithdrawnConsent,
    // The record rests on the public professional record, which a private consent
    // withdrawal does not reach.
    IndependentPublicRecord,
    // The subject declared it themselves; it was never a consequence of sensing.
    SelfDeclared,
    // The record names nobody, so there is no identity in it to withdraw.
    NeverIdentified,
    // The operator must be able to account for access they granted. Kept as an audit
    // record, not as intelligence about the person.
    OperatorAccountability,
};

[[nodiscard]] std::string_view describe(RetentionBasis basis) noexcept;

// A decision, with its reason.
struct Retention {
    RetentionDecision decision = RetentionDecision::DeleteNow;
    RetentionBasis basis = RetentionBasis::DerivedFromWithdrawnConsent;

    [[nodiscard]] bool operator==(const Retention&) const noexcept = default;
};

// What happens to one derived record when a subject withdraws consent.
//
// `edge_basis` matters only for `RelationEdge` and is ignored otherwise: an edge the
// subject declared about their own life is not a consequence of having been sensed, and
// an edge from the public record was never theirs to withdraw.
[[nodiscard]] Retention on_revocation(DerivedRecord record,
                                      std::optional<EdgeBasis> edge_basis) noexcept;

// When a record collected under a lease must be gone.
//
// `retention_days` of zero on the lease means the operator's default applies, which is
// what `ConsentLease::retention_days` documents. A default of zero or less means the
// operator has set no limit, and this returns nothing — an absent expiry, not an infinite
// one, so a caller has to handle "no limit configured" rather than receiving a date so far
// away it looks like a policy.
[[nodiscard]] std::optional<TimePoint> expires_at(const sentry::policy::ConsentLease& lease,
                                                  TimePoint collected_at,
                                                  int operator_default_days) noexcept;

// Proof that something was deleted, holding nothing about the person it concerned.
//
// Deliberately not a `SubjectRecord`: no name, no label, no attributes. An opaque record
// id, when it went, and why. A tombstone that carried personal data would defeat the
// deletion it is meant to evidence.
struct Tombstone {
    std::string record_id{};
    DerivedRecord kind{};
    TimePoint deleted_at{};
    RetentionBasis reason{};
};

// The number of days in the operator's default window, expressed once so that a caller
// cannot quietly pick a longer one.
inline constexpr int kDefaultRetentionDays = 30;

}  // namespace sentry::identity
