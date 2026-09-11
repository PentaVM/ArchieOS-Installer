#pragma once

// Turning a decoded projection into things the scene can draw.
//
// This is a trust boundary, and not because the daemon is hostile. It is a boundary
// because the two sides ship separately: an observatory built today will, sooner or
// later, be pointed at a daemon that is older, newer, or briefly broken, and what it
// does then decides whether an operator sees the truth, a gap, or a confident lie.
//
// Three rules, in order of how much damage getting them wrong does.
//
// **A missing field is not a zero.** Reading an absent severity as 0 draws a healthy
// node for something the display knows nothing about — the one failure that is worse
// than showing nothing, because it is indistinguishable from good news.
//
// **An unknown value is not an error.** A module state, or a whole node kind, that this
// build has never heard of comes from a daemon that has grown, not from a daemon that
// is broken. It is drawn, marked as unrecognised, and reported. Refusing it would mean
// every upgrade of the control plane blanks parts of the observatory until the
// observatory is rebuilt to match, which is how a display becomes something nobody
// trusts to be complete.
//
// **One bad record costs one record.** A malformed entry is dropped and counted; the
// rest of the payload still arrives. A parser that gives up on the first defect hands
// the operator an empty screen because one module in forty had a null where a string
// belonged.
//
// Everything unusable is *counted*. The report is not diagnostics — it is meant to be
// shown, so that a scene missing four of its nodes says so instead of looking complete.

#include <optional>
#include <string>
#include <string_view>
#include <vector>

#include "archie/visual/budget.hpp"
#include "archie/visual/freshness.hpp"
#include "archie/visual/graph.hpp"
#include "archie/visual/value.hpp"

namespace archie::visual {

enum class DefectKind {
    /// The payload, or a record in it, was not the shape the schema promises.
    NotAnObject,
    /// A field the display needs was absent.
    MissingField,
    /// Present, but of another type. Never coerced.
    WrongType,
    /// A record with no usable identity. Unfixable: it cannot be tracked or inspected.
    EmptyIdentity,
    /// The same key twice in one object.
    DuplicateKey,
    /// A string this build does not recognise, where an enumeration was expected.
    /// Informational, not fatal — the record is still used.
    UnknownEnum,
    /// The payload declares a schema this build predates.
    NewerSchema,
};

struct IngestDefect {
    DefectKind kind = DefectKind::NotAnObject;
    /// Where it was, in a form a person can act on: `modules[3].id`.
    std::string path;
    std::string detail;
};

/// What happened during one ingest. Meant to be displayed, not just logged.
struct IngestReport {
    std::vector<IngestDefect> defects;

    /// Records that could not be used at all. The scene should say so rather than
    /// letting an operator believe they are looking at everything.
    int dropped = 0;

    /// Records that made it through, including those with recoverable defects.
    int accepted = 0;

    /// Fields present in the payload that this build has no use for. Not a problem —
    /// it is the ordinary signature of a daemon newer than the display — but worth
    /// surfacing when someone is wondering why a field they added is not showing up.
    int unknown_fields = 0;

    /// The schema version the payload declared, if it declared one.
    std::optional<double> schema_version{};

    /// The payload comes from a daemon newer than this build understands.
    bool newer_schema = false;

    [[nodiscard]] bool clean() const noexcept { return defects.empty() && dropped == 0; }

    void note(DefectKind kind, std::string path, std::string detail);
    [[nodiscard]] int count_of(DefectKind kind) const noexcept;
};

/// Severity for a module state label, using the labels `ModuleState::label` emits.
///
/// The grading is a judgement about what an operator needs to look at, and it follows
/// the upstream enum's own documentation: a module that is deliberately off is not a
/// problem, one that cannot run on this hardware is expected on some nodes, one that is
/// blocked or missing a dependency is something a person can act on, and one that is
/// invalid is broken.
///
/// `recognised` is set false for a label this build has never seen.
[[nodiscard]] int severity_for_module_state(std::string_view label, bool& recognised) noexcept;

/// The kernel schema version this build was written against.
inline constexpr double kKnownKernelSchema = 1.0;

/// Severity for an earthquake grading, using the labels `EarthquakeSeverity` emits.
///
/// The scale runs past the palette deliberately. A Major event grades above a Strong one
/// even though both are drawn in the top colour, because severity also decides which
/// nodes survive the render budget — and when a globe holds more events than the tier
/// permits, the largest is the one that must not be the one dropped.
[[nodiscard]] int severity_for_earthquake(std::string_view label, bool& recognised) noexcept;

/// The world schema version this build was written against.
inline constexpr double kKnownWorldSchema = 1.0;

/// Observations from a `WorldSnapshot`, as nodes placed on the globe.
[[nodiscard]] std::vector<NodeSpec> ingest_world(const Value& world, Freshness freshness,
                                                 IngestReport& report);

/// Severity for a peer, from the labels `NodeHealth` emits and the reachable flag.
///
/// The upstream enum draws a distinction worth preserving exactly: `Unreachable` is
/// documented as "no recent contact", explicitly *not* the same as `Unhealthy`, because
/// we do not know that it is broken. So it grades between degraded and broken — worse
/// than a node reporting trouble it can describe, better than one we know is failing.
/// Collapsing the two would either raise a false alarm every time a link flaps or hide a
/// node that is genuinely down.
[[nodiscard]] int severity_for_peer(std::string_view health, bool reachable,
                                    bool& recognised) noexcept;

/// Severity for a model, from the labels `Availability` emits.
///
/// Graded the same way as modules and peers, and for the same reason: a model that is
/// administratively off is behaving correctly, one that is merely cold is usable and
/// simply not resident, and one that is offline is temporarily unreachable — which is a
/// statement about what we do not know rather than about what is broken.
[[nodiscard]] int severity_for_model(std::string_view availability, bool& recognised) noexcept;

/// Models from a `DisplaySnapshot`, as nodes.
[[nodiscard]] std::vector<NodeSpec> ingest_models(const Value& snapshot, Freshness freshness,
                                                  IngestReport& report);

/// Peers from a `DisplaySnapshot`, as nodes.
[[nodiscard]] std::vector<NodeSpec> ingest_peers(const Value& snapshot, Freshness freshness,
                                                 IngestReport& report);

/// Modules from a `DisplaySnapshot`, as nodes.
[[nodiscard]] std::vector<NodeSpec> ingest_modules(const Value& snapshot, Freshness freshness,
                                                   IngestReport& report);

/// Kernel telemetry from a `KernelSnapshot`.
///
/// `freshness` is a parameter rather than something derived here because this file has
/// no clock, and because a reading's age is the one thing that cannot be recovered from
/// the payload alone. Making it explicit means it cannot be forgotten.
[[nodiscard]] KernelReading ingest_kernel(const Value& kernel, Freshness freshness,
                                          bool display_present, IngestReport& report);

}  // namespace archie::visual
