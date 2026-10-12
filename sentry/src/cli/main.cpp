// `archie-sentry` — the person graph from a terminal.
//
// RFC: docs/RFC-008-SENTRY-WORLD-MODEL.md §Identity
//
// The first consumer of `sentry::world`. Until something links the library, its rules are a
// compiler's opinion; this is the program that makes them apply to a real file on a real
// machine, and it is how the owner seeds the graph before any camera or message ingestion
// exists to fill it automatically.
//
// Every mutation goes through the same refusals the store applies on load. A record this
// tool would refuse to add is a record that could not survive a restart anyway, so refusing
// it at the prompt — with the reason — is better than writing something that quietly
// disappears.
//
// # Sensitive values are hidden unless asked for
//
// `show` redacts anything classified `Sensitive` and prints a count instead. Not because
// the owner may not see it — it is their graph, and `--all` prints everything — but because
// a terminal is a thing people screen-share, and a home address that appears without being
// asked for is one that appears in a recording nobody was thinking about.

#include "sentry/ingest/chat_export.hpp"
#include "sentry/store/graph_store.hpp"
#include "sentry/store/wanda_projection.hpp"

#include <algorithm>
#include <array>
#include <cstdio>
#include <fstream>
#include <cstdlib>
#include <map>
#include <set>
#include <string>
#include <string_view>
#include <vector>

using namespace sentry::identity;
using namespace sentry::store;

