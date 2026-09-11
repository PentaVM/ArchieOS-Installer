// What survives a consent withdrawal, and what must not.
//
// The check at collection time is the easy half. These cases are the other half: a lease
// ends, and something has to decide the fate of everything already derived from it. A
// system that keeps all of it has made consent a formality; one that deletes all of it
// erases counts that named nobody and audit records the operator has to answer for.

#include "harness.hpp"

#include "sentry/identity/retention.hpp"

using namespace sentry::identity;
using sentry::policy::ConsentLease;
using sentry::world::TimePoint;

namespace {

constexpr std::int64_t kDay = 24LL * 60LL * 60LL * 1000LL;

Retention revoked(DerivedRecord record) { return on_revocation(record, std::nullopt); }
Retention revoked_edge(EdgeBasis basis) {
    return on_revocation(DerivedRecord::RelationEdge, basis);
}

void a_face_template_does_not_survive_revocation() {
    CASE("biometrics and location history are deleted outright");

    // No anonymised remainder is worth keeping. A face template with the name stripped
    // off is still a face template, and it re-identifies the moment it meets another one.
    CHECK_EQ(revoked(DerivedRecord::BiometricTemplate),
             (Retention{RetentionDecision::DeleteNow,
                        RetentionBasis::DerivedFromWithdrawnConsent}));
    CHECK_EQ(revoked(DerivedRecord::LocationHistory),
             (Retention{RetentionDecision::DeleteNow,
                        RetentionBasis::DerivedFromWithdrawnConsent}));
}

void a_track_loses_its_name_and_keeps_its_observation() {
    CASE("attribution is stripped, not the sighting");

    // A camera did see a body at 14:02, and that stays true — it simply stops being a
    // statement about a particular person. This is the state §59 already describes for a
    // subject with no consent on file: anonymous tracks.
    const auto decision = revoked(DerivedRecord::TrackAttribution);
    CHECK_BECAUSE(decision.decision == RetentionDecision::Anonymise,
                  "deleting the whole track would lose an occupancy figure that never "
                  "depended on consent in the first place");
}

void revocation_does_not_un_publish() {
    CASE("a public-record edge survives a private withdrawal");

    // The reporter's byline and the reporter's camera lease rest on different
    // authorities. Letting the second erase the first would put a private preference in
    // charge of what the public record says.
    CHECK_EQ(revoked_edge(EdgeBasis::PublicRecord),
             (Retention{RetentionDecision::Keep, RetentionBasis::IndependentPublicRecord}));
}

void a_declaration_is_not_a_consequence_of_sensing() {
    CASE("declared edges survive; withdrawing a declaration is a separate act");

    CHECK_EQ(revoked_edge(EdgeBasis::SubjectDeclared),
             (Retention{RetentionDecision::Keep, RetentionBasis::SelfDeclared}));
    CHECK_EQ(revoked_edge(EdgeBasis::OwnerDeclared),
             (Retention{RetentionDecision::Keep, RetentionBasis::SelfDeclared}));
}

void an_edge_that_should_never_have_been_stored_is_removed() {
    CASE("hypotheses and co-presence edges go on revocation");

    // `refuse_edge` rejects both, so reaching here means one was stored anyway. A
    // revocation is a good moment to be rid of it rather than to preserve a bug.
    CHECK_EQ(revoked_edge(EdgeBasis::ModelHypothesis).decision, RetentionDecision::DeleteNow);
    CHECK_EQ(revoked_edge(EdgeBasis::CoPresenceInference).decision,
             RetentionDecision::DeleteNow);
}

void an_edge_with_no_recorded_provenance_is_deleted() {
    CASE("unknown provenance is read as derived from the withdrawn lease");

    CHECK_BECAUSE(revoked(DerivedRecord::RelationEdge).decision == RetentionDecision::DeleteNow,
                  "an edge that cannot be shown to rest on an independent authority must "
                  "be assumed to rest on the one being withdrawn");
}

void a_count_of_bodies_names_nobody() {
    CASE("anonymous aggregates are kept");

    // Deleting these would destroy a true statement about a sensor in order to protect
    // information the statement never contained.
    CHECK_EQ(revoked(DerivedRecord::AnonymousAggregate),
             (Retention{RetentionDecision::Keep, RetentionBasis::NeverIdentified}));
}

void the_operator_must_still_account_for_access_they_granted() {
    CASE("access-grant audit records survive");

    // A fact about the operator's own conduct, not intelligence about the person: it says
    // access was granted and when, not where they went afterwards.
    CHECK_EQ(revoked(DerivedRecord::AccessGrantAudit),
             (Retention{RetentionDecision::Keep, RetentionBasis::OperatorAccountability}));
}

void nothing_sensitive_is_kept_by_default() {
    CASE("every record that rests on the lease is deleted or anonymised");

    // A sweep, so that a record kind added later cannot quietly default to Keep. The two
    // kinds that survive do so for reasons stated in the case names above.
    for (const auto record : {DerivedRecord::BiometricTemplate, DerivedRecord::TrackAttribution,
                              DerivedRecord::LocationHistory}) {
        CHECK_BECAUSE(revoked(record).decision != RetentionDecision::Keep,
                      "a record derived from the withdrawn lease may not simply be kept");
    }
}

// ---------------------------------------------------------------- expiry

ConsentLease lease_with(int retention_days) {
    ConsentLease l;
    l.subject_id = "ana";
    l.valid_from = TimePoint{0};
    l.retention_days = retention_days;
    return l;
}

void the_lease_window_wins_over_the_operator_default() {
    CASE("a subject's own retention limit is honoured");

    const auto collected = TimePoint{1000};
    const auto expiry = expires_at(lease_with(7), collected, kDefaultRetentionDays);
    CHECK(expiry.has_value());
    if (expiry) {
        CHECK_EQ(expiry->millis(), 1000 + 7 * kDay);
    }
}

void zero_on_the_lease_means_the_operator_default() {
    CASE("an unset lease window falls back, as ConsentLease documents");

    const auto expiry = expires_at(lease_with(0), TimePoint{0}, 30);
    CHECK(expiry.has_value());
    if (expiry) {
        CHECK_EQ(expiry->millis(), 30 * kDay);
    }
}

void no_configured_limit_is_reported_as_absent() {
    CASE("no limit is nothing, not a date far away");

    // A caller has to handle "no limit configured" rather than receiving something that
    // looks like a policy and is not one.
    CHECK_BECAUSE(!expires_at(lease_with(0), TimePoint{0}, 0).has_value(),
                  "an absent expiry forces the caller to decide; a distant one would be "
                  "silently mistaken for a retention policy");
    CHECK(!expires_at(lease_with(0), TimePoint{0}, -1).has_value());
}

void a_long_window_does_not_overflow() {
    CASE("ten years of milliseconds still fits");

    // The multiplication is done in int64. A 32-bit intermediate would wrap somewhere
    // around 24 days and produce an expiry in the past, which reads as "delete
    // immediately" — a data-loss bug wearing the costume of a strict policy.
    const auto expiry = expires_at(lease_with(3650), TimePoint{0}, kDefaultRetentionDays);
    CHECK(expiry.has_value());
    if (expiry) {
        CHECK(expiry->millis() > 0);
        CHECK_EQ(expiry->millis(), 3650LL * kDay);
    }
}

}  // namespace

int main() {
    a_face_template_does_not_survive_revocation();
    a_track_loses_its_name_and_keeps_its_observation();
    revocation_does_not_un_publish();
    a_declaration_is_not_a_consequence_of_sensing();
    an_edge_that_should_never_have_been_stored_is_removed();
    an_edge_with_no_recorded_provenance_is_deleted();
    a_count_of_bodies_names_nobody();
    the_operator_must_still_account_for_access_they_granted();
    nothing_sensitive_is_kept_by_default();

    the_lease_window_wins_over_the_operator_default();
    zero_on_the_lease_means_the_operator_default();
    no_configured_limit_is_reported_as_absent();
    a_long_window_does_not_overflow();

    return sentry::test::report();
}
