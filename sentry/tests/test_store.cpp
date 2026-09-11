// The store, and the fact that reading is a trust boundary.
//
// Most of these cases describe a file that is already wrong when the program opens it —
// hand-edited, restored from a backup taken before a rule existed, written by an older
// build, synchronised from another machine. Validating on write and trusting the read is
// the natural design and it is the one that lets every one of those situations put a record
// into the graph that the rules would refuse today.

#include "harness.hpp"

#include "sentry/store/graph_store.hpp"

#include <filesystem>
#include <fstream>

using namespace sentry::store;
using namespace sentry::identity;

namespace {

Profile subject(std::string id, SubjectKind kind) {
    Profile p;
    p.subject.subject_id = std::move(id);
    p.subject.kind = kind;
    p.subject.display_label = p.subject.subject_id;
    return p;
}

Trait trait(std::string key, std::string value, TraitBasis basis) {
    Trait t;
    t.key = std::move(key);
    t.value = std::move(value);
    t.basis = basis;
    return t;
}

RelationEdge edge(std::string from, std::string to, RelationKind kind, EdgeBasis basis,
                  std::string declarer) {
    RelationEdge e;
    e.from_id = std::move(from);
    e.to_id = std::move(to);
    e.kind = kind;
    e.basis = basis;
    e.declarer_id = std::move(declarer);
    return e;
}

PersonGraph sample() {
    PersonGraph g;
    g.owner_id = "nelson";
    g.profiles.push_back(subject("nelson", SubjectKind::Owner));
    auto rafa = subject("rafa", SubjectKind::ConsentedPerson);
    rafa.traits.push_back(trait("age", "14", TraitBasis::SubjectDeclared));
    rafa.traits.push_back(trait("eye_colour", "brown", TraitBasis::OwnerDeclared));
    rafa.traits.push_back(trait("hobbies", "skate", TraitBasis::InferredFromMessages));
    g.profiles.push_back(std::move(rafa));
    g.profiles.push_back(subject("ana", SubjectKind::ConsentedPerson));
    g.asserted_edges.push_back(
        edge("nelson", "rafa", RelationKind::Friend, EdgeBasis::OwnerDeclared, "nelson"));
    g.asserted_edges.push_back(
        edge("nelson", "ana", RelationKind::Friend, EdgeBasis::OwnerDeclared, "nelson"));
    return g;
}

std::filesystem::path scratch(std::string_view name) {
    auto dir = std::filesystem::temp_directory_path() / "sentry-store-tests";
    std::error_code ec;
    std::filesystem::create_directories(dir, ec);
    return dir / name;
}

std::string with_header(std::string body) {
    return "v\t1\nowner\tnelson\nsubject\tnelson\towner\tNelson\n" + std::move(body);
}

// ---------------------------------------------------------------- round trip

void a_graph_survives_a_round_trip() {
    CASE("what goes in comes back");

    const auto original = sample();
    const auto reloaded = parse(serialise(original));
    CHECK(reloaded.has_value());
    if (!reloaded) {
        return;
    }

    CHECK(reloaded->clean());
    CHECK_EQ(reloaded->graph.owner_id, std::string{"nelson"});
    CHECK_EQ(reloaded->graph.profiles.size(), std::size_t{3});
    CHECK_EQ(reloaded->graph.asserted_edges.size(), std::size_t{2});

    const auto* rafa = reloaded->graph.find("rafa");
    CHECK(rafa != nullptr);
    if (rafa) {
        CHECK_EQ(rafa->traits.size(), std::size_t{3});
        // The standing of a trait has to survive, or a chat-derived guess comes back
        // indistinguishable from something the person confirmed.
        CHECK_BECAUSE(rafa->asserted("age") != nullptr && rafa->asserted("age")->value == "14",
                      "a confirmed value must still read as confirmed after a restart");
        CHECK_BECAUSE(rafa->asserted("hobbies") == nullptr,
                      "and a guess must still read as a guess");
    }

    // And the reconstructed graph traverses.
    const auto graph = reloaded->graph.to_relation_graph();
    CHECK_EQ(graph.degree("rafa"), std::optional{std::size_t{1}});
    CHECK_EQ(graph.mutual("rafa", "ana"), std::vector<std::string>{"nelson"});
}

void awkward_characters_survive() {
    CASE("tabs, newlines and backslashes in a value");

    // The format's two structural characters are exactly the two a person will eventually
    // paste into a note.
    PersonGraph g;
    g.owner_id = "nelson";
    auto p = subject("nelson", SubjectKind::Owner);
    auto t = trait("note", "line one\nline\ttwo\\three\r\nend", TraitBasis::OwnerDeclared);
    t.source_note = "a note\twith\ttabs";
    p.traits.push_back(t);
    g.profiles.push_back(std::move(p));

    const auto reloaded = parse(serialise(g));
    CHECK(reloaded.has_value());
    if (reloaded) {
        CHECK(reloaded->clean());
        const auto* back = reloaded->graph.find("nelson");
        CHECK(back != nullptr);
        if (back && !back->traits.empty()) {
            CHECK_EQ(back->traits[0].value,
                     std::string{"line one\nline\ttwo\\three\r\nend"});
            CHECK_EQ(back->traits[0].source_note, std::string{"a note\twith\ttabs"});
        }
    }
}

void an_empty_graph_round_trips() {
    CASE("a first run is not an error state");

    PersonGraph g;
    g.owner_id = "nelson";
    const auto reloaded = parse(serialise(g));
    CHECK(reloaded.has_value());
    if (reloaded) {
        CHECK(reloaded->graph.profiles.empty());
        CHECK_EQ(reloaded->graph.owner_id, std::string{"nelson"});
    }
}

// ---------------------------------------------------------------- the trust boundary

void a_hand_edited_family_edge_is_refused_on_load() {
    CASE("a rule that only holds at write time is not a rule");

    // §40: family is not read out of the public record. Someone edits the file, or it
    // predates the rule. The load path is the only place left to catch it.
    const auto text = with_header(
        "subject\tmama\tconsented_person\tMama\n"
        "edge\tnelson\tmama\tfamily\tpublic_record\t\tfrom a directory\n");

    const auto loaded = parse(text);
    CHECK(loaded.has_value());
    if (loaded) {
        CHECK_BECAUSE(loaded->graph.asserted_edges.empty(),
                      "the edge must not enter the graph merely because it is already in "
                      "the file");
        CHECK_EQ(loaded->rejections.size(), std::size_t{1});
        CHECK(!loaded->rejections.empty());
        if (loaded->rejections.empty()) { return; }
        CHECK_EQ(loaded->rejections[0].kind, RejectionKind::RefusedByPolicy);
        CHECK_BECAUSE(loaded->rejections[0].line == 5,
                      "the line number is what makes a rejection actionable");
    }
}

void a_co_presence_edge_in_the_file_is_refused_on_load() {
    CASE("the edge that must not exist does not get in through the disk");

    const auto text = with_header(
        "subject\tstranger\tconsented_person\tStranger\n"
        "edge\tnelson\tstranger\tcolleague\tco_presence_inference\tnelson\tseen together\n");

    const auto loaded = parse(text);
    CHECK(loaded.has_value());
    if (loaded) {
        CHECK(loaded->graph.asserted_edges.empty());
        CHECK_EQ(loaded->rejections.size(), std::size_t{1});
        CHECK(!loaded->rejections.empty());
        if (!loaded->rejections.empty()) {
            CHECK_EQ(loaded->rejections[0].kind, RejectionKind::RefusedByPolicy);
        }
    }
}

void a_model_hypothesis_cannot_be_promoted_by_editing_the_file() {
    CASE("changing hyp to edge does not make a guess an assertion");

    const auto text = with_header(
        "subject\tx\tconsented_person\tX\n"
        "edge\tnelson\tx\tcolleague\tmodel_hypothesis\t\t\n");

    const auto loaded = parse(text);
    CHECK(loaded.has_value());
    if (loaded) {
        CHECK_BECAUSE(loaded->graph.asserted_edges.empty(),
                      "the basis travels with the edge, so relabelling the record type "
                      "changes nothing");
        CHECK(!loaded->rejections.empty());
        if (!loaded->rejections.empty()) {
            CHECK_EQ(loaded->rejections[0].kind, RejectionKind::RefusedByPolicy);
        }
    }
}

void a_private_trait_on_a_public_figure_is_refused_on_load() {
    CASE("§40 is re-checked when the file is read");

    const auto text = with_header(
        "subject\treporter\tpublic_figure\tReporter\n"
        "trait\treporter\thome_address\tCalle Falsa 123\tpublic_record\t-\t\n"
        "trait\treporter\temployer\tEl Pais\tpublic_record\t-\t\n");

    const auto loaded = parse(text);
    CHECK(loaded.has_value());
    if (loaded) {
        const auto* reporter = loaded->graph.find("reporter");
        CHECK(reporter != nullptr);
        if (reporter) {
            CHECK_BECAUSE(reporter->traits.size() == 1,
                          "the address is dropped and the professional record is kept");
            CHECK_EQ(reporter->traits[0].key, std::string{"employer"});
        }
        CHECK_EQ(loaded->rejections.size(), std::size_t{1});
        if (!loaded->rejections.empty()) {
            CHECK_EQ(loaded->rejections[0].kind, RejectionKind::RefusedByPolicy);
        }
    }
}

void a_rejection_never_quotes_the_value_it_rejected() {
    CASE("a rejection message is not a place to publish an address");

    const auto text = with_header(
        "subject\treporter\tpublic_figure\tReporter\n"
        "trait\treporter\thome_address\tCalle Falsa 123\tpublic_record\t-\t\n");

    const auto loaded = parse(text);
    CHECK(loaded.has_value());
    if (loaded && !loaded->rejections.empty()) {
        CHECK_BECAUSE(loaded->rejections[0].detail.find("Calle Falsa") == std::string::npos,
                      "a message about a refused home address that quotes the address has "
                      "written it into a log file with none of the store's protections");
    }
}

void the_owner_still_overrides_it_by_declaring_the_subject_differently() {
    CASE("the refusal is about the capacity, and the owner sets the capacity");

    // Not a loophole — the point. §40 protects people Sentry holds *because they are in the
    // news*. The owner deciding someone is a person in their life rather than a public
    // figure is exactly the override, recorded in the file where it can be seen.
    const auto text = with_header(
        "subject\treporter\tconsented_person\tReporter\n"
        "trait\treporter\thome_address\tCalle Falsa 123\towner_declared\t-\t\n");

    const auto loaded = parse(text);
    CHECK(loaded.has_value());
    if (loaded) {
        CHECK(loaded->clean());
        const auto* reporter = loaded->graph.find("reporter");
        CHECK(reporter != nullptr);
        if (reporter) {
            CHECK_EQ(reporter->traits.size(), std::size_t{1});
        }
    }
}

// ---------------------------------------------------------------- malformed input

void one_bad_line_does_not_cost_the_graph() {
    CASE("a corrupt byte must not erase the relationship history");

    const auto text = with_header(
        "subject\trafa\tconsented_person\tRafa\n"
        "this line is nonsense\n"
        "trait\trafa\tage\t14\tsubject_declared\t-\t\n"
        "subject\tana\tnot_a_real_kind\tAna\n"
        "edge\tnelson\trafa\tfriend\towner_declared\tnelson\t\n");

    const auto loaded = parse(text);
    CHECK(loaded.has_value());
    if (loaded) {
        CHECK_EQ(loaded->graph.profiles.size(), std::size_t{2});  // nelson, rafa
        CHECK_EQ(loaded->graph.asserted_edges.size(), std::size_t{1});
        CHECK_EQ(loaded->rejections.size(), std::size_t{2});
        const auto* rafa = loaded->graph.find("rafa");
        CHECK(rafa != nullptr);
        if (rafa) {
            CHECK_BECAUSE(rafa->traits.size() == 1,
                          "everything after the bad line still loads");
        }
    }
}

void a_dangling_backslash_fails_its_line_and_nothing_else() {
    CASE("an escape this build does not define is not guessed at");

    const auto text = with_header(
        "subject\trafa\tconsented_person\tRafa\\\n"
        "subject\tana\tconsented_person\tAna\n");

    const auto loaded = parse(text);
    CHECK(loaded.has_value());
    if (loaded) {
        CHECK(loaded->graph.find("rafa") == nullptr);
        CHECK(loaded->graph.find("ana") != nullptr);
        CHECK(!loaded->rejections.empty());
        if (!loaded->rejections.empty()) {
            CHECK_EQ(loaded->rejections[0].kind, RejectionKind::Malformed);
        }
    }

    // An escape nobody defined is refused rather than passed through, because guessing is
    // how a parser starts accepting data it should not.
    const auto unknown = parse(with_header("subject\tx\tconsented_person\tA\\qB\n"));
    CHECK(unknown.has_value());
    if (unknown) {
        CHECK_EQ(unknown->rejections.size(), std::size_t{1});
    }
}

void an_edge_to_an_undeclared_subject_is_reported_as_such() {
    CASE("unknown subject is its own rejection kind");

    const auto text = with_header("edge\tnelson\tghost\tfriend\towner_declared\tnelson\t\n");
    const auto loaded = parse(text);
    CHECK(loaded.has_value());
    if (loaded) {
        CHECK(loaded->graph.asserted_edges.empty());
        CHECK(!loaded->rejections.empty());
        if (!loaded->rejections.empty()) {
            CHECK_EQ(loaded->rejections[0].kind, RejectionKind::UnknownSubject);
        }
    }
}

void a_subject_declared_twice_is_not_merged() {
    CASE("a store that guesses which declaration is current changes someone's record");

    const auto text = with_header(
        "subject\trafa\tconsented_person\tRafa\n"
        "subject\trafa\tpublic_figure\tRafa Again\n");

    const auto loaded = parse(text);
    CHECK(loaded.has_value());
    if (loaded) {
        CHECK_EQ(loaded->graph.profiles.size(), std::size_t{2});  // nelson, rafa
        CHECK(!loaded->rejections.empty());
        if (!loaded->rejections.empty()) {
            CHECK_EQ(loaded->rejections[0].kind, RejectionKind::DuplicateSubject);
        }
        const auto* rafa = loaded->graph.find("rafa");
        CHECK(rafa != nullptr);
        if (rafa) {
            CHECK_BECAUSE(rafa->subject.kind == SubjectKind::ConsentedPerson,
                          "the first declaration stands; a second one that could silently "
                          "reclassify someone is refused");
        }
    }
}

void carriage_returns_do_not_destroy_the_graph() {
    CASE("a store edited on Windows still loads on Arch");

    const auto loaded = parse("v\t1\r\nowner\tnelson\r\nsubject\tnelson\towner\tNelson\r\n");
    CHECK(loaded.has_value());
    if (loaded) {
        CHECK(loaded->clean());
        CHECK_EQ(loaded->graph.profiles.size(), std::size_t{1});
    }
}

// ---------------------------------------------------------------- fatal headers

void a_file_without_a_version_is_not_interpreted() {
    CASE("an unversioned file is fatal, not a rejection");

    CHECK_EQ(parse("owner\tnelson\n").error(), StoreError::MissingVersion);
    CHECK_EQ(parse("").error(), StoreError::MissingVersion);
    CHECK_EQ(parse("v\tnot-a-number\n").error(), StoreError::MissingVersion);
}

void a_future_format_is_refused_whole() {
    CASE("an unknown version means every line might mean something else");

    // Deliberately not "skip what we do not understand". A format change can alter the
    // meaning of a field that still parses, and a partial read of it would be silently
    // wrong rather than loudly broken.
    CHECK_EQ(parse("v\t2\nowner\tnelson\n").error(), StoreError::UnsupportedVersion);
}

void a_file_with_no_owner_has_no_root() {
    CASE("without a root nothing in the file is justified");

    CHECK_EQ(parse("v\t1\nsubject\tx\tconsented_person\tX\n").error(),
             StoreError::MissingOwner);
}

// ---------------------------------------------------------------- disk

void saving_and_loading_a_real_file() {
    CASE("the graph reaches a disk and comes back");

    const auto path = scratch("round-trip.tsv");
    std::error_code ec;
    std::filesystem::remove(path, ec);

    const GraphStore store{path};
    CHECK_BECAUSE(store.load().error() == StoreError::NotFound,
                  "a first run and a truncated store are not the same situation");

    const auto written = store.save(sample());
    CHECK(written.has_value());
    if (written) {
        CHECK_EQ(*written, std::size_t{3});
    }

    const auto loaded = store.load();
    CHECK(loaded.has_value());
    if (loaded) {
        CHECK(loaded->clean());
        CHECK_EQ(loaded->graph.profiles.size(), std::size_t{3});
        CHECK_EQ(loaded->graph.asserted_edges.size(), std::size_t{2});
    }

    // No temporary left behind.
    bool stray = false;
    for (const auto& entry : std::filesystem::directory_iterator(path.parent_path())) {
        if (entry.path().string().find(".tmp.") != std::string::npos) {
            stray = true;
        }
    }
    CHECK_BECAUSE(!stray, "a successful save leaves no temporary file");

    std::filesystem::remove(path, ec);
}

void saving_refuses_what_loading_would_refuse() {
    CASE("a store that writes what it cannot read loses data every round trip");

    PersonGraph g;
    g.owner_id = "nelson";
    g.profiles.push_back(subject("nelson", SubjectKind::Owner));
    auto reporter = subject("reporter", SubjectKind::PublicFigure);
    reporter.traits.push_back(trait("home_address", "x", TraitBasis::PublicRecord));
    reporter.traits.push_back(trait("employer", "y", TraitBasis::PublicRecord));
    g.profiles.push_back(std::move(reporter));
    g.asserted_edges.push_back(
        edge("nelson", "reporter", RelationKind::Family, EdgeBasis::PublicRecord, ""));

    const auto path = scratch("refused.tsv");
    std::error_code ec;
    std::filesystem::remove(path, ec);

    const GraphStore store{path};
    CHECK(store.save(g).has_value());

    const auto loaded = store.load();
    CHECK(loaded.has_value());
    if (loaded) {
        CHECK_BECAUSE(loaded->clean(),
                      "nothing refusable reached the file, so the read is clean rather "
                      "than full of rejections describing the writer's own output");
        const auto* reporter = loaded->graph.find("reporter");
        CHECK(reporter != nullptr);
        if (reporter) {
            CHECK_EQ(reporter->traits.size(), std::size_t{1});
        }
        CHECK(loaded->graph.asserted_edges.empty());
    }

    std::filesystem::remove(path, ec);
}

void a_failed_save_leaves_the_previous_graph_intact() {
    CASE("a write that cannot happen does not destroy what is there");

    const auto path = scratch("intact.tsv");
    std::error_code ec;
    std::filesystem::remove(path, ec);

    const GraphStore store{path};
    CHECK(store.save(sample()).has_value());

    // A store whose directory does not exist cannot be written. The original is untouched
    // because the temporary, not the target, is what the write touches first.
    const GraphStore doomed{scratch("no-such-dir") / "nested" / "graph.tsv"};
    CHECK_EQ(doomed.save(sample()).error(), StoreError::Unwritable);

    const auto still_there = store.load();
    CHECK(still_there.has_value());
    if (still_there) {
        CHECK_EQ(still_there->graph.profiles.size(), std::size_t{3});
    }

    std::filesystem::remove(path, ec);
}

}  // namespace

int main() {
    a_graph_survives_a_round_trip();
    awkward_characters_survive();
    an_empty_graph_round_trips();

    a_hand_edited_family_edge_is_refused_on_load();
    a_co_presence_edge_in_the_file_is_refused_on_load();
    a_model_hypothesis_cannot_be_promoted_by_editing_the_file();
    a_private_trait_on_a_public_figure_is_refused_on_load();
    a_rejection_never_quotes_the_value_it_rejected();
    the_owner_still_overrides_it_by_declaring_the_subject_differently();

    one_bad_line_does_not_cost_the_graph();
    a_dangling_backslash_fails_its_line_and_nothing_else();
    an_edge_to_an_undeclared_subject_is_reported_as_such();
    a_subject_declared_twice_is_not_merged();
    carriage_returns_do_not_destroy_the_graph();

    a_file_without_a_version_is_not_interpreted();
    a_future_format_is_refused_whole();
    a_file_with_no_owner_has_no_root();

    saving_and_loading_a_real_file();
    saving_refuses_what_loading_would_refuse();
    a_failed_save_leaves_the_previous_graph_intact();

    return sentry::test::report();
}