namespace {

// Exit codes, so a script can tell a refusal from a broken store.
constexpr int kOk = 0;
constexpr int kUsage = 64;        // EX_USAGE
constexpr int kStoreProblem = 65; // EX_DATAERR
constexpr int kRefused = 77;      // EX_NOPERM

std::filesystem::path graph_path() {
    if (const char* from_env = std::getenv("ARCHIE_SENTRY_GRAPH")) {
        return std::filesystem::path{from_env};
    }
    if (const char* state = std::getenv("STATE_DIRECTORY")) {
        return std::filesystem::path{state} / "graph.tsv";
    }
    return std::filesystem::path{"/var/lib/archie/sentry/graph.tsv"};
}

void print_usage() {
    std::puts(
        "archie-sentry — the person graph\n"
        "\n"
        "  show [--all]                 the graph as a tree rooted at the owner\n"
        "  export-json [--all]          bounded Wanda graph projection (schema 1)\n"
        "  person <id> [--all]          one profile\n"
        "  init <owner-id> [label]      create the graph, rooted at you\n"
        "  add-person <id> <kind> [label]\n"
        "                               kind: owner | consented_person | public_figure\n"
        "  add-edge <from> <to> <kind> <basis> [declarer] [note]\n"
        "  set <subject> <key> <value> <basis> [note]\n"
        "                               basis: owner_declared | subject_declared |\n"
        "                                      public_record | inferred_from_messages |\n"
        "                                      model_inferred\n"
        "  mutual <a> <b>               people they both know\n"
        "  path <id>                    why this person is in the graph\n"
        "  check                        orphans, rejections, and what was refused\n"
        "  ingest <file> <your-name> [label] [--apply]\n"
        "                               read a WhatsApp export. Shows what it would add;\n"
        "                               --apply writes it. Everything it finds is a\n"
        "                               hypothesis for you to confirm.\n"
        "\n"
        "The graph lives at $ARCHIE_SENTRY_GRAPH, else $STATE_DIRECTORY/graph.tsv,\n"
        "else /var/lib/archie/sentry/graph.tsv.");
}

int fail(std::string_view message, int code) {
    std::fprintf(stderr, "archie-sentry: %.*s\n", static_cast<int>(message.size()),
                 message.data());
    return code;
}

// Load, reporting rejections to stderr. A record the file held and the rules refuse is not
// a reason to stop, but it is always a reason to say something: a graph that silently
// shrinks between restarts is the kind of bug nobody finds.
std::optional<PersonGraph> load_or_report(const GraphStore& store) {
    auto loaded = store.load();
    if (!loaded) {
        std::fprintf(stderr, "archie-sentry: %s (%s)\n",
                     std::string{describe(loaded.error())}.c_str(),
                     store.path().string().c_str());
        return std::nullopt;
    }
    for (const auto& rejection : loaded->rejections) {
        std::fprintf(stderr, "archie-sentry: line %zu refused (%s): %s\n", rejection.line,
                     std::string{name(rejection.kind)}.c_str(), rejection.detail.c_str());
    }
    return std::move(loaded->graph);
}

bool save_or_report(const GraphStore& store, const PersonGraph& graph) {
    const auto written = store.save(graph);
    if (!written) {
        std::fprintf(stderr, "archie-sentry: %s\n",
                     std::string{describe(written.error())}.c_str());
        return false;
    }
    return true;
}

template <typename E, std::size_t N>
std::optional<E> parse_enum(std::string_view text, const std::array<E, N>& all) {
    for (const auto value : all) {
        if (name(value) == text) {
            return value;
        }
    }
    return std::nullopt;
}

constexpr std::array kSubjectKinds{SubjectKind::Owner, SubjectKind::ConsentedPerson,
                                   SubjectKind::PublicFigure, SubjectKind::AnonymousTrack};
constexpr std::array kTraitBases{TraitBasis::OwnerDeclared, TraitBasis::SubjectDeclared,
                                 TraitBasis::PublicRecord, TraitBasis::InferredFromMessages,
                                 TraitBasis::ModelInferred};
constexpr std::array kRelationKinds{
    RelationKind::Colleague, RelationKind::Employer,   RelationKind::Coauthor,
    RelationKind::PublishedBy, RelationKind::Represents, RelationKind::Household,
    RelationKind::Family,    RelationKind::Friend,     RelationKind::EmergencyContact,
    RelationKind::AuthorizedGuest, RelationKind::Partner, RelationKind::Manager};
constexpr std::array kEdgeBases{EdgeBasis::OwnerDeclared, EdgeBasis::SubjectDeclared,
                                EdgeBasis::PublicRecord, EdgeBasis::ModelHypothesis,
                                EdgeBasis::CoPresenceInference};

void print_trait(const Trait& trait, bool reveal) {
    const bool hide = !reveal && trait.effective_sensitivity() == Sensitivity::Sensitive;
    std::printf("    %-18s %-28s [%s%s]\n", trait.key.c_str(),
                hide ? "(sensitive — use --all)" : trait.value.c_str(),
                std::string{name(trait.basis)}.c_str(),
                trait.is_hypothesis() ? ", hypothesis" : "");
}

void print_profile(const Profile& profile, bool reveal) {
    std::printf("  %s (%s)\n", profile.subject.display_label.empty()
                                   ? profile.subject.subject_id.c_str()
                                   : profile.subject.display_label.c_str(),
                std::string{name(profile.subject.kind)}.c_str());
    for (const auto& trait : profile.traits) {
        print_trait(trait, reveal);
    }
}

int cmd_show(const PersonGraph& graph, bool reveal) {
    const auto relations = graph.to_relation_graph();

    // Ordered by distance from the owner, because that is the order in which the graph
    // makes sense: everyone is here because of a chain back to the root, and printing the
    // chain length beside them shows it.
    std::map<std::size_t, std::vector<const Profile*>> by_degree;
    std::vector<const Profile*> unreachable;
    for (const auto& profile : graph.profiles) {
        if (const auto degree = relations.degree(profile.subject.subject_id)) {
            by_degree[*degree].push_back(&profile);
        } else {
            unreachable.push_back(&profile);
        }
    }

    std::printf("graph rooted at %s — %zu people, %zu relationships, %zu hypotheses\n\n",
                graph.owner_id.c_str(), graph.profiles.size(), graph.asserted_edges.size(),
                graph.hypotheses.size());

    for (const auto& [degree, profiles] : by_degree) {
        std::printf("%zu %s away:\n", degree,
                    degree == 1 ? "relationship" : "relationships");
        for (const auto* profile : profiles) {
            print_profile(*profile, reveal);
        }
        std::puts("");
    }

    // Someone reachable only through an unconfirmed hypothesis is not an orphan. Lumping
    // the two together made the ingester look broken: it had just added a person and
    // correctly declined to assert the relationship, and `show` reported the result as a
    // record with no reason to exist.
    std::vector<const Profile*> awaiting;
    std::vector<const Profile*> orphaned;
    for (const auto* profile : unreachable) {
        const auto& id = profile->subject.subject_id;
        const bool proposed = std::ranges::any_of(graph.hypotheses, [&id](const auto& edge) {
            return edge.from_id == id || edge.to_id == id;
        });
        (proposed ? awaiting : orphaned).push_back(profile);
    }

    if (!awaiting.empty()) {
        std::printf("proposed, not yet confirmed (%zu):\n", awaiting.size());
        for (const auto* profile : awaiting) {
            print_profile(*profile, reveal);
        }
        std::puts("  Confirm one with `add-edge` and it joins the graph.\n");
    }

    if (!orphaned.empty()) {
        // Not deleted. A record with no chain to the root is evidence of a bug, and
        // removing it would hide the bug that produced it.
        std::printf("no path to the owner (%zu):\n", orphaned.size());
        for (const auto* profile : orphaned) {
            print_profile(*profile, reveal);
        }
    }
    return kOk;
}

int cmd_check(const PersonGraph& graph) {
    const auto relations = graph.to_relation_graph();
    const auto orphans = relations.orphans();

    std::printf("people:        %zu\n", graph.profiles.size());
    std::printf("relationships: %zu\n", graph.asserted_edges.size());
    std::printf("hypotheses:    %zu\n", graph.hypotheses.size());

    std::size_t sensitive = 0;
    std::size_t guesses = 0;
    for (const auto& profile : graph.profiles) {
        sensitive += profile.at_or_above(Sensitivity::Sensitive).size();
        for (const auto& trait : profile.traits) {
            guesses += trait.is_hypothesis() ? 1 : 0;
        }
    }
    std::printf("sensitive traits: %zu\n", sensitive);
    std::printf("unconfirmed traits: %zu\n", guesses);

    if (orphans.empty()) {
        std::puts("\nevery person has a path back to the owner.");
        return kOk;
    }
    std::printf("\n%zu with no path to the owner:\n", orphans.size());
    for (const auto& id : orphans) {
        std::printf("  %s\n", id.c_str());
    }
    std::puts(
        "\nA record with no chain back to the owner has no reason to exist. This is\n"
        "reported rather than deleted, because it is evidence of how it got there.");
    return kOk;
}

}  // namespace

