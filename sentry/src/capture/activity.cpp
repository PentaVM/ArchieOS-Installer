#include "sentry/capture/activity.hpp"

#include <algorithm>

namespace sentry::capture {

using sentry::identity::OwnerBiometricMatch;

std::string_view name(CaptureSource source) noexcept {
    switch (source) {
        case CaptureSource::TextEntry:      return "text_entry";
        case CaptureSource::Email:          return "email";
        case CaptureSource::WebSearch:      return "web_search";
        case CaptureSource::Navigation:     return "navigation";
        case CaptureSource::AppUsage:       return "app_usage";
        case CaptureSource::Notification:   return "notification";
        case CaptureSource::Clipboard:      return "clipboard";
        case CaptureSource::FileActivity:   return "file_activity";
        case CaptureSource::DeviceLocation: return "device_location";
    }
    return "unknown";
}

std::string_view name(ActorAttribution attribution) noexcept {
    switch (attribution) {
        case ActorAttribution::ConfirmedOwner: return "confirmed_owner";
        case ActorAttribution::PresumedOwner:  return "presumed_owner";
        case ActorAttribution::Unknown:        return "unknown";
        case ActorAttribution::ConfirmedOther: return "confirmed_other";
    }
    return "unknown";
}

std::string_view name(ContentAuthor author) noexcept {
    switch (author) {
        case ContentAuthor::Owner:        return "owner";
        case ContentAuthor::Counterparty: return "counterparty";
        case ContentAuthor::System:       return "system";
    }
    return "unknown";
}

std::string_view name(CaptureDisposition disposition) noexcept {
    switch (disposition) {
        case CaptureDisposition::Record:          return "record";
        case CaptureDisposition::CountAnonymously: return "count_anonymously";
        case CaptureDisposition::Discard:         return "discard";
    }
    return "unknown";
}

std::string_view describe(CaptureReason reason) noexcept {
    switch (reason) {
        case CaptureReason::OwnerActivity:
            return "the owner's own activity on their own device";
        case CaptureReason::NotTheOwner:
            return "someone other than the owner was at the device; recorded that it was in "
                   "use, kept nothing about who or what";
        case CaptureReason::ActorNotConfirmed:
            return "the person at the device could not be confirmed to be the owner, and "
                   "policy does not record on presumption";
        case CaptureReason::DeviceNotEnrolled:
            return "the event came from a device the owner never enrolled; nothing links it "
                   "to them";
        case CaptureReason::SourceDisabled:
            return "the owner switched this kind of capture off";
        case CaptureReason::CounterpartyContent:
            return "the content was written by someone else and arrived at the owner's "
                   "device; only the fact of contact is kept, never the message";
    }
    return "unknown reason";
}

ActorAttribution attribution_from_biometric(OwnerBiometricMatch match) noexcept {
    switch (match) {
        case OwnerBiometricMatch::MatchedWithLiveness:
            return ActorAttribution::ConfirmedOwner;
        case OwnerBiometricMatch::NoMatch:
            // A live face that is not the owner's. This is the signal that says someone
            // else is here — the one case that legitimately lowers the attribution.
            return ActorAttribution::ConfirmedOther;
        case OwnerBiometricMatch::LivenessNotEstablished:
        case OwnerBiometricMatch::MatcherUnavailable:
            // Says nothing. A covered camera or a matcher that did not run is not evidence
            // of a guest, and treating it as one would suppress the owner's own capture
            // every time they used the phone in the dark.
            return ActorAttribution::Unknown;
    }
    return ActorAttribution::Unknown;
}

bool CapturePolicy::source_enabled(CaptureSource source) const noexcept {
    return std::ranges::find(disabled_sources, source) == disabled_sources.end();
}

Sensitivity effective_sensitivity(const CaptureEvent& event) noexcept {
    // A floor from the source, before any declared value. Some sources are sensitive by
    // their nature no matter what an agent claims: a location stream is where the owner is,
    // an email is correspondence, a clipboard holds whatever was last copied — which is a
    // password often enough that it cannot be assumed harmless.
    Sensitivity from_source = Sensitivity::Personal;
    switch (event.source) {
        case CaptureSource::Email:
        case CaptureSource::Notification:
        case CaptureSource::Clipboard:
        case CaptureSource::DeviceLocation:
            from_source = Sensitivity::Sensitive;
            break;
        case CaptureSource::WebSearch:
        case CaptureSource::TextEntry:
        case CaptureSource::Navigation:
        case CaptureSource::FileActivity:
            from_source = Sensitivity::Personal;
            break;
        case CaptureSource::AppUsage:
            // Which app is open is the least exposing thing here — closer to public than
            // personal, but a personal floor is the safe side.
            from_source = Sensitivity::Personal;
            break;
    }

    if (!event.declared_sensitivity) {
        return from_source;
    }
    // The stricter of the two wins, exactly as in the profile layer: an agent may say a
    // field is more sensitive than its source suggests, never less.
    return sentry::identity::at_least_as_strict(*event.declared_sensitivity, from_source)
               ? *event.declared_sensitivity
               : from_source;
}

CaptureVerdict capture_disposition(const CapturePolicy& policy,
                                   const std::optional<DeviceEnrollment>& enrollment,
                                   const CaptureEvent& event) noexcept {
    // 1. The device has to be one the owner linked. Checked before content, before actor,
    //    before anything: a device that is not enrolled has no standing to feed the graph,
    //    and "linkearse solo a mí" is precisely this gate. A withdrawn enrollment counts as
    //    not enrolled — the id is kept so this refuses rather than silently re-trusting.
    if (!enrollment || !enrollment->active || enrollment->device_id != event.device_id ||
        event.observed_at < enrollment->enrolled_at) {
        return {CaptureDisposition::Discard, CaptureReason::DeviceNotEnrolled};
    }

    // 2. A source the owner switched off produces nothing, whoever is at the device.
    if (!policy.source_enabled(event.source)) {
        return {CaptureDisposition::Discard, CaptureReason::SourceDisabled};
    }

    // 3. The actor. This is the "mum is using it" gate, and its whole point is that
    //    detecting a guest suppresses capture rather than profiling them. Confirmed-other
    //    is never recorded and never attributed to a person; the most that is kept is that
    //    the device was in use, with no content and no identity — the AnonymousTrack rule.
    switch (event.actor) {
        case ActorAttribution::ConfirmedOther:
            return {CaptureDisposition::CountAnonymously, CaptureReason::NotTheOwner};
        case ActorAttribution::Unknown:
            if (!policy.record_when_unknown) {
                // An unknown actor is the case most likely to be someone else. Kept as an
                // anonymous count rather than dropped, so "the device was used at all" is
                // still known without recording who or what.
                return {CaptureDisposition::CountAnonymously, CaptureReason::ActorNotConfirmed};
            }
            break;
        case ActorAttribution::PresumedOwner:
            if (!policy.record_when_presumed) {
                return {CaptureDisposition::CountAnonymously, CaptureReason::ActorNotConfirmed};
            }
            break;
        case ActorAttribution::ConfirmedOwner:
            break;
    }

    // 4. Whose words these are. The owner is at the device and it is their activity — but
    //    if the content was written by a counterparty (a message they received), the text
    //    is that person's. Keep that the owner is in contact with them; never the message.
    //    Folding "everything X said to me" into a searchable store is the §40/§113 problem
    //    arriving through the owner's own inbox.
    if (event.author == ContentAuthor::Counterparty) {
        return {CaptureDisposition::CountAnonymously, CaptureReason::CounterpartyContent};
    }

    return {CaptureDisposition::Record, CaptureReason::OwnerActivity};
}

}  // namespace sentry::capture
