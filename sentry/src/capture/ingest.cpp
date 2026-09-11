#include "sentry/capture/ingest.hpp"

#include <array>
#include <charconv>

namespace sentry::capture {

using sentry::identity::EdgeBasis;
using sentry::identity::RelationKind;

namespace {

// A minimal reader for one flat JSON object. Not a general parser: it finds a string or
// number value for a known key, and understands nothing nested. That is all the fixed event
// shape needs, and a full JSON parser is a far larger surface for bugs than this.
//
// Returns the raw value text for `"key"`, unescaping the two escapes an agent might emit in
// a label (`\"` and `\\`). Nothing here is fed to the graph as content, so the escape set
// is deliberately small.
std::optional<std::string> field(std::string_view json, std::string_view key) {
    std::string needle = "\"";
    needle += key;
    needle += "\"";
    auto at = json.find(needle);
    if (at == std::string_view::npos) {
        return std::nullopt;
    }
    at = json.find(':', at + needle.size());
    if (at == std::string_view::npos) {
        return std::nullopt;
    }
    ++at;
    while (at < json.size() && (json[at] == ' ' || json[at] == '\t')) {
        ++at;
    }
    if (at >= json.size()) {
        return std::nullopt;
    }

    if (json[at] == '"') {
        ++at;
        std::string out;
        while (at < json.size() && json[at] != '"') {
            if (json[at] == '\\' && at + 1 < json.size()) {
                ++at;
                out.push_back(json[at] == 'n' ? '\n' : json[at]);
            } else {
                out.push_back(json[at]);
            }
            ++at;
        }
        return out;
    }

    // A bare number or literal, up to the next delimiter.
    const auto end = json.find_first_of(",}", at);
    auto value = json.substr(at, (end == std::string_view::npos ? json.size() : end) - at);
    while (!value.empty() && (value.back() == ' ' || value.back() == '\t')) {
        value.remove_suffix(1);
    }
    return std::string{value};
}

template <typename E, std::size_t N>
std::optional<E> to_enum(std::string_view text, const std::array<E, N>& all) {
    for (const auto value : all) {
        if (name(value) == text) {
            return value;
        }
    }
    return std::nullopt;
}

constexpr std::array kSources{
    CaptureSource::TextEntry,    CaptureSource::Email,        CaptureSource::WebSearch,
    CaptureSource::Navigation,   CaptureSource::AppUsage,     CaptureSource::Notification,
    CaptureSource::Clipboard,    CaptureSource::FileActivity, CaptureSource::DeviceLocation};

constexpr std::array kActors{ActorAttribution::ConfirmedOwner, ActorAttribution::PresumedOwner,
                             ActorAttribution::Unknown, ActorAttribution::ConfirmedOther};

constexpr std::array kAuthors{ContentAuthor::Owner, ContentAuthor::Counterparty,
                              ContentAuthor::System};

}  // namespace

std::optional<CaptureEvent> parse_event(std::string_view line) {
    if (line.empty() || line.size() > kMaxCaptureLineBytes) {
        return std::nullopt;
    }
    const auto device = field(line, "device_id");
    const auto source = field(line, "source");
    if (!device || device->empty() || device->size() > kMaxCaptureIdentifierBytes || !source) {
        return std::nullopt;
    }
    const auto parsed_source = to_enum(*source, kSources);
    if (!parsed_source) {
        return std::nullopt;
    }

    CaptureEvent event;
    event.device_id = *device;
    event.source = *parsed_source;

    // Actor and author default to the most conservative reading when absent or unknown: an
    // event that does not say who was there is `Unknown` (recorded only under an explicit
    // policy), and one that does not say who wrote it is `System`.
    event.actor =
        field(line, "actor").and_then([](auto a) { return to_enum(a, kActors); }).value_or(
            ActorAttribution::Unknown);
    event.author =
        field(line, "author").and_then([](auto a) { return to_enum(a, kAuthors); }).value_or(
            ContentAuthor::System);

    if (const auto label = field(line, "counterparty")) {
        if (label->size() > kMaxCaptureIdentifierBytes) {
            return std::nullopt;
        }
        event.counterparty_label = *label;
    }

    if (const auto ts = field(line, "observed_at")) {
        std::int64_t millis = 0;
        const auto [ptr, ec] = std::from_chars(ts->data(), ts->data() + ts->size(), millis);
        if (ec == std::errc{}) {
            event.observed_at = TimePoint{millis};
        }
    }
    return event;
}

CaptureSummary ingest_stream(
    const CapturePolicy& policy,
    const std::map<std::string, DeviceEnrollment, std::less<>>& enrollments,
    std::string_view owner_id,
    std::string_view text) {
    CaptureSummary summary;

    if (text.size() > kMaxCaptureBatchBytes) {
        text = text.substr(0, kMaxCaptureBatchBytes);
        summary.input_truncated = true;
    }

    std::map<CaptureSource, std::size_t> recorded_by_source;

    std::size_t start = 0;
    while (start <= text.size()) {
        const auto nl = text.find('\n', start);
        auto line = text.substr(start, nl == std::string_view::npos ? std::string_view::npos
                                                                    : nl - start);
        start = (nl == std::string_view::npos) ? text.size() + 1 : nl + 1;

        while (!line.empty() && (line.back() == '\r' || line.back() == ' ')) {
            line.remove_suffix(1);
        }
        if (line.empty()) {
            continue;
        }

        if (summary.events_seen + summary.unparsed >= kMaxCaptureEvents) {
            summary.input_truncated = true;
            break;
        }

        const auto event = parse_event(line);
        if (!event) {
            summary.unparsed = summary.unparsed == SIZE_MAX ? SIZE_MAX : summary.unparsed + 1;
            continue;
        }
        summary.events_seen = summary.events_seen == SIZE_MAX ? SIZE_MAX : summary.events_seen + 1;

        std::optional<DeviceEnrollment> enrollment;
        if (const auto it = enrollments.find(event->device_id); it != enrollments.end()) {
            enrollment = it->second;
        }

        const auto verdict = capture_disposition(policy, enrollment, *event);

        // A counterparty appearing at all is a contact observation, whether or not the
        // event's content was recorded — that the owner is in touch with them is metadata,
        // and it is the one thing kept from a message that is otherwise dropped. Counted
        // before the disposition switch so it is captured for both the recorded owner-sent
        // message and the suppressed counterparty-sent one.
        if (!event->counterparty_label.empty() &&
            verdict.reason != CaptureReason::DeviceNotEnrolled &&
            verdict.reason != CaptureReason::NotTheOwner) {
            const auto existing = summary.contact_counts.find(event->counterparty_label);
            if (existing != summary.contact_counts.end()) {
                existing->second = existing->second == SIZE_MAX ? SIZE_MAX : existing->second + 1;
            } else if (summary.contact_counts.size() < kMaxUniqueContacts) {
                summary.contact_counts.emplace(event->counterparty_label, 1);
            } else {
                summary.suppressed.contact_capacity =
                    summary.suppressed.contact_capacity == SIZE_MAX
                        ? SIZE_MAX
                        : summary.suppressed.contact_capacity + 1;
            }
        }

        switch (verdict.disposition) {
            case CaptureDisposition::Record:
                ++summary.recorded;
                ++recorded_by_source[event->source];
                break;
            case CaptureDisposition::CountAnonymously:
            case CaptureDisposition::Discard:
                switch (verdict.reason) {
                    case CaptureReason::NotTheOwner:
                        ++summary.suppressed.not_the_owner;
                        break;
                    case CaptureReason::ActorNotConfirmed:
                        ++summary.suppressed.actor_not_confirmed;
                        break;
                    case CaptureReason::DeviceNotEnrolled:
                        ++summary.suppressed.device_not_enrolled;
                        break;
                    case CaptureReason::SourceDisabled:
                        ++summary.suppressed.source_disabled;
                        break;
                    case CaptureReason::CounterpartyContent:
                        ++summary.suppressed.counterparty_content;
                        break;
                    case CaptureReason::OwnerActivity:
                        break;
                }
                break;
        }
    }

    for (const auto& [source, count] : recorded_by_source) {
        summary.by_source.push_back(SourceTally{source, count});
    }

    // Turn each contact into a hypothesis edge from the owner, exactly as the chat-export
    // ingester does: a name observed in the owner's messages is evidence the owner knows
    // them, carried as a guess for the owner to confirm, never asserted.
    for (const auto& [label, count] : summary.contact_counts) {
        RelationEdge edge;
        edge.from_id = std::string{owner_id};
        edge.to_id = label;
        edge.kind = RelationKind::Friend;
        edge.basis = EdgeBasis::ModelHypothesis;
        edge.source_note = "observed in captured messages (" + std::to_string(count) +
                           "). The kind is a guess: a message says the owner is in contact "
                           "with this person, not how.";
        summary.contact_hypotheses.push_back(std::move(edge));
    }

    return summary;
}

}  // namespace sentry::capture
