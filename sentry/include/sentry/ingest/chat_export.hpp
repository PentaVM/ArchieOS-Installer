// Reading a chat export, and turning it into questions rather than answers.
//
// RFC: docs/RFC-008-SENTRY-WORLD-MODEL.md §6.3
//
// The owner exports a conversation from WhatsApp — the app's own "Export chat", which is
// first-party, risks no account, and produces a plain text file. This reads it.
//
// # What is taken, and what is deliberately left
//
// Metadata only: who appears, when they first and last wrote, how many messages. **Message
// content is parsed and discarded.** It is not stored in a profile and never reaches the
// graph.
//
// That is a design decision rather than an omission. A person graph that quietly
// accumulated the text of every conversation would become the single most damaging file on
// the machine, and it would do so as a side effect of a feature about relationships. What
// the graph needs from a chat is "the owner has been talking to this person since March
// 2024", and that is a date and a count.
//
// # Everything produced is a hypothesis
//
// A name in an export is a name in someone's address book. It is evidence that the owner
// knows a person, not a statement about who that person is, so every edge produced here
// goes to `hypothesise_edge` and every trait carries `InferredFromMessages`. The owner
// confirms what is true; nothing here asserts.
//
// # A group chat produces no edges between other people
//
// Two people being in the same group is not a relationship. It is the same inference as two
// people being in the same room, arrived at through a different sensor, and RFC-008 refuses
// it for exactly the same reason: repeated across a hundred groups it is a social graph
// assembled from membership lists rather than from anything anybody said.
//
// A one-to-one conversation is different. The owner has a direct, sustained exchange with
// that person, which is real evidence about the owner's own life — still a hypothesis, but a
// well-founded one.

#pragma once

#include <cstddef>
#include <map>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

#include "sentry/identity/profile.hpp"
#include "sentry/identity/relationship.hpp"

namespace sentry::ingest {

using sentry::identity::Profile;
using sentry::identity::RelationEdge;

// A calendar date, as far as the export can be trusted to state one.
//
// The year is always known when a line parsed at all. The day and month may not be: see
// `DateOrder`.
struct PartialDate {
    int year = 0;
    // Absent when the export's day/month order could not be determined.
    std::optional<int> month{};
    std::optional<int> day{};

    [[nodiscard]] std::string to_string() const;
    [[nodiscard]] bool operator==(const PartialDate&) const noexcept = default;
};

// Which way round an export writes its dates.
//
// `03/04/2024` is the third of April or the fourth of March, and one line cannot say which.
// A whole export usually can: if any date has a first component above twelve, the format is
// day-first, and if any has a second component above twelve it is month-first.
//
// When neither appears — a short export that happens to live entirely in the first twelve
// days of some months — the order stays `Ambiguous` and only the year is reported. Guessing
// would put a person's "known since" date three weeks wrong for no benefit, and the year is
// the part the graph actually uses.
enum class DateOrder {
    DayFirst,
    MonthFirst,
    Ambiguous,
};

[[nodiscard]] std::string_view name(DateOrder order) noexcept;

// One person as they appear in an export.
struct Participant {
    // Exactly as written: a display name, a phone number, or a saved contact name. Not
    // normalised, because the owner is the one who recognises it.
    std::string display_name{};
    std::size_t message_count = 0;
    PartialDate first_seen{};
    PartialDate last_seen{};
};

// What a whole export amounts to.
struct ChatExport {
    // Everyone who sent a message, in first-appearance order.
    std::vector<Participant> participants{};
    // Lines that carried a timestamp but no sender: "Messages are end-to-end encrypted",
    // "X was added", "You deleted this message". Counted, never attributed.
    std::size_t system_messages = 0;
    // Lines that continued the previous message rather than starting one.
    std::size_t continuation_lines = 0;
    // Lines that matched no known shape at all.
    std::size_t unparsed_lines = 0;
    DateOrder order = DateOrder::Ambiguous;

    // Three or more senders means a group, where co-membership says nothing about how the
    // others know each other.
    [[nodiscard]] bool is_group() const noexcept { return participants.size() > 2; }

    [[nodiscard]] const Participant* find(std::string_view display_name) const;
};

// Read an export. Never fails: a file that is not an export produces an empty result with
// every line counted as unparsed, which is a more useful thing to show a person than an
// error code.
[[nodiscard]] ChatExport parse_export(std::string_view text);

// What one export suggests about the graph.
//
// `owner_names` are the ways the owner appears as a sender in this export — the app writes
// the account holder under whatever name their own device knows, and only the owner can say
// which that is. Every other participant becomes a candidate.
//
// `id_for` maps a display name to the identifier the graph should use, so that "Rafa",
// "Rafa Movil" and a bare phone number can be the same person when the owner says they are.
// Returning nothing skips that participant entirely.
struct Suggestion {
    // People not already in the graph, as `ConsentedPerson` — the owner's own contacts,
    // held because the owner has a relationship with them, which is what the consent
    // machinery then governs.
    std::vector<Profile> profiles{};
    // Owner-to-participant edges, from one-to-one conversations only. Always hypotheses.
    std::vector<RelationEdge> hypotheses{};
    // Why a group produced no edges, when it did not. Present so the absence is visible
    // rather than looking like the parse failed.
    std::string note{};
};

[[nodiscard]] Suggestion suggest(const ChatExport& parsed,
                                 std::string_view owner_id,
                                 const std::vector<std::string>& owner_names,
                                 std::string_view conversation_label);

}  // namespace sentry::ingest
