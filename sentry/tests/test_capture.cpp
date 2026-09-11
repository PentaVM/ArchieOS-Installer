// Capturing the owner's life without capturing everyone else's.
//
// The parsing is easy; the gate is the point. Every case here is one of the two ways this
// feature turns into surveillance — recording a guest's activity, or storing the words of
// people who messaged the owner — and the rule that stops each.

#include "harness.hpp"

#include "sentry/capture/activity.hpp"

using namespace sentry::capture;
using sentry::identity::OwnerBiometricMatch;
using sentry::identity::Sensitivity;
using sentry::world::TimePoint;

namespace {

TimePoint at(std::int64_t ms) { return TimePoint{ms}; }

DeviceEnrollment enrolled(std::string id) {
    DeviceEnrollment d;
    d.device_id = std::move(id);
    d.label = "a device";
    d.enrolled_at = at(100);
    d.active = true;
    return d;
}

CapturePolicy policy() {
    CapturePolicy p;
    p.owner_id = "nelson";
    p.record_when_presumed = true;
    p.record_when_unknown = false;
    return p;
}

CaptureEvent event(CaptureSource source, ActorAttribution actor, ContentAuthor author) {
    CaptureEvent e;
    e.device_id = "phone";
    e.source = source;
    e.actor = actor;
    e.author = author;
    e.observed_at = at(1'000);
    return e;
}

CaptureVerdict decide(const CapturePolicy& p, const CaptureEvent& e) {
    return capture_disposition(p, enrolled("phone"), e);
}

// ---------------------------------------------------------------- the owner

void the_owners_own_activity_is_recorded() {
    CASE("an email the owner writes on their own phone");

    const auto v = decide(policy(), event(CaptureSource::Email, ActorAttribution::ConfirmedOwner,
                                          ContentAuthor::Owner));
    CHECK_EQ(v, (CaptureVerdict{CaptureDisposition::Record, CaptureReason::OwnerActivity}));
}

void a_search_the_owner_makes_is_recorded() {
    CASE("cuando busco cosas en Google");

    const auto v = decide(policy(), event(CaptureSource::WebSearch,
                                          ActorAttribution::ConfirmedOwner, ContentAuthor::Owner));
    CHECK_EQ(v.disposition, CaptureDisposition::Record);
}

// ---------------------------------------------------------------- someone else

void when_mum_uses_the_phone_nothing_is_recorded_about_her() {
    CASE("sepa cuando alguien más lo usa — and stops");

    // The load-bearing case. A confirmed guest is never recorded and never profiled. The
    // most that is kept is that the device was in use, with no content and no identity.
    for (const auto source : {CaptureSource::Email, CaptureSource::WebSearch,
                              CaptureSource::TextEntry, CaptureSource::Notification,
                              CaptureSource::DeviceLocation}) {
        const auto v = decide(policy(), event(source, ActorAttribution::ConfirmedOther,
                                              ContentAuthor::Owner));
        CHECK_BECAUSE(v.disposition == CaptureDisposition::CountAnonymously,
                      "detecting a guest suppresses capture; it does not open a dossier "
                      "on them");
        CHECK_EQ(v.reason, CaptureReason::NotTheOwner);
    }
}

void a_guest_is_never_recorded_even_under_the_most_permissive_policy() {
    CASE("no policy switch turns guest capture back on");

    CapturePolicy loose = policy();
    loose.record_when_presumed = true;
    loose.record_when_unknown = true;

    const auto v = decide(loose, event(CaptureSource::Email, ActorAttribution::ConfirmedOther,
                                       ContentAuthor::Owner));
    CHECK_BECAUSE(v.disposition != CaptureDisposition::Record,
                  "recording someone else's activity is not a setting the owner can enable, "
                  "because the someone else did not consent to any of it");
}

// ---------------------------------------------------------------- uncertainty

void an_unknown_actor_is_not_recorded_by_default() {
    CASE("a just-woken device is the case most likely to be someone else");

    const auto v = decide(policy(), event(CaptureSource::TextEntry, ActorAttribution::Unknown,
                                          ContentAuthor::Owner));
    CHECK_EQ(v.disposition, CaptureDisposition::CountAnonymously);
    CHECK_EQ(v.reason, CaptureReason::ActorNotConfirmed);
}

void the_owner_may_choose_to_record_on_presumption() {
    CASE("an unlocked personal device is almost always the owner");

    // The life-logger default: without this, a phone that only biometrically confirms on
    // unlock would capture nothing for the rest of the session.
    const auto v = decide(policy(), event(CaptureSource::TextEntry,
                                          ActorAttribution::PresumedOwner, ContentAuthor::Owner));
    CHECK_EQ(v.disposition, CaptureDisposition::Record);

    CapturePolicy strict = policy();
    strict.record_when_presumed = false;
    CHECK_EQ(decide(strict, event(CaptureSource::TextEntry, ActorAttribution::PresumedOwner,
                                  ContentAuthor::Owner))
                 .disposition,
             CaptureDisposition::CountAnonymously);
}

void a_covered_camera_is_not_a_guest() {
    CASE("liveness-not-established says nothing, it does not accuse");

    // The bug this guards: treating a matcher that could not run as "someone else is here"
    // would suppress the owner's own capture every time they used the phone in the dark.
    CHECK_EQ(attribution_from_biometric(OwnerBiometricMatch::MatchedWithLiveness),
             ActorAttribution::ConfirmedOwner);
    CHECK_EQ(attribution_from_biometric(OwnerBiometricMatch::NoMatch),
             ActorAttribution::ConfirmedOther);
    CHECK_BECAUSE(attribution_from_biometric(OwnerBiometricMatch::LivenessNotEstablished) ==
                      ActorAttribution::Unknown,
                  "a covered camera is not evidence of a guest");
    CHECK_EQ(attribution_from_biometric(OwnerBiometricMatch::MatcherUnavailable),
             ActorAttribution::Unknown);
}

// ---------------------------------------------------------------- received content

void a_message_someone_sends_the_owner_is_metadata_only() {
    CASE("received content belongs to whoever sent it");

    // The owner is at their own device, reading a message. But the words are the sender's,
    // and folding "everything X said to me" into a searchable store attributes X's private
    // messages — §40 arriving through the owner's inbox.
    auto e = event(CaptureSource::Notification, ActorAttribution::ConfirmedOwner,
                   ContentAuthor::Counterparty);
    e.counterparty_label = "Rafa";
    const auto v = decide(policy(), e);

    CHECK_BECAUSE(v.disposition == CaptureDisposition::CountAnonymously,
                  "that the owner is in contact with Rafa is kept; the text of what Rafa "
                  "wrote is not");
    CHECK_EQ(v.reason, CaptureReason::CounterpartyContent);
}

void the_owners_own_message_is_recorded() {
    CASE("what the owner writes is theirs to keep");

    auto e = event(CaptureSource::Notification, ActorAttribution::ConfirmedOwner,
                   ContentAuthor::Owner);
    CHECK_EQ(decide(policy(), e).disposition, CaptureDisposition::Record);
}

// ---------------------------------------------------------------- the device

void an_un_enrolled_device_feeds_nothing() {
    CASE("linkearse solo a mí");

    // A phone the owner never linked has no standing, whatever it claims. Checked before
    // content and before the actor: the graph cannot be fed by a device that is not theirs.
    const auto v = capture_disposition(policy(), std::nullopt,
                                       event(CaptureSource::Email,
                                             ActorAttribution::ConfirmedOwner,
                                             ContentAuthor::Owner));
    CHECK_EQ(v, (CaptureVerdict{CaptureDisposition::Discard, CaptureReason::DeviceNotEnrolled}));
}

void a_withdrawn_device_is_refused_not_silently_re_trusted() {
    CASE("un-enrolling a device stops it");

    auto device = enrolled("phone");
    device.active = false;
    const auto v = capture_disposition(policy(), device,
                                       event(CaptureSource::Email,
                                             ActorAttribution::ConfirmedOwner,
                                             ContentAuthor::Owner));
    CHECK_EQ(v.reason, CaptureReason::DeviceNotEnrolled);
}

void an_event_from_before_enrollment_is_not_trusted() {
    CASE("a device's history before the owner linked it is not theirs to import");

    auto e = event(CaptureSource::Email, ActorAttribution::ConfirmedOwner, ContentAuthor::Owner);
    e.observed_at = at(50);  // before enrolled_at = 100
    CHECK_EQ(decide(policy(), e).reason, CaptureReason::DeviceNotEnrolled);
}

void a_device_id_mismatch_is_refused() {
    CASE("an event claiming a different device than its enrollment");

    auto e = event(CaptureSource::Email, ActorAttribution::ConfirmedOwner, ContentAuthor::Owner);
    e.device_id = "someone-elses-phone";
    const auto v = capture_disposition(policy(), enrolled("phone"), e);
    CHECK_EQ(v.reason, CaptureReason::DeviceNotEnrolled);
}

// ---------------------------------------------------------------- disabled sources

void a_disabled_source_produces_nothing() {
    CASE("the owner can switch a whole kind of capture off");

    CapturePolicy p = policy();
    p.disabled_sources = {CaptureSource::DeviceLocation};
    const auto v = decide(p, event(CaptureSource::DeviceLocation,
                                   ActorAttribution::ConfirmedOwner, ContentAuthor::Owner));
    CHECK_EQ(v, (CaptureVerdict{CaptureDisposition::Discard, CaptureReason::SourceDisabled}));

    // Other sources still flow.
    CHECK_EQ(decide(p, event(CaptureSource::WebSearch, ActorAttribution::ConfirmedOwner,
                             ContentAuthor::Owner))
                 .disposition,
             CaptureDisposition::Record);
}

// ---------------------------------------------------------------- sensitivity

void a_source_carries_a_sensitivity_floor() {
    CASE("an email and a location are sensitive by their nature");

    auto mail = event(CaptureSource::Email, ActorAttribution::ConfirmedOwner, ContentAuthor::Owner);
    CHECK_EQ(effective_sensitivity(mail), Sensitivity::Sensitive);

    auto app = event(CaptureSource::AppUsage, ActorAttribution::ConfirmedOwner, ContentAuthor::Owner);
    CHECK_EQ(effective_sensitivity(app), Sensitivity::Personal);

    // An agent may raise it, never lower it.
    auto app_sensitive = app;
    app_sensitive.declared_sensitivity = Sensitivity::Sensitive;
    CHECK_EQ(effective_sensitivity(app_sensitive), Sensitivity::Sensitive);

    auto mail_downplayed = mail;
    mail_downplayed.declared_sensitivity = Sensitivity::Personal;
    CHECK_BECAUSE(effective_sensitivity(mail_downplayed) == Sensitivity::Sensitive,
                  "an agent claiming an email is only personal does not make it so");
}

}  // namespace

int main() {
    the_owners_own_activity_is_recorded();
    a_search_the_owner_makes_is_recorded();

    when_mum_uses_the_phone_nothing_is_recorded_about_her();
    a_guest_is_never_recorded_even_under_the_most_permissive_policy();

    an_unknown_actor_is_not_recorded_by_default();
    the_owner_may_choose_to_record_on_presumption();
    a_covered_camera_is_not_a_guest();

    a_message_someone_sends_the_owner_is_metadata_only();
    the_owners_own_message_is_recorded();

    an_un_enrolled_device_feeds_nothing();
    a_withdrawn_device_is_refused_not_silently_re_trusted();
    an_event_from_before_enrollment_is_not_trusted();
    a_device_id_mismatch_is_refused();

    a_disabled_source_produces_nothing();
    a_source_carries_a_sensitivity_floor();

    return sentry::test::report();
}
