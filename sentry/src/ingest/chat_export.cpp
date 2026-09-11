#include "sentry/ingest/chat_export.hpp"

#include <algorithm>
#include <cctype>

namespace sentry::ingest {

using namespace sentry::identity;

std::string_view name(DateOrder order) noexcept {
    switch (order) {
        case DateOrder::DayFirst:   return "day_first";
        case DateOrder::MonthFirst: return "month_first";
        case DateOrder::Ambiguous:  return "ambiguous";
    }
    return "unknown";
}

std::string PartialDate::to_string() const {
    if (year == 0) {
        return "unknown";
    }
    std::string out = std::to_string(year);
    if (month && day) {
        const auto pad = [](int v) {
            return (v < 10 ? std::string{"0"} : std::string{}) + std::to_string(v);
        };
        out += "-" + pad(*month) + "-" + pad(*day);
    }
    return out;
}

const Participant* ChatExport::find(std::string_view display_name) const {
    const auto it = std::ranges::find_if(participants, [display_name](const Participant& p) {
        return p.display_name == display_name;
    });
    return it == participants.end() ? nullptr : &*it;
}

namespace {

// A timestamp as it appeared, before the day/month order is known.
struct RawDate {
    int first = 0;
    int second = 0;
    int year = 0;
    bool valid = false;
};

struct Line {
    RawDate date{};
    std::string sender{};  // empty for a system message
    bool has_timestamp = false;
};

// WhatsApp writes several invisible characters into its exports — a byte-order mark, and
// left-to-right marks around timestamps on iOS. They are not whitespace to `isspace` and
// they will silently defeat every prefix test below.
bool is_invisible(unsigned char c, std::string_view rest) {
    if (c == 0xEF && rest.size() >= 2 && static_cast<unsigned char>(rest[0]) == 0xBB &&
        static_cast<unsigned char>(rest[1]) == 0xBF) {
        return true;  // BOM
    }
    // U+200E / U+200F, as UTF-8: E2 80 8E / E2 80 8F.
    return c == 0xE2 && rest.size() >= 2 && static_cast<unsigned char>(rest[0]) == 0x80 &&
           (static_cast<unsigned char>(rest[1]) == 0x8E ||
            static_cast<unsigned char>(rest[1]) == 0x8F);
}

std::string_view strip_invisible(std::string_view line) {
    while (!line.empty()) {
        const auto c = static_cast<unsigned char>(line.front());
        const auto rest = line.substr(1);
        if (is_invisible(c, rest)) {
            line.remove_prefix(3);
            continue;
        }
        if (std::isspace(c) != 0) {
            line.remove_prefix(1);
            continue;
        }
        break;
    }
    while (!line.empty() && (line.back() == '\r' || line.back() == '\n')) {
        line.remove_suffix(1);
    }
    return line;
}

// Read digits, returning how many were consumed.
std::size_t read_int(std::string_view text, std::size_t at, int& out) {
    std::size_t n = 0;
    out = 0;
    while (at + n < text.size() && std::isdigit(static_cast<unsigned char>(text[at + n])) != 0) {
        out = out * 10 + (text[at + n] - '0');
        ++n;
    }
    return n;
}

// `d/m/yy` or `d/m/yyyy`, with `/`, `-` or `.` between the parts. Returns the offset just
// past the date, or zero if the text does not start with one.
std::size_t read_date(std::string_view text, RawDate& out) {
    std::size_t at = 0;
    int a = 0;
    const auto na = read_int(text, at, a);
    if (na == 0 || na > 4) {
        return 0;
    }
    at += na;
    if (at >= text.size() || (text[at] != '/' && text[at] != '-' && text[at] != '.')) {
        return 0;
    }
    const char sep = text[at];
    ++at;
    int b = 0;
    const auto nb = read_int(text, at, b);
    if (nb == 0) {
        return 0;
    }
    at += nb;
    if (at >= text.size() || text[at] != sep) {
        return 0;
    }
    ++at;
    int c = 0;
    const auto nc = read_int(text, at, c);
    if (nc == 0) {
        return 0;
    }
    at += nc;

    // ISO-ish `yyyy-mm-dd`: a four-digit first component fixes the order outright.
    if (na == 4) {
        out = RawDate{b, c, a, true};
        return at;
    }
    // Two-digit years. WhatsApp has not existed since 1970, so a two-digit year is this
    // century — and treating "24" as 1924 would put every date a hundred years wrong.
    out = RawDate{a, b, nc <= 2 ? 2000 + c : c, true};
    return at;
}

// One line, split into a timestamp and a sender if it has them.
//
// Two shapes, and the separator is what tells them apart:
//   [12/03/2024, 14:32:10] Rafa: hello        (iOS)
//   12/03/2024, 14:32 - Rafa: hello           (Android)
Line read_line(std::string_view line) {
    Line out;
    line = strip_invisible(line);
    if (line.empty()) {
        return out;
    }

    const bool bracketed = line.front() == '[';
    std::string_view rest = bracketed ? line.substr(1) : line;

    RawDate date;
    const auto after_date = read_date(rest, date);
    if (after_date == 0) {
        return out;
    }
    rest = rest.substr(after_date);

    // Skip the time, whatever its shape. Only the date reaches the graph.
    std::size_t at = 0;
    while (at < rest.size() && rest[at] != ']' && rest[at] != '-') {
        ++at;
    }
    if (at >= rest.size()) {
        return out;
    }
    // A bracketed line must close its bracket; an unbracketed one is separated by " - ".
    if (bracketed != (rest[at] == ']')) {
        return out;
    }
    rest = rest.substr(at + 1);

    out.date = date;
    out.has_timestamp = true;

    while (!rest.empty() && (rest.front() == ' ' || rest.front() == '\t')) {
        rest.remove_prefix(1);
    }

    // `Sender: text`. A system message has no colon before the first sentence ends, and a
    // sender name containing a colon is not a case worth guessing at — better a system
    // message miscounted than a person invented.
    const auto colon = rest.find(':');
    if (colon == std::string_view::npos) {
        return out;  // system message
    }
    const auto sender = rest.substr(0, colon);
    if (sender.empty() || sender.size() > 128) {
        return out;
    }

    out.sender = std::string{sender};
    return out;
}

std::size_t word_count(std::string_view text) {
    std::size_t words = text.empty() ? 0 : 1;
    for (std::size_t i = 1; i < text.size(); ++i) {
        if (text[i] == ' ' && text[i - 1] != ' ') {
            ++words;
        }
    }
    return words;
}

// Whether a candidate sender is really a system message about another sender.
//
// A system message can carry a colon of its own — "Nelson changed the group description
// to: ..." — and would otherwise become a person named after the sentence. The app does not
// mark these, and a word count cannot separate them: that phrase and "Maria Jose de la Cruz
// Martinez" are both six words.
//
// What does separate them is that the sentence begins with a name the file already uses on
// its own. Nelson sends messages as "Nelson", so a sender that is "Nelson" followed by
// several more words is a sentence about Nelson rather than a second person.
//
// The gap has to be wide, because "Ana" and "Ana Maria" are two plausible contacts. Three
// extra words is the line, and it is a heuristic: a contact saved as "Ana Maria de la Cruz"
// alongside a contact "Ana" would be taken for a system message. That residual case costs
// a suggestion the owner would have ignored; the other direction costs a fabricated person
// in their graph.
bool looks_like_a_sentence_about(std::string_view candidate, std::string_view sender) {
    if (candidate.size() <= sender.size() + 1) {
        return false;
    }
    if (candidate.substr(0, sender.size()) != sender || candidate[sender.size()] != ' ') {
        return false;
    }
    return word_count(candidate) >= word_count(sender) + 3;
}

}  // namespace

ChatExport parse_export(std::string_view text) {
    ChatExport result;

    std::vector<Line> lines;
    std::size_t start = 0;
    while (start <= text.size()) {
        const auto at = text.find('\n', start);
        const auto raw = text.substr(start, at == std::string_view::npos ? std::string_view::npos
                                                                        : at - start);
        if (!strip_invisible(raw).empty()) {
            lines.push_back(read_line(raw));
            if (!lines.back().has_timestamp) {
                // Either a continuation of the previous message or noise. A line with no
                // timestamp following one that had a timestamp is a continuation; anything
                // before the first timestamp is a header the export writes.
                const bool after_a_message =
                    std::ranges::any_of(lines, [](const Line& l) { return l.has_timestamp; });
                if (after_a_message) {
                    ++result.continuation_lines;
                } else {
                    ++result.unparsed_lines;
                }
            }
        }
        if (at == std::string_view::npos) {
            break;
        }
        start = at + 1;
    }

    // Decide the day/month order across the whole export before interpreting any date.
    bool day_first = false;
    bool month_first = false;
    for (const auto& line : lines) {
        if (!line.date.valid) {
            continue;
        }
        if (line.date.first > 12) {
            day_first = true;
        }
        if (line.date.second > 12) {
            month_first = true;
        }
    }
    // Both would mean the file mixes formats, which is not something to resolve by picking
    // one: the honest answer is that the order is unknown.
    result.order = (day_first && !month_first)   ? DateOrder::DayFirst
                   : (month_first && !day_first) ? DateOrder::MonthFirst
                                                 : DateOrder::Ambiguous;

    const auto to_partial = [&result](const RawDate& raw) {
        PartialDate out;
        out.year = raw.year;
        if (result.order == DateOrder::DayFirst) {
            out.day = raw.first;
            out.month = raw.second;
        } else if (result.order == DateOrder::MonthFirst) {
            out.month = raw.first;
            out.day = raw.second;
        }
        return out;
    };

    const auto earlier = [](const PartialDate& a, const PartialDate& b) {
        if (a.year != b.year) {
            return a.year < b.year;
        }
        return a.month.value_or(0) * 100 + a.day.value_or(0) <
               b.month.value_or(0) * 100 + b.day.value_or(0);
    };

    for (const auto& line : lines) {
        if (!line.has_timestamp) {
            continue;
        }
        if (line.sender.empty()) {
            ++result.system_messages;
            continue;
        }
        const auto when = to_partial(line.date);
        auto it = std::ranges::find_if(result.participants, [&line](const Participant& p) {
            return p.display_name == line.sender;
        });
        if (it == result.participants.end()) {
            result.participants.push_back(Participant{line.sender, 1, when, when});
            continue;
        }
        ++it->message_count;
        if (earlier(when, it->first_seen)) {
            it->first_seen = when;
        }
        if (earlier(it->last_seen, when)) {
            it->last_seen = when;
        }
    }

    // Applied once the whole file has been read, because it needs to know every sender the
    // file uses on its own before it can tell which candidates are sentences about them.
    const auto names = result.participants;
    std::erase_if(result.participants, [&names, &result](const Participant& candidate) {
        for (const auto& other : names) {
            if (other.display_name != candidate.display_name &&
                looks_like_a_sentence_about(candidate.display_name, other.display_name)) {
                // Counted rather than discarded. These lines happened; they are just not
                // about a person the graph should hold.
                result.system_messages += candidate.message_count;
                return true;
            }
        }
        return false;
    });

    return result;
}

Suggestion suggest(const ChatExport& parsed,
                   std::string_view owner_id,
                   const std::vector<std::string>& owner_names,
                   std::string_view conversation_label) {
    Suggestion out;

    const auto is_owner = [&owner_names](const std::string& name) {
        return std::ranges::find(owner_names, name) != owner_names.end();
    };

    std::vector<const Participant*> others;
    for (const auto& participant : parsed.participants) {
        if (!is_owner(participant.display_name)) {
            others.push_back(&participant);
        }
    }

    for (const auto* participant : others) {
        Profile profile;
        // The display name is the identifier until the owner says otherwise. Inventing a
        // synthetic id here would make the same person two people the next time a different
        // export spells their name the same way.
        profile.subject.subject_id = participant->display_name;
        profile.subject.kind = SubjectKind::ConsentedPerson;
        profile.subject.display_label = participant->display_name;

        const auto note = std::string{"from a chat export: "} + std::string{conversation_label};

        // "conocida en 2024" — the year is the part that survives an ambiguous date format,
        // and it is the part the graph is actually asked about.
        Trait known_since;
        known_since.key = "known_since";
        known_since.value = participant->first_seen.to_string();
        known_since.basis = TraitBasis::InferredFromMessages;
        known_since.source_note = note;
        profile.traits.push_back(std::move(known_since));

        Trait last_heard;
        last_heard.key = "last_message";
        last_heard.value = participant->last_seen.to_string();
        last_heard.basis = TraitBasis::InferredFromMessages;
        last_heard.source_note = note;
        profile.traits.push_back(std::move(last_heard));

        Trait volume;
        volume.key = "messages_exchanged";
        volume.value = std::to_string(participant->message_count);
        volume.basis = TraitBasis::InferredFromMessages;
        volume.source_note = note;
        profile.traits.push_back(std::move(volume));

        out.profiles.push_back(std::move(profile));
    }

    if (parsed.is_group()) {
        // Deliberate, and said out loud. Two people in the same group is the same inference
        // as two people in the same room — arrived at through a different sensor, refused
        // for the same reason.
        out.note =
            "a group conversation: the people in it are suggested as contacts, but no "
            "relationships between them are proposed. Sharing a group is not knowing each "
            "other, and a hundred groups of that would be a social graph assembled from "
            "membership lists.";
        return out;
    }

    // One-to-one. The owner has a direct, sustained exchange with this person, which is
    // real evidence about the owner's own life — still a hypothesis, but a well-founded one
    // that they can confirm in a second.
    for (const auto* participant : others) {
        RelationEdge edge;
        edge.from_id = std::string{owner_id};
        edge.to_id = participant->display_name;
        edge.kind = RelationKind::Friend;
        edge.basis = EdgeBasis::ModelHypothesis;
        edge.source_note = std::string{"a one-to-one conversation: "} +
                           std::string{conversation_label} + ", " +
                           std::to_string(participant->message_count) + " messages from them. The kind is a guess too: a chat says the owner knows this person, not how.";
        out.hypotheses.push_back(std::move(edge));
    }
    if (!out.hypotheses.empty()) {
        out.note =
            "a one-to-one conversation, so the relationship is proposed as a hypothesis. "
            "Confirm it to move it into the graph; nothing here asserts.";
    }
    return out;
}

}  // namespace sentry::ingest
