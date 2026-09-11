// Turning a stream of captured events into graph updates — through the gate, always.
//
// RFC: docs/RFC-008-SENTRY-WORLD-MODEL.md §Capture
//
// A device agent posts a stream of what it observed. This reads that stream, runs every
// event through `capture_disposition`, and produces two things and only two:
//
//   - an activity summary: counts per source, never content, for what the owner did;
//   - contact observations: that the owner is in touch with someone, as hypotheses.
//
// What it deliberately does not produce is a store of everything that happened. The events
// themselves are consumed and dropped. A capture system whose output was "every keystroke,
// every message, forever" would be the most damaging file on the machine, and the summary
// is what the graph actually needs: that the owner searched forty times today, that they
// have been in contact with Rafa since March.
//
// Every suppression is counted and reported, so the owner can see the gate working rather
// than trust that it does.

#pragma once

#include <cstddef>
#include <map>
#include <string>
#include <string_view>
#include <vector>

#include "sentry/capture/activity.hpp"
#include "sentry/identity/relationship.hpp"

namespace sentry::capture {

using sentry::identity::RelationEdge;

// Hard intake bounds. Device agents send short batches; a sensor bug or hostile client
// cannot turn a billion ambient events or labels into a billion heap allocations.
inline constexpr std::size_t kMaxCaptureBatchBytes = 4 * 1024 * 1024;
inline constexpr std::size_t kMaxCaptureLineBytes = 4096;
inline constexpr std::size_t kMaxCaptureEvents = 16384;
inline constexpr std::size_t kMaxUniqueContacts = 512;
inline constexpr std::size_t kMaxCaptureIdentifierBytes = 192;

// Counts for one kind of activity. No content, ever — just how much.
struct SourceTally {
    CaptureSource source{};
    std::size_t recorded = 0;
};

// Why events did not become part of the owner's knowledge, counted so the owner can audit
// the gate rather than take it on faith.
struct SuppressionTally {
    std::size_t not_the_owner = 0;
    std::size_t actor_not_confirmed = 0;
    std::size_t device_not_enrolled = 0;
    std::size_t source_disabled = 0;
    std::size_t counterparty_content = 0;
    std::size_t contact_capacity = 0;

    [[nodiscard]] std::size_t total() const noexcept {
        return not_the_owner + actor_not_confirmed + device_not_enrolled + source_disabled +
               counterparty_content + contact_capacity;
    }
};

// What a stream amounted to.
struct CaptureSummary {
    std::size_t events_seen = 0;
    std::size_t recorded = 0;
    std::vector<SourceTally> by_source{};
    SuppressionTally suppressed{};
    // Lines that did not parse as an event.
    std::size_t unparsed = 0;
    // Set when byte/event bounds stopped intake. Exact overflow is deliberately not
    // counted by scanning the discarded tail: doing so would defeat the bound.
    bool input_truncated = false;

    // People the owner was observed to be in contact with, as hypothesis edges from the
    // owner. Never asserted — a name in a notification is evidence the owner knows someone,
    // not a statement about who they are, exactly as in the chat-export ingester.
    std::vector<RelationEdge> contact_hypotheses{};
    // How many times each counterparty appeared, so a one-off is distinguishable from a
    // sustained correspondence. Keyed by the label the device knew them under.
    std::map<std::string, std::size_t, std::less<>> contact_counts{};
};

// Parse one NDJSON line into an event. Returns nothing on a line that is not an event.
//
// The format is what a device agent posts: one flat JSON object per line, with string
// fields for the enums (`source`, `actor`, `author`) matching their `name()`. Hand-rolled
// rather than pulling in a JSON library, for the same reason the store is: Sentry has no
// third-party dependencies, and the object shape here is fixed and shallow.
[[nodiscard]] std::optional<CaptureEvent> parse_event(std::string_view line);

// Read a whole stream and summarise it.
//
// `enrollments` is looked up by device id; an event from a device not in it is suppressed
// as un-enrolled. The owner's id roots the contact hypotheses.
[[nodiscard]] CaptureSummary ingest_stream(
    const CapturePolicy& policy,
    const std::map<std::string, DeviceEnrollment, std::less<>>& enrollments,
    std::string_view owner_id,
    std::string_view text);

}  // namespace sentry::capture
