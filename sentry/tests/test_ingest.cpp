// Reading a chat export without turning it into a dossier.
//
// Two properties matter more than the parsing: message content never reaches the graph, and
// a group conversation proposes no relationships between the people in it. The rest of these
// cases are about a format that varies by platform, locale and phone, where a wrong guess
// puts someone's "known since" date three weeks or a hundred years out.

#include "harness.hpp"

#include "sentry/ingest/chat_export.hpp"

using namespace sentry::ingest;
using namespace sentry::identity;

namespace {

// A real iOS export shape, including the invisible marks the app writes.
const char* kIosChat =
    "\xEF\xBB\xBF"  // BOM
    "[12/03/2024, 14:32:10] Messages and calls are end-to-end encrypted.\n"
    "\xE2\x80\x8E[12/03/2024, 14:33:02] Nelson: hola\n"
    "[12/03/2024, 14:33:40] Rafa: hola! como vas\n"
    "esta linea continua el mensaje anterior\n"
    "[28/07/2024, 09:11:00] Rafa: te mando la foto\n"
    "[03/01/2025, 20:02:11] Nelson: gracias\n";

const char* kAndroidGroup =
    "12/03/2024, 14:33 - Nelson: hola a todos\n"
    "12/03/2024, 14:34 - Rafa: hola\n"
    "13/03/2024, 09:00 - Ana: buenas\n"
    "14/03/2024, 10:00 - Beto: aqui ando\n";

void the_ios_shape_parses() {
    CASE("bracketed timestamps, BOM and left-to-right marks");

    const auto parsed = parse_export(kIosChat);
    CHECK_EQ(parsed.participants.size(), std::size_t{2});
    CHECK_BECAUSE(parsed.system_messages == 1,
                  "the encryption notice has a timestamp and no sender; it is counted, "
                  "never attributed to a person");
    CHECK_BECAUSE(parsed.continuation_lines == 1,
                  "a line with no timestamp continues the message above it");

    const auto* rafa = parsed.find("Rafa");
    CHECK(rafa != nullptr);
    if (rafa) {
        CHECK_EQ(rafa->message_count, std::size_t{2});
    }
}

void the_android_shape_parses() {
    CASE("dash-separated timestamps");

    const auto parsed = parse_export(kAndroidGroup);
    CHECK_EQ(parsed.participants.size(), std::size_t{4});
    CHECK(parsed.is_group());
}

void invisible_marks_do_not_defeat_the_parse() {
    CASE("a left-to-right mark is not whitespace to isspace");

    // The failure this prevents is total: every line starts with an invisible byte, no
    // line matches, and the export reads as empty with no error anywhere.
    const auto parsed = parse_export("\xE2\x80\x8E[01/02/2024, 10:00:00] Ana: hola\n");
    CHECK_EQ(parsed.participants.size(), std::size_t{1});
}

// ---------------------------------------------------------------- dates

void a_day_above_twelve_settles_the_order() {
    CASE("28/07 can only be day-first");

    const auto parsed = parse_export(kIosChat);
    CHECK_EQ(parsed.order, DateOrder::DayFirst);

    const auto* rafa = parsed.find("Rafa");
    CHECK(rafa != nullptr);
    if (rafa) {
        CHECK_EQ(rafa->first_seen.to_string(), std::string{"2024-03-12"});
        CHECK_EQ(rafa->last_seen.to_string(), std::string{"2024-07-28"});
    }
}

void a_second_component_above_twelve_settles_it_the_other_way() {
    CASE("07/28 can only be month-first");

    const auto parsed = parse_export("[07/28/2024, 09:11:00] Ana: hola\n");
    CHECK_EQ(parsed.order, DateOrder::MonthFirst);
    const auto* ana = parsed.find("Ana");
    CHECK(ana != nullptr);
    if (ana) {
        CHECK_EQ(ana->first_seen.to_string(), std::string{"2024-07-28"});
    }
}

void an_undecidable_export_keeps_only_the_year() {
    CASE("guessing would put the date three weeks out for no benefit");

    // Every component is twelve or under, so nothing in the file says which way round it
    // is. The year is unambiguous and is the part the graph is actually asked about.
    const auto parsed = parse_export(
        "[03/04/2024, 10:00:00] Ana: hola\n"
        "[05/06/2024, 11:00:00] Ana: otra\n");
    CHECK_EQ(parsed.order, DateOrder::Ambiguous);
    const auto* ana = parsed.find("Ana");
    CHECK(ana != nullptr);
    if (ana) {
        CHECK_BECAUSE(ana->first_seen.to_string() == "2024",
                      "the year survives an undecidable format; a fabricated day does not");
        CHECK(!ana->first_seen.day.has_value());
    }
}

void a_file_that_mixes_formats_is_undecidable_rather_than_resolved() {
    CASE("both orders present means neither is known");

    const auto parsed = parse_export(
        "[28/07/2024, 10:00:00] Ana: hola\n"
        "[07/28/2024, 11:00:00] Ana: otra\n");
    CHECK_BECAUSE(parsed.order == DateOrder::Ambiguous,
                  "a file that contradicts itself is not resolved by picking a side");
}

void a_two_digit_year_is_this_century() {
    CASE("24 is 2024, not 1924");

    const auto parsed = parse_export("[28/07/24, 09:11:00] Ana: hola\n");
    const auto* ana = parsed.find("Ana");
    CHECK(ana != nullptr);
    if (ana) {
        CHECK_BECAUSE(ana->first_seen.year == 2024,
                      "reading it as 1924 would put every date in the file a century out");
    }
}

void an_iso_date_needs_no_inference() {
    CASE("a four-digit leading component fixes the order outright");

    const auto parsed = parse_export("[2024-03-05, 09:11:00] Ana: hola\n");
    const auto* ana = parsed.find("Ana");
    CHECK(ana != nullptr);
    if (ana) {
        CHECK_EQ(ana->first_seen.year, 2024);
    }
}

void a_file_that_is_not_an_export_produces_nothing_and_does_not_crash() {
    CASE("a wrong file is a count, not an error code");

    const auto parsed = parse_export("hello\nthis is a shopping list\nmilk\n");
    CHECK(parsed.participants.empty());
    CHECK_EQ(parsed.unparsed_lines, std::size_t{3});
    CHECK(parse_export("").participants.empty());
}

// ---------------------------------------------------------------- what reaches the graph

void a_system_message_with_a_colon_does_not_invent_a_person() {
    CASE("the format does not mark system messages, so this is a heuristic");

    // Without the guard, "Nelson changed the group description to: algo" becomes a person
    // named after the sentence, and that person then has a profile in the owner's graph.
    const auto parsed = parse_export(
        "[12/03/2024, 14:33:02] Nelson: hola\n"
        "[12/03/2024, 14:34:00] Nelson changed the group description to: algo\n");

    CHECK_EQ(parsed.participants.size(), std::size_t{1});
    CHECK_BECAUSE(parsed.system_messages == 1,
                  "miscounting a system message costs a number; inventing a person costs "
                  "a record in someone's graph");

    // A word count cannot do this: that phrase and "Maria Jose de la Cruz Martinez" are
    // both six words. What separates them is that the sentence begins with a name the file
    // already uses on its own, so a long real name still parses when no shorter form of it
    // appears as a sender.
    const auto long_name = parse_export(
        "[12/03/2024, 14:33:02] Maria Jose de la Cruz Martinez: hola\n");
    CHECK_EQ(long_name.participants.size(), std::size_t{1});
}

void message_content_never_reaches_the_graph() {
    CASE("a relationship graph is not a place to keep conversations");

    // The failure this prevents is not a leak so much as a slow accumulation: a person
    // graph that quietly kept the text of every conversation would become the most
    // damaging file on the machine, as a side effect of a feature about relationships.
    const auto parsed = parse_export(kIosChat);
    const auto suggested = suggest(parsed, "nelson", {"Nelson"}, "Rafa");

    for (const auto& profile : suggested.profiles) {
        for (const auto& trait : profile.traits) {
            CHECK_BECAUSE(trait.value.find("hola") == std::string::npos &&
                              trait.value.find("foto") == std::string::npos &&
                              trait.value.find("gracias") == std::string::npos,
                          "no message text may appear in a trait value");
            CHECK(trait.source_note.find("hola") == std::string::npos);
        }
    }
}

void a_group_proposes_no_relationships_between_its_members() {
    CASE("sharing a group is not knowing each other");

    // The same inference as two people in the same room, arrived at through a different
    // sensor. A hundred groups of it is a social graph assembled from membership lists.
    const auto parsed = parse_export(kAndroidGroup);
    const auto suggested = suggest(parsed, "nelson", {"Nelson"}, "Los del barrio");

    CHECK_BECAUSE(suggested.hypotheses.empty(),
                  "a group conversation proposes no edges at all, not even weak ones");
    CHECK_BECAUSE(!suggested.note.empty(),
                  "the absence must be explained, or it looks like the parse failed");
    CHECK_BECAUSE(suggested.profiles.size() == 3,
                  "the people are still suggested as contacts; it is the edges between "
                  "them that are refused");
}

void a_one_to_one_conversation_proposes_a_hypothesis_and_never_an_assertion() {
    CASE("nothing read out of a chat is asserted");

    const auto parsed = parse_export(kIosChat);
    const auto suggested = suggest(parsed, "nelson", {"Nelson"}, "Rafa");

    CHECK_EQ(suggested.hypotheses.size(), std::size_t{1});
    if (!suggested.hypotheses.empty()) {
        const auto& edge = suggested.hypotheses[0];
        CHECK_EQ(edge.from_id, std::string{"nelson"});
        CHECK_EQ(edge.to_id, std::string{"Rafa"});
        CHECK_BECAUSE(edge.basis == EdgeBasis::ModelHypothesis,
                      "a name in an export is evidence the owner knows someone, not a "
                      "statement about who they are");

        // And it is refused as an assertion, by the same function the store uses. This is
        // the property that matters: it cannot be moved into the asserted graph by
        // accident, only by the owner deciding to.
        SubjectRecord owner;
        owner.subject_id = "nelson";
        owner.kind = SubjectKind::Owner;
        SubjectRecord rafa;
        rafa.subject_id = "Rafa";
        rafa.kind = SubjectKind::ConsentedPerson;
        CHECK_EQ(refuse_edge(edge, owner, rafa, "nelson"),
                 std::optional{EdgeRefusal::GenerativeCannotAssert});
    }
}

void every_trait_from_a_chat_is_a_hypothesis() {
    CASE("a reading of a sentence is not a confirmed fact");

    const auto parsed = parse_export(kIosChat);
    const auto suggested = suggest(parsed, "nelson", {"Nelson"}, "Rafa");
    CHECK(!suggested.profiles.empty());
    for (const auto& profile : suggested.profiles) {
        for (const auto& trait : profile.traits) {
            CHECK_EQ(trait.basis, TraitBasis::InferredFromMessages);
            CHECK(trait.is_hypothesis());
        }
        // So `asserted()` finds nothing until a person confirms it.
        CHECK_BECAUSE(profile.asserted("known_since") == nullptr,
                      "until the owner confirms, the profile must read as 'suspected', "
                      "not as 'known'");
        CHECK(profile.any("known_since") != nullptr);
    }
}

void the_owner_is_not_suggested_as_their_own_contact() {
    CASE("the account holder appears as a sender like anyone else");

    const auto parsed = parse_export(kIosChat);
    const auto suggested = suggest(parsed, "nelson", {"Nelson"}, "Rafa");
    for (const auto& profile : suggested.profiles) {
        CHECK(profile.subject.subject_id != "Nelson");
    }
    CHECK_EQ(suggested.profiles.size(), std::size_t{1});
}

void known_since_is_the_answer_the_owner_asked_for() {
    CASE("conocida en 2024");

    const auto parsed = parse_export(kIosChat);
    const auto suggested = suggest(parsed, "nelson", {"Nelson"}, "Rafa");
    CHECK(!suggested.profiles.empty());
    if (!suggested.profiles.empty()) {
        const auto* known = suggested.profiles[0].any("known_since");
        CHECK(known != nullptr);
        if (known) {
            CHECK_EQ(known->value, std::string{"2024-03-12"});
        }
        const auto* volume = suggested.profiles[0].any("messages_exchanged");
        CHECK(volume != nullptr);
        if (volume) {
            CHECK_EQ(volume->value, std::string{"2"});
        }
    }
}

void a_suggested_contact_is_a_person_not_a_public_figure() {
    CASE("someone in the owner's chats is someone in the owner's life");

    // The kind decides what may be recorded. A contact from a private conversation is a
    // `ConsentedPerson` — which is what puts them under the consent machinery rather than
    // under §40's public-record rules, where their hobbies would be refused.
    const auto parsed = parse_export(kIosChat);
    const auto suggested = suggest(parsed, "nelson", {"Nelson"}, "Rafa");
    for (const auto& profile : suggested.profiles) {
        CHECK_EQ(profile.subject.kind, SubjectKind::ConsentedPerson);
        for (const auto& trait : profile.traits) {
            CHECK(!refuse_trait(profile.subject, trait));
        }
    }
}

}  // namespace

int main() {
    the_ios_shape_parses();
    the_android_shape_parses();
    invisible_marks_do_not_defeat_the_parse();

    a_day_above_twelve_settles_the_order();
    a_second_component_above_twelve_settles_it_the_other_way();
    an_undecidable_export_keeps_only_the_year();
    a_file_that_mixes_formats_is_undecidable_rather_than_resolved();
    a_two_digit_year_is_this_century();
    an_iso_date_needs_no_inference();
    a_file_that_is_not_an_export_produces_nothing_and_does_not_crash();

    a_system_message_with_a_colon_does_not_invent_a_person();
    message_content_never_reaches_the_graph();
    a_group_proposes_no_relationships_between_its_members();
    a_one_to_one_conversation_proposes_a_hypothesis_and_never_an_assertion();
    every_trait_from_a_chat_is_a_hypothesis();
    the_owner_is_not_suggested_as_their_own_contact();
    known_since_is_the_answer_the_owner_asked_for();
    a_suggested_contact_is_a_person_not_a_public_figure();

    return sentry::test::report();
}