int main(int argc, char** argv) {
    std::vector<std::string> args(argv + 1, argv + argc);
    bool reveal = false;
    std::erase_if(args, [&reveal](const std::string& a) {
        if (a == "--all") {
            reveal = true;
            return true;
        }
        return false;
    });

    if (args.empty() || args[0] == "-h" || args[0] == "--help") {
        print_usage();
        return args.empty() ? kUsage : kOk;
    }

    const GraphStore store{graph_path()};
    const std::string& command = args[0];

    // First creation participates in the same OS-owned transaction as every update.
    // Directory creation is idempotent; checking for an existing graph happens ONLY
    // after locking, so two first-run processes cannot overwrite each other's owner.
    if (command == "init" && !store.path().parent_path().empty()) {
        std::error_code ec;
        std::filesystem::create_directories(store.path().parent_path(), ec);
        if (ec) { return fail("graph directory could not be created", kStoreProblem); }
    }
    auto transaction = GraphTransaction::acquire(store.path());
    if (!transaction) {
        return fail(describe(transaction.error()), kStoreProblem);
    }

    if (command == "init") {
        if (args.size() < 2) {
            return fail("init needs the owner's identifier", kUsage);
        }
        std::error_code ec;
        const bool exists = std::filesystem::exists(store.path(), ec);
        if (ec || exists) {
            return fail("init requires a new graph; the existing graph is untouched", kStoreProblem);
        }
        PersonGraph graph;
        graph.owner_id = args[1];
        Profile owner;
        owner.subject.subject_id = args[1];
        owner.subject.kind = SubjectKind::Owner;
        owner.subject.display_label = args.size() > 2 ? args[2] : args[1];
        graph.profiles.push_back(std::move(owner));

        if (!save_or_report(store, graph)) {
            return kStoreProblem;
        }
        std::printf("graph created at %s, rooted at %s\n", store.path().string().c_str(),
                    graph.owner_id.c_str());
        return kOk;
    }

    auto graph = load_or_report(store);
    if (!graph) {
        return kStoreProblem;
    }

    if (command == "export-json") {
        if (args.size() != 1) { return fail("export-json accepts only --all", kUsage); }
        const auto json = project_wanda_json(*graph, reveal);
        if (!json) { return fail(describe(json.error()), kStoreProblem); }
        // Validate/construct the complete bounded document before writing stdout.
        if (std::fwrite(json->data(), 1, json->size(), stdout) != json->size() ||
            std::fputc('\n', stdout) == EOF || std::fflush(stdout) != 0) {
            return fail("graph projection could not be written", kStoreProblem);
        }
        return kOk;
    }

    if (command == "show") {
        return cmd_show(*graph, reveal);
    }
    if (command == "check") {
        return cmd_check(*graph);
    }

    if (command == "person") {
        if (args.size() < 2) {
            return fail("person needs an identifier", kUsage);
        }
        const auto* profile = graph->find(args[1]);
        if (!profile) {
            return fail("no such person in the graph", kUsage);
        }
        print_profile(*profile, reveal);
        const auto relations = graph->to_relation_graph();
        if (const auto path = relations.path_to_owner(args[1])) {
            std::printf("  in the graph because: ");
            for (std::size_t i = 0; i < path->size(); ++i) {
                std::printf("%s%s", (*path)[i].c_str(), i + 1 < path->size() ? " -> " : "\n");
            }
        } else {
            std::puts("  no path to the owner — this record has no justification.");
        }
        return kOk;
    }

    if (command == "mutual") {
        if (args.size() < 3) {
            return fail("mutual needs two identifiers", kUsage);
        }
        const auto shared = graph->to_relation_graph().mutual(args[1], args[2]);
        if (shared.empty()) {
            std::puts("no one in common.");
            return kOk;
        }
        for (const auto& id : shared) {
            std::printf("%s\n", id.c_str());
        }
        return kOk;
    }

    if (command == "path") {
        if (args.size() < 2) {
            return fail("path needs an identifier", kUsage);
        }
        const auto found = graph->to_relation_graph().path_to_owner(args[1]);
        if (!found) {
            return fail("no path to the owner", kRefused);
        }
        for (std::size_t i = 0; i < found->size(); ++i) {
            std::printf("%s%s", (*found)[i].c_str(), i + 1 < found->size() ? " -> " : "\n");
        }
        return kOk;
    }

    if (command == "add-person") {
        if (args.size() < 3) {
            return fail("add-person needs an identifier and a kind", kUsage);
        }
        const auto kind = parse_enum(args[2], kSubjectKinds);
        if (!kind) {
            return fail("unknown kind; try owner, consented_person or public_figure", kUsage);
        }
        if (graph->find(args[1])) {
            return fail("that person is already in the graph", kUsage);
        }
        Profile profile;
        profile.subject.subject_id = args[1];
        profile.subject.kind = *kind;
        profile.subject.display_label = args.size() > 3 ? args[3] : args[1];
        graph->profiles.push_back(std::move(profile));
        if (!save_or_report(store, *graph)) {
            return kStoreProblem;
        }
        std::printf("added %s\n", args[1].c_str());
        return kOk;
    }

    if (command == "set") {
        if (args.size() < 5) {
            return fail("set needs a subject, key, value and basis", kUsage);
        }
        auto* profile = graph->find_mutable(args[1]);
        if (!profile) {
            return fail("no such person in the graph", kUsage);
        }
        const auto basis = parse_enum(args[4], kTraitBases);
        if (!basis) {
            return fail("unknown basis; see --help", kUsage);
        }
        Trait trait;
        trait.key = args[2];
        trait.value = args[3];
        trait.basis = *basis;
        trait.source_note = args.size() > 5 ? args[5] : std::string{};

        // The same refusal the store applies on load. Something refused here could not
        // survive a restart anyway, and saying so now is better than writing a record that
        // quietly disappears.
        if (const auto refusal = refuse_trait(profile->subject, trait)) {
            return fail(describe(*refusal), kRefused);
        }
        std::printf("%s: %s is %s\n", args[1].c_str(), trait.key.c_str(),
                    std::string{name(trait.effective_sensitivity())}.c_str());
        profile->traits.push_back(std::move(trait));
        return save_or_report(store, *graph) ? kOk : kStoreProblem;
    }

    if (command == "ingest") {
        if (args.size() < 3) {
            return fail("ingest needs a file and the name you appear under in it", kUsage);
        }
        bool apply = false;
        std::vector<std::string> rest(args.begin() + 1, args.end());
        std::erase_if(rest, [&apply](const std::string& a) {
            if (a == "--apply") {
                apply = true;
                return true;
            }
            return false;
        });
        if (rest.size() < 2) {
            return fail("ingest needs a file and the name you appear under in it", kUsage);
        }

        std::ifstream in(rest[0], std::ios::binary);
        if (!in) {
            return fail("cannot read that file", kStoreProblem);
        }
        const std::string text((std::istreambuf_iterator<char>(in)),
                               std::istreambuf_iterator<char>());

        const auto parsed = sentry::ingest::parse_export(text);
        if (parsed.participants.empty()) {
            std::printf("no conversation found: %zu lines matched nothing.\n",
                        parsed.unparsed_lines);
            return kUsage;
        }
        std::printf("%zu people, %zu system messages, dates read %s\n",
                    parsed.participants.size(), parsed.system_messages,
                    std::string{sentry::ingest::name(parsed.order)}.c_str());
        if (parsed.order == sentry::ingest::DateOrder::Ambiguous) {
            // Said plainly rather than hidden: the owner may know the answer, and a
            // silently year-only date looks like a bug rather than a refusal to guess.
            std::puts("  the day/month order could not be determined from this file, so "
                      "only years are recorded.");
        }

        const auto label = rest.size() > 2 ? rest[2] : rest[0];
        const auto suggested =
            sentry::ingest::suggest(parsed, graph->owner_id, {rest[1]}, label);

        if (!suggested.note.empty()) {
            std::printf("\n%s\n", suggested.note.c_str());
        }
        std::puts("");
        for (const auto& profile : suggested.profiles) {
            const bool known = graph->find(profile.subject.subject_id) != nullptr;
            std::printf("  %s%s\n", profile.subject.display_label.c_str(),
                        known ? " (already in the graph)" : "");
            for (const auto& trait : profile.traits) {
                std::printf("      %-20s %s\n", trait.key.c_str(), trait.value.c_str());
            }
        }
        for (const auto& edge : suggested.hypotheses) {
            std::printf("  hypothesis: %s -> %s\n", edge.from_id.c_str(),
                        edge.to_id.c_str());
        }

        if (!apply) {
            std::puts("\nNothing written. Re-run with --apply to record it.");
            return kOk;
        }

        std::size_t added = 0;
        for (const auto& profile : suggested.profiles) {
            if (auto* existing = graph->find_mutable(profile.subject.subject_id)) {
                // Traits append rather than replace. A second export of the same chat is a
                // later reading, and the first one is not made wrong by it.
                for (const auto& trait : profile.traits) {
                    if (!refuse_trait(existing->subject, trait)) {
                        existing->traits.push_back(trait);
                    }
                }
                continue;
            }
            graph->profiles.push_back(profile);
            ++added;
        }
        for (const auto& edge : suggested.hypotheses) {
            graph->hypotheses.push_back(edge);
        }
        if (!save_or_report(store, *graph)) {
            return kStoreProblem;
        }
        std::printf("\n%zu new people, %zu hypotheses. None of it is asserted: run "
                    "`add-edge` to confirm a relationship.\n",
                    added, suggested.hypotheses.size());
        return kOk;
    }

    if (command == "add-edge") {
        if (args.size() < 5) {
            return fail("add-edge needs two people, a kind and a basis", kUsage);
        }
        const auto kind = parse_enum(args[3], kRelationKinds);
        const auto basis = parse_enum(args[4], kEdgeBases);
        if (!kind || !basis) {
            return fail("unknown relation kind or basis; see --help", kUsage);
        }
        const auto* from = graph->find(args[1]);
        const auto* to = graph->find(args[2]);
        if (!from || !to) {
            return fail("both people must already be in the graph", kUsage);
        }

        RelationEdge edge;
        edge.from_id = args[1];
        edge.to_id = args[2];
        edge.kind = *kind;
        edge.basis = *basis;
        edge.declarer_id = args.size() > 5 ? args[5] : std::string{};
        edge.source_note = args.size() > 6 ? args[6] : std::string{};

        if (*basis == EdgeBasis::ModelHypothesis) {
            graph->hypotheses.push_back(edge);
            std::puts("recorded as a hypothesis: readable, and outside the asserted graph.");
            return save_or_report(store, *graph) ? kOk : kStoreProblem;
        }
        if (const auto refusal =
                refuse_edge(edge, from->subject, to->subject, edge.declarer_id)) {
            return fail(describe(*refusal), kRefused);
        }
        graph->asserted_edges.push_back(std::move(edge));
        std::printf("%s -> %s\n", args[1].c_str(), args[2].c_str());
        return save_or_report(store, *graph) ? kOk : kStoreProblem;
    }

    print_usage();
    return fail("unknown command", kUsage);
}
