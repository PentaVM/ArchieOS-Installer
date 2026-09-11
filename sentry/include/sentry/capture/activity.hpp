// Ambient activity capture — the owner's own life across their own devices, and the line
// that keeps it from becoming a household surveillance system.
//
// RFC: docs/RFC-008-SENTRY-WORLD-MODEL.md §Capture
//
// The owner wants Archie to see what they do: the email they are writing, the thing they
// searched for, the app in front of them, the message that just arrived — live, on every
// device they own. That is legitimate. It is their activity on their hardware, and a
// personal system that cannot see any of it is not much of a brain.
//
// Two lines make the difference between that and spyware, and both are enforced here rather
// than left to whoever writes the device agent.
//
// # The device is not always the owner
//
// A phone gets handed to someone. "Sepa cuando alguien más lo usa, como mi mamá" is the
// requirement — and the safe reading of it is the opposite of the dangerous one. Detecting
// that the person at the device is *not* the owner must **suppress** capture, not open a
// second dossier. Mum using the phone is not a subject to be profiled; she is a reason to
// stop recording. So an event confirmed to belong to someone other than the owner is never
// folded into the owner's knowledge and never attributed to a person — at most it is counted
// as "the device was in use by someone else", which is exactly the `AnonymousTrack` rule
// from the camera layer, applied to a keyboard.
//
// # Received content belongs to whoever sent it
//
// When a message arrives on the owner's phone, the notification carries someone else's
// words. Folding "here is everything X ever said to me, searchable forever" into the graph
// attributes X's private messages — the §40/§113 problem arriving through a different door.
// So content authored by a counterparty is metadata only: that the owner is in contact with
// X, and when, never the text of what X wrote. The owner's own words are the owner's to
// keep.

#pragma once

#include <cstdint>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

#include "sentry/identity/profile.hpp"
#include "sentry/identity/subject.hpp"
#include "sentry/world/time.hpp"

namespace sentry::capture {

using sentry::identity::Sensitivity;
using sentry::world::TimePoint;

// What kind of activity an event describes.
//
// Grouped by the OS mechanism that can actually deliver it live and persistently, because
// the honest capture path is an on-device agent using supported streams — a notification
// listener, an accessibility service, an IMAP IDLE session — not a headless web login that
// gets an account banned.
enum class CaptureSource {
    // Text the owner is entering: a field, a document, a keystroke stream. Accessibility
    // service on a phone; input hooks or an editor plugin on a desktop.
    TextEntry,
    // An email being read or composed. IMAP/JMAP session, or the Gmail API.
    Email,
    // A search query. Browser extension, or the accessibility tree.
    WebSearch,
    // A page or screen the owner navigated to. URL, app deep-link, window title.
    Navigation,
    // Which application is in the foreground, and for how long.
    AppUsage,
    // An OS notification. On a phone this is where WhatsApp, Instagram, SMS and mail
    // message previews arrive, live, through the supported NotificationListenerService.
    Notification,
    // Something copied to the clipboard.
    Clipboard,
    // A file opened, saved or shared.
    FileActivity,
    // Where the device says it is.
    DeviceLocation,
};

[[nodiscard]] std::string_view name(CaptureSource source) noexcept;

// Who was believed to be at the device when the event happened.
//
// Ordered from strongest positive identification of the owner to strongest negation, but
// never collapsed to a bool: "presumed the owner because nothing said otherwise" and
// "confirmed the owner by a live biometric match" authorize different things, and a system
// that treated them the same would record a guest's activity the moment the owner walked
// away without locking the screen.
enum class ActorAttribution {
    // A positive signal placed the owner at the device: a live biometric match, the
    // owner's own unlock, an explicit "this is me".
    ConfirmedOwner,
    // Nothing contradicts the owner being there, but nothing confirmed it either. The
    // ordinary steady state of an unlocked personal device.
    PresumedOwner,
    // Genuinely unknown: the device woke, the session is fresh, no signal has arrived.
    Unknown,
    // A signal placed someone *other* than the owner at the device: a face that did not
    // match, a different unlock credential, an explicit "not me". This is the "mum is
    // using it" state, and it is a reason to stop, not a person to profile.
    ConfirmedOther,
};

[[nodiscard]] std::string_view name(ActorAttribution attribution) noexcept;

// Map a live owner-biometric result to an attribution.
//
// Bridges Codex's owner-biometric gate (`OwnerBiometricMatch`) into this layer as the
// strongest signal available. A matcher that is unavailable or could not establish liveness
// says nothing — it does not get to lower the attribution to `ConfirmedOther`, because "the
// camera was covered" is not "someone else is here".
[[nodiscard]] ActorAttribution attribution_from_biometric(
    sentry::identity::OwnerBiometricMatch match) noexcept;

// Who authored the content of an event.
//
// Distinct from who was at the device: the owner reading a message they received is the
// owner at the device, but the words are the sender's. This is the axis §40 turns on.
enum class ContentAuthor {
    // The owner produced it: what they typed, searched, composed.
    Owner,
    // Someone else produced it and it arrived at the owner's device: a received message,
    // an incoming email's body.
    Counterparty,
    // The system produced it: an app name, a window title, a URL. Nobody's private speech.
    System,
};

[[nodiscard]] std::string_view name(ContentAuthor author) noexcept;

// A device the owner has enrolled.
//
// Capture only flows from enrolled devices. "Linkearse solo a mí" is this: an event from a
// device the owner never enrolled is refused, so the graph cannot be fed by a phone that is
// not theirs, whatever it claims about itself.
struct DeviceEnrollment {
    std::string device_id{};
    // A human label: "Nelson's phone", "work laptop".
    std::string label{};
    // When the owner enrolled it. An event stamped before this did not happen on an
    // enrolled device and is not trusted.
    TimePoint enrolled_at{};
    // Withdrawn devices keep their id so a later event from them is refused rather than
    // silently re-trusted.
    bool active = true;
};

// One thing observed on a device.
struct CaptureEvent {
    std::string device_id{};
    CaptureSource source{};
    ActorAttribution actor = ActorAttribution::Unknown;
    ContentAuthor author = ContentAuthor::System;
    // If the content is a message, who the other party is, as the device knows them (a
    // contact name, a handle, a number). Empty otherwise. Never resolved to a graph
    // identity here — that is the consent-gated step, done later and elsewhere.
    std::string counterparty_label{};
    // How exposed the content is. Supplied by the agent when it knows; otherwise
    // `capture_disposition` classifies from the source.
    std::optional<Sensitivity> declared_sensitivity{};
    TimePoint observed_at{};
};

// The owner's standing choices about capture.
struct CapturePolicy {
    std::string owner_id{};
    // Whether to record when the actor is only presumed, not confirmed, to be the owner.
    //
    // A life-logger wants this true: an unlocked personal device is almost always the
    // owner, and refusing everything not biometrically confirmed would capture nothing.
    // The privacy-first default is false. Either way, `ConfirmedOther` is never recorded.
    bool record_when_presumed = true;
    // Whether to record at all when the actor is entirely unknown (a just-woken device, a
    // brand-new session). Stricter than `record_when_presumed`, and false by default: an
    // unknown actor is the case most likely to be someone else.
    bool record_when_unknown = false;
    // Sources the owner has switched off entirely, whatever the attribution.
    std::vector<CaptureSource> disabled_sources{};

