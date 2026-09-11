#pragma once

// Which view the observatory is showing, and how it changes.
//
// The four modes are four readings of the same node, not four applications. Each is
// backed by a projection that already exists, and none of them invents a subject:
//
//  - **Cerebra** — the whole node as one field: modules, models, peers and tasks
//    together. The default, because it is the only view that answers "what is this
//    machine doing" without the operator first knowing where to look.
//  - **Brain** — the cognition subsystem alone, read from the same module and model
//    lists. A closer look at a part of Cerebra, not a different source of truth.
//  - **Watcher** — the world: observations and the providers that supplied them.
//  - **Fabric** — the other nodes, and the transport that reaches them.
//
// Two rules govern movement between them, and both exist to keep the operator oriented.
//
// A mode change is *always* the operator's. Nothing in this file switches modes on an
// alarm, a threshold or an arriving observation. A display that reorganises itself
// because something happened elsewhere takes the screen away from whoever was reading
// it, at the exact moment they were most likely reading it for a reason — and it
// teaches them that what they are looking at is not their choice.
//
// And a change already under way is never abandoned mid-air. Requests that arrive
// during a morph queue, and only the most recent one survives, so an operator pressing
// four keys in a second lands on the fourth view rather than travelling through all of
// them. The exception is turning back: asking for the mode being left reverses the
// morph in place, because the shortest way back to where you were is the way you came.

#include <cstdint>
#include <optional>

#include "archie/visual/graph.hpp"

namespace archie::visual {

enum class SceneMode {
    Cerebra,
    Brain,
    Watcher,
    Fabric,
};

/// Whether a node of this kind belongs in this view.
///
/// Until this existed the four modes differed only in where the camera sat, which made
/// them four angles on one scene rather than four readings of it. Cerebra is the whole
/// node, so it shows everything including kinds this build cannot classify — it is the
/// view of record, and a thing missing from it is missing from the observatory. The
/// other three are each about one part, and showing the rest alongside would drown the
/// part they exist to isolate.
[[nodiscard]] constexpr bool visible_in(SceneMode mode, NodeKind kind) noexcept {
    switch (mode) {
        case SceneMode::Cerebra:
            return true;
        case SceneMode::Brain:
            switch (kind) {
                case NodeKind::Module:
                case NodeKind::Model:
                case NodeKind::Task:
                    return true;
                case NodeKind::Peer:
                case NodeKind::Observation:
                case NodeKind::Sensor:
                case NodeKind::Unknown:
                    return false;
            }
            return false;
        case SceneMode::Watcher:
            switch (kind) {
                case NodeKind::Observation:
                case NodeKind::Sensor:
                    return true;
                case NodeKind::Module:
                case NodeKind::Model:
                case NodeKind::Task:
                case NodeKind::Peer:
                case NodeKind::Unknown:
                    return false;
            }
            return false;
        case SceneMode::Fabric:
            switch (kind) {
                case NodeKind::Peer:
                    return true;
                case NodeKind::Module:
                case NodeKind::Model:
                case NodeKind::Task:
                case NodeKind::Observation:
                case NodeKind::Sensor:
                case NodeKind::Unknown:
                    return false;
            }
            return false;
    }
    return false;
}

struct ModePolicy {
    /// How long a morph between two modes takes.
    std::int64_t morph_ms = 1200;
};

/// Which mode is showing, and what is happening between modes.
///
/// Clock-free: `advance` takes a delta.
class ModeDirector final {
public:
    ModeDirector() = default;
    explicit ModeDirector(ModePolicy policy) : m_policy(policy) {}

    /// The mode being shown, or being left during a morph.
    [[nodiscard]] SceneMode current() const noexcept { return m_from; }

    /// The mode being moved to. Equal to `current` when settled.
    [[nodiscard]] SceneMode target() const noexcept { return m_to; }

    [[nodiscard]] bool morphing() const noexcept { return m_from != m_to; }

    /// How far through the morph, in [0, 1]. 1 when settled.
    [[nodiscard]] float progress() const noexcept;

    /// The request waiting for the current morph to finish, if any.
    [[nodiscard]] std::optional<SceneMode> queued() const noexcept { return m_queued; }

    /// Ask for a mode. Only ever called in response to the operator.
    void request(SceneMode mode);

    /// Advance the morph. Negative deltas are ignored rather than rewinding.
    void advance(std::int64_t delta_ms);

private:
    void start(SceneMode from, SceneMode to, std::int64_t elapsed_ms);

    ModePolicy m_policy{};
    SceneMode m_from = SceneMode::Cerebra;
    SceneMode m_to = SceneMode::Cerebra;
    std::optional<SceneMode> m_queued{};
    std::int64_t m_elapsed_ms = 0;
};

}  // namespace archie::visual
