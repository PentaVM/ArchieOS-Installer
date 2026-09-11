#include "sentry/identity/retention.hpp"

namespace sentry::identity {

std::string_view name(DerivedRecord record) noexcept {
    switch (record) {
        case DerivedRecord::BiometricTemplate:  return "biometric_template";
        case DerivedRecord::TrackAttribution:   return "track_attribution";
        case DerivedRecord::LocationHistory:    return "location_history";
        case DerivedRecord::RelationEdge:       return "relation_edge";
        case DerivedRecord::AnonymousAggregate: return "anonymous_aggregate";
        case DerivedRecord::AccessGrantAudit:   return "access_grant_audit";
    }
    return "unknown";
}

std::string_view name(RetentionDecision decision) noexcept {
    switch (decision) {
        case RetentionDecision::Keep:      return "keep";
        case RetentionDecision::DeleteNow: return "delete_now";
        case RetentionDecision::Anonymise: return "anonymise";
    }
    return "unknown";
}

std::string_view describe(RetentionBasis basis) noexcept {
    switch (basis) {
        case RetentionBasis::DerivedFromWithdrawnConsent:
            return "this exists only because the subject consented, and they have taken "
                   "that back";
        case RetentionBasis::IndependentPublicRecord:
            return "this rests on the public professional record, which a private consent "
                   "withdrawal does not reach; revocation does not un-publish";
        case RetentionBasis::SelfDeclared:
            return "the subject stated this about their own life; it was never a "
                   "consequence of having been sensed";
        case RetentionBasis::NeverIdentified:
            return "this names nobody, so there is no identity in it to withdraw";
        case RetentionBasis::OperatorAccountability:
            return "the operator must be able to account for access they granted; kept as "
                   "an audit record, not as intelligence about the person";
    }
    return "unknown basis";
}

Retention on_revocation(DerivedRecord record, std::optional<EdgeBasis> edge_basis) noexcept {
    switch (record) {
        case DerivedRecord::BiometricTemplate:
        case DerivedRecord::LocationHistory:
            // Nothing about these survives the lease. They exist only because someone
            // agreed, they are the most sensitive things here, and there is no anonymised
            // remainder worth keeping: a face template with the name removed is still a
            // face template, and it re-identifies the moment it meets another one.
            return {RetentionDecision::DeleteNow,
                    RetentionBasis::DerivedFromWithdrawnConsent};

        case DerivedRecord::TrackAttribution:
            // The attribution goes; the observation stays. A camera did see a body at
            // 14:02 and that remains true — it simply stops being a statement about Ana.
            // Deleting the whole track instead would lose an occupancy figure that never
            // depended on consent, and §59 already describes this state: with no identity
            // consent, camera sightings are anonymous tracks.
            return {RetentionDecision::Anonymise,
                    RetentionBasis::DerivedFromWithdrawnConsent};

        case DerivedRecord::RelationEdge:
            if (!edge_basis) {
                // An edge whose provenance nobody recorded cannot be shown to rest on an
                // independent authority, and the safe reading of an unknown provenance is
                // that it came from the lease being withdrawn.
                return {RetentionDecision::DeleteNow,
                        RetentionBasis::DerivedFromWithdrawnConsent};
            }
            switch (*edge_basis) {
                case EdgeBasis::PublicRecord:
                    // Revocation does not un-publish. A byline is a fact about the public
                    // record and was never the subject's to withdraw; letting a consent
                    // withdrawal erase it would put private preference in charge of what
                    // the public record says.
                    return {RetentionDecision::Keep,
                            RetentionBasis::IndependentPublicRecord};
                case EdgeBasis::SubjectDeclared:
                case EdgeBasis::OwnerDeclared:
                    // Someone said this about their own life. It is not a consequence of
                    // sensing, so the sensing lease ending does not reach it. Withdrawing
                    // a *declaration* is a separate act, and one the subject makes
                    // directly rather than as a side effect.
                    return {RetentionDecision::Keep, RetentionBasis::SelfDeclared};
                case EdgeBasis::ModelHypothesis:
                case EdgeBasis::CoPresenceInference:
                    // Neither may be in the asserted graph at all — `refuse_edge` rejects
                    // both. Reaching here means one was stored anyway, and a revocation is
                    // the right moment to be rid of it.
                    return {RetentionDecision::DeleteNow,
                            RetentionBasis::DerivedFromWithdrawnConsent};
            }
            return {RetentionDecision::DeleteNow,
                    RetentionBasis::DerivedFromWithdrawnConsent};

        case DerivedRecord::AnonymousAggregate:
            // A count of bodies names nobody. There is no identity in it to withdraw, and
            // deleting it would destroy a true statement about a sensor to protect
            // information it never contained.
            return {RetentionDecision::Keep, RetentionBasis::NeverIdentified};

        case DerivedRecord::AccessGrantAudit:
            // The operator granted someone access to a space. That is a fact about the
            // operator's own conduct, and they have to be able to account for it — to a
            // landlord, an insurer, or an investigation. Kept as an audit record rather
            // than as intelligence: it says access was granted and when, not where the
            // person went afterwards.
            return {RetentionDecision::Keep, RetentionBasis::OperatorAccountability};
    }

    return {RetentionDecision::DeleteNow, RetentionBasis::DerivedFromWithdrawnConsent};
}

std::optional<TimePoint> expires_at(const sentry::policy::ConsentLease& lease,
                                    TimePoint collected_at,
                                    int operator_default_days) noexcept {
    const int days = lease.retention_days > 0 ? lease.retention_days : operator_default_days;
    if (days <= 0) {
        // No limit configured. Reported as absent rather than as a date far in the future,
        // so a caller has to decide what to do about it instead of receiving something
        // that looks like a policy and is not one.
        return std::nullopt;
    }

    constexpr std::int64_t kMillisPerDay = 24LL * 60LL * 60LL * 1000LL;
    return TimePoint{collected_at.millis() + static_cast<std::int64_t>(days) * kMillisPerDay};
}

}  // namespace sentry::identity