    [[nodiscard]] bool source_enabled(CaptureSource source) const noexcept;
};

// What to do with an event.
enum class CaptureDisposition {
    // Fold it into the owner's knowledge, attributed to the owner.
    Record,
    // The device was in use, but not by the owner (or not confirmably so, under a strict
    // policy). Keep a count that the device was active; keep no content and attach no
    // identity. The `AnonymousTrack` rule, applied to a device.
    CountAnonymously,
    // Drop it. An un-enrolled device, a disabled source, content that may not be held.
    Discard,
};

[[nodiscard]] std::string_view name(CaptureDisposition disposition) noexcept;

// Why an event got the disposition it did. Each maps to a different thing the owner might
// change, which is why it is an enum and not folded into the disposition.
enum class CaptureReason {
    OwnerActivity,
    // Someone other than the owner was at the device.
    NotTheOwner,
    // The actor could not be confirmed and policy does not record on presumption.
    ActorNotConfirmed,
    // The device is not one the owner enrolled.
    DeviceNotEnrolled,
    // The owner switched this source off.
    SourceDisabled,
    // The content was authored by a counterparty; only the fact of contact is kept.
    CounterpartyContent,
};

[[nodiscard]] std::string_view describe(CaptureReason reason) noexcept;

// A disposition with its reason.
struct CaptureVerdict {
    CaptureDisposition disposition{};
    CaptureReason reason{};

    [[nodiscard]] bool operator==(const CaptureVerdict&) const noexcept = default;
};

// Decide what happens to one captured event. This is the whole gate.
//
// `enrollment` is the device's enrollment if it has one. Checked first: an event from an
// un-enrolled device is discarded before anything about its content is considered, because
// a device the owner never linked has no standing to feed anything.
[[nodiscard]] CaptureVerdict capture_disposition(
    const CapturePolicy& policy,
    const std::optional<DeviceEnrollment>& enrollment,
    const CaptureEvent& event) noexcept;

// The sensitivity that actually applies to an event, given its source and any declared
// value. A password field is sensitive whatever the agent says; the stricter wins, exactly
// as in the profile layer.
[[nodiscard]] Sensitivity effective_sensitivity(const CaptureEvent& event) noexcept;

}  // namespace sentry::capture
