// Durable storage for the person graph.
//
// RFC: docs/RFC-008-SENTRY-WORLD-MODEL.md §Identity
//
// Everything in `sentry/identity/` is pure: it decides, it holds nothing. That is the right
// shape for rules and the wrong shape for a graph the owner spends months building, so the
// store lives here, in its own library, and is the only part of Sentry that touches a disk.
//
// # The load path is a trust boundary
//
// The obvious design validates on write and trusts the file on read. It is wrong. A file
// is edited by hand, restored from a backup taken before a rule existed, written by an
// older build, or synchronised from another machine — and in every one of those cases a
// record that `refuse_edge` would reject today is sitting in it.
//
// So `load()` re-runs every refusal. A record that would not be accepted now is not
// accepted now, whatever a previous version of this program thought. It is reported rather
// than dropped in silence, because a graph that quietly shrinks between restarts is the
// kind of bug nobody finds.
//
// # One bad line does not cost the graph
//
// A truncated write, a disk error, an editor that mangled a character: the affected line is
// reported with its number and everything else still loads. Refusing the whole file would
// turn one corrupt byte into the loss of the entire relationship history, which is a worse
// outcome than a graph with a hole and a loud message about it.
//
// # Format
//
// Line-oriented, tab-separated, one record per line, with `\\`, `\t` and `\n` escaped in
// every field. Deliberately not JSON: Sentry has no third-party dependencies on purpose,
// and a hand-rolled JSON parser is a far larger surface for bugs than a hand-rolled
// splitter over a format with exactly three escapes.

#pragma once

#include <cstddef>
#include <expected>
#include <filesystem>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

#include "sentry/identity/profile.hpp"
#include "sentry/identity/relationship.hpp"
#include "sentry/identity/subject.hpp"

namespace sentry::store {

using sentry::identity::Profile;
using sentry::identity::RelationEdge;
using sentry::identity::RelationGraph;

// The format version written into every file.
//
// Present so that a future change can be detected rather than misread. A file whose version
// this build does not know is refused outright — unlike a single bad line, an unknown
// format means every line might mean something other than what it appears to.
inline constexpr int kFormatVersion = 1;

// Everything the graph holds, as one value.
struct PersonGraph {
    std::string owner_id{};
    std::vector<Profile> profiles{};
    std::vector<RelationEdge> asserted_edges{};
    std::vector<RelationEdge> hypotheses{};

    // Build the traversable graph. Hypotheses stay out of the asserted edges, exactly as
    // `RelationGraph` requires.
    [[nodiscard]] RelationGraph to_relation_graph() const;

    [[nodiscard]] const Profile* find(std::string_view subject_id) const;

    // The same lookup for a caller that is about to edit. Present so that editing a profile
    // does not require casting away constness, which is the kind of thing that stops being
    // obviously safe the moment someone moves the code.
    [[nodiscard]] Profile* find_mutable(std::string_view subject_id);
};

// Why one record was not loaded.
enum class RejectionKind {
    // The line did not parse: wrong field count, an unknown record tag, an unknown enum
    // value, or an escape sequence that does not terminate.
    Malformed,
    // The line parsed, and the record it describes is one the rules refuse today.
    RefusedByPolicy,
    // A record referring to a subject the file never declared.
    UnknownSubject,
    // A second declaration of a subject already declared.
    DuplicateSubject,
};

[[nodiscard]] std::string_view name(RejectionKind kind) noexcept;

// One record that did not make it in.
struct Rejection {
    std::size_t line = 0;
    RejectionKind kind{};
    // What was wrong, for a person reading a log. Never contains the record's values: a
    // rejection message about a home address that quotes the address has published it into
    // a log file that has none of the store's protections.
    std::string detail{};
};

// What a load produced.
struct LoadResult {
    PersonGraph graph{};
    std::vector<Rejection> rejections{};

    [[nodiscard]] bool clean() const noexcept { return rejections.empty(); }
};

// Why a whole file could not be read or written.
enum class StoreError {
    // No file at that path. Distinct from an empty one: a first run and a truncated store
    // are not the same situation.
    NotFound,
    Unreadable,
    Unwritable,
    // Missing or unparseable version header.
    MissingVersion,
    // A version this build does not understand.
    UnsupportedVersion,
    // The file declares no owner, so nothing in it has a root.
    MissingOwner,
};

[[nodiscard]] std::string_view describe(StoreError error) noexcept;

// A person graph on disk.
class GraphStore {
  public:
    explicit GraphStore(std::filesystem::path path) : path_(std::move(path)) {}

    [[nodiscard]] const std::filesystem::path& path() const noexcept { return path_; }

    // Read the graph, re-validating every record against today's rules.
    [[nodiscard]] std::expected<LoadResult, StoreError> load() const;

    // Write the graph, atomically.
    //
    // Written to a temporary in the same directory, flushed, then renamed over the target.
    // A crash leaves either the previous graph or the new one, never a half-written file —
    // and the temporary is in the same directory because a rename across filesystems is a
    // copy, which is not atomic.
    //
    // Records that today's rules would refuse are not written. Persisting something that
    // cannot be loaded back produces a store that silently loses data on every round trip.
    [[nodiscard]] std::expected<std::size_t, StoreError> save(const PersonGraph& graph) const;

  private:
    std::filesystem::path path_;
};

// Serialise to the on-disk text, without touching a disk. Exposed for testing and for
// callers that keep the graph somewhere other than a file.
[[nodiscard]] std::string serialise(const PersonGraph& graph);

// Parse the on-disk text, applying every refusal.
[[nodiscard]] std::expected<LoadResult, StoreError> parse(std::string_view text);

}  // namespace sentry::store
