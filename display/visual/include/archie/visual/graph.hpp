#pragma once

// The scene graph's memory: which thing on screen is which, and for how long.
//
// This is the load-bearing decision behind everything that looks expensive. A node can
// only glide from one place to another if the renderer knows the two frames are showing
// the *same* node; if identity is recomputed from each payload, every poll is a fresh
// scene and the only honest way to draw it is a cut. That is the difference between an
// observatory and a slideshow, and no amount of shader work fixes it.
//
// So this layer owns three things the projection cannot:
//
//  - **Identity.** A node is its id, not its position in a list. Payload ordering
//    changes nothing, because a provider reordering its results is not an event.
//  - **A seat.** Each node holds a layout slot for as long as it exists, so the scene
//    does not reshuffle around it when an unrelated node appears.
//  - **Grace.** A node missing from one poll is not gone; it is missing. Projections
//    have gaps, providers flap, and a module that restarts within three seconds should
//    not visibly die and be reborn.
//
// It holds no clock and no geometry. Where a node sits in space is the renderer's
// business; *that it is the same node* is this file's.

#include <cstdint>
#include <map>
#include <optional>
#include <string>
#include <vector>

#include "archie/visual/freshness.hpp"
#include "archie/visual/geo.hpp"

namespace archie::visual {

/// What a node stands for. Drives the shape grammar, so an operator can tell a module
/// from a peer without reading a label.
enum class NodeKind {
    /// A loaded or installed module in this node's registry.
    Module,
    /// Another ArchieOS node on the fabric.
    Peer,
    /// A registered model.
    Model,
    /// A world observation — an earthquake, a storm — from a provider.
    Observation,
    /// A camera or other sensor.
    Sensor,
    /// A task in the execution chain.
    Task,
    /// Present in the payload, but of a kind this build does not know.
    ///
    /// Deliberately drawn rather than dropped. A display built before a subsystem
    /// existed should show that something is there and admit it does not know what,
    /// which is strictly more useful than silently omitting it.
    Unknown,
};

/// The visual vocabulary. Distinct kinds must be distinguishable at a glance and
/// without colour, since colour is already carrying severity and freshness.
enum class NodeShape {
    Cube,
    Sphere,
    Octahedron,
    Ring,
    Cone,
    Capsule,
    /// Reserved for `NodeKind::Unknown`: deliberately the least specific form.
    Placeholder,
};

[[nodiscard]] constexpr NodeShape shape_of(NodeKind kind) noexcept {
    switch (kind) {
        case NodeKind::Module:
            return NodeShape::Cube;
        case NodeKind::Peer:
            return NodeShape::Octahedron;
        case NodeKind::Model:
            return NodeShape::Capsule;
        case NodeKind::Observation:
            return NodeShape::Sphere;
        case NodeKind::Sensor:
            return NodeShape::Cone;
        case NodeKind::Task:
            return NodeShape::Ring;
        case NodeKind::Unknown:
            return NodeShape::Placeholder;
    }
    return NodeShape::Placeholder;
}

/// Whether a node is arriving, settled, or on its way out.
enum class NodePresence {
    /// Seen for the first time. The renderer may play an entry.
    Entering,
    /// Settled. Steady state for almost every node almost all of the time.
    Present,
    /// Missing from the most recent payload but still within its grace period. Drawn,
    /// dimmed, not yet mourned.
    Departing,
};

/// One node as the projection describes it. Everything here comes from real state.
struct NodeSpec {
    std::string id;
    NodeKind kind = NodeKind::Unknown;
    std::string label;

    /// Drives scale. Whatever the source considers size: a quake's magnitude, a
    /// module's dependent count, a peer's core count.
    float magnitude = 0.0F;

    /// Drives colour. Higher is worse. Left to the projection to define, because what
    /// counts as severe differs per kind and the display is in no position to judge.
    int severity = 0;

    Freshness freshness = Freshness::Live;

    /// Ids this node declares a dependency on. Carried through unchanged: edges are
    /// drawn from declared relationships, never from layout proximity.
    std::vector<std::string> depends_on{};

    /// Where in the world this is, when that is a meaningful question.
    ///
    /// Present for observations and absent for everything else, and the difference
    /// decides how the node is placed: a module has no latitude, and inventing one would
    /// put it somewhere on a globe as though it meant something.
    std::optional<GeoPoint> location{};

    /// The version string the payload reported, when it has one.
    std::string version{};

    /// Why the thing is in the state it is in.
    ///
    /// The control plane populates this for every module including loaded ones, and the
    /// observatory was throwing it away. It is the difference between a node coloured
    /// amber and a node coloured amber *because it needs a capability this machine does
    /// not have* — the first is a prompt to go and find out, the second is the answer.
    std::string explanation{};

    /// Whether the source that reported this is the authority for it.
    ///
    /// A separate question from how old the reading is. Age is answered by `freshness`;
    /// this is about whether the thing that said it was entitled to. A cached batch from
    /// a fallback source is current and unauthoritative at the same time, and a display
    /// that collapses the two tells the operator the wrong thing about both.
    bool authoritative = true;

    [[nodiscard]] bool visually_equivalent(const NodeSpec& other) const noexcept;
};

/// A node as the scene knows it, including what the projection cannot say.
struct SceneNode {
    NodeSpec spec{};

    /// The node's layout seat, held for its whole life including grace. Stable slots
    /// are what stop the scene reshuffling every time an unrelated node appears.
    int slot = 0;

    NodePresence presence = NodePresence::Entering;

    std::int64_t first_seen_ms = 0;
    std::int64_t last_seen_ms = 0;

    /// Bumped only when the spec changes in a way that is visible. A revision that
    /// moved on every poll would make the renderer re-run every transition forever,
    /// which is exactly the failure this class exists to prevent.
    std::uint64_t revision = 0;
};

/// One node's visible change, with both sides of it.
///
/// Carrying the old value as well as the new is what lets the event layer derive a
/// pulse from the same comparison that detected the change, rather than running its own
/// and risking a second, disagreeing opinion about what happened.
struct NodeChange {
    std::string id;
    int severity_from = 0;
    int severity_to = 0;
    Freshness freshness_from = Freshness::Live;
    Freshness freshness_to = Freshness::Live;
};

/// What changed in one application. Empty means the scene is already correct, and the
/// renderer should do nothing at all.
struct GraphDelta {
    std::vector<std::string> added;
    std::vector<NodeChange> updated;
    std::vector<std::string> departing;
    std::vector<std::string> returned;
    std::vector<std::string> removed;

    /// Ids that appeared more than once in a single payload. The first occurrence wins;
    /// the rest are dropped rather than allowed to create a second node with the same
    /// identity, which would put two of the same thing on screen.
    std::vector<std::string> duplicate_ids;

    /// Entries with no id. Dropped: a node with no identity cannot be tracked, animated
    /// or inspected, and inventing one would make it a new node on every poll.
    int anonymous_dropped = 0;

    [[nodiscard]] bool empty() const noexcept {
        return added.empty() && updated.empty() && departing.empty() && returned.empty() &&
               removed.empty() && duplicate_ids.empty() && anonymous_dropped == 0;
    }
};

struct GraphPolicy {
    /// How long a node stays `Entering` after it is first seen.
    ///
    /// Settling on a timer rather than on the next payload, because those are different
    /// things. A node is arriving for as long as its entry takes to play; if that were
    /// tied to the next poll instead, a slow projection would hold nodes in their entry
    /// state for seconds, and a projection that stopped entirely would leave the whole
    /// scene permanently arriving.
    std::int64_t entry_duration_ms = 600;

    /// How long a node missing from the payload is kept before it is forgotten.
    ///
    /// Projections have gaps and providers flap. Without grace, a module that restarts
    /// between two polls plays a death and a birth, and an operator watching sees an
    /// event that did not happen.
    std::int64_t departure_grace_ms = 3000;
};

/// Holds node identity across refreshes.
///
/// Clock-free: every mutating call takes the current time, so a test can drive a week
/// of flapping in a millisecond, backwards if it likes.
class SceneGraph final {
public:
    SceneGraph() = default;
    explicit SceneGraph(GraphPolicy policy) : m_policy(policy) {}

    /// Reconcile the scene against a payload.
    ///
    /// Nodes present keep their slot and history; nodes absent begin their grace;
    /// nodes that return within it are the same node, not a new one.
    GraphDelta apply(const std::vector<NodeSpec>& specs, std::int64_t now_ms);

    /// Forget nodes whose grace has run out. Separate from `apply` so the renderer can
    /// finish an exit animation before the node stops existing.
    GraphDelta collect(std::int64_t now_ms);

    [[nodiscard]] const std::map<std::string, SceneNode>& nodes() const noexcept {
        return m_nodes;
    }

    /// Bumped whenever anything a consumer could notice changes: an arrival, a
    /// departure, a node settling after its entry, or a spec whose severity, freshness,
    /// dependencies or place is different.
    ///
    /// The contract is deliberately "anything noticeable" rather than the narrower
    /// "anything that moves a node". The narrow version was tried first and was a trap:
    /// presence is invisible to the layout, so a departing node bumped nothing — and a
    /// caller using this to decide whether to record a timeline entry silently stopped
    /// recording the moment anything began to leave, which is exactly the moment someone
    /// scrubs back to find. Recomputing a layout on a departure costs one walk of a
    /// graph that has just changed anyway; getting the contract subtly wrong cost a
    /// class of history nobody would have noticed was missing.
    ///
    /// It exists because layout is a pure function of the graph, so recomputing it while
    /// the graph has not changed is waste by definition — at four thousand nodes it was
    /// most of the difference between deciding a frame in 27ms and in eight.
    [[nodiscard]] std::uint64_t generation() const noexcept { return m_generation; }
    [[nodiscard]] const SceneNode* find(const std::string& id) const noexcept;
    [[nodiscard]] std::size_t size() const noexcept { return m_nodes.size(); }

    /// Which nodes declare a dependency on `id`.
    ///
    /// The payload says what each thing depends on; the question an operator asks when
    /// they click something is the other way round — what leans on this, and therefore
    /// what is affected if it is the thing that is broken.
    [[nodiscard]] std::vector<std::string> dependents_of(const std::string& id) const;

    /// Edges implied by the nodes' own declared dependencies, both ends resolved.
    ///
    /// An edge to an id that is not in the scene is not drawn: a line to nowhere claims
    /// a relationship with something the operator cannot see or inspect.
    [[nodiscard]] std::vector<std::pair<std::string, std::string>> edges() const;

private:
    int take_slot();
    void release_slot(int slot);

    GraphPolicy m_policy{};
    std::map<std::string, SceneNode> m_nodes{};
    std::vector<bool> m_slot_used{};
    std::uint64_t m_revision_counter = 0;
    std::uint64_t m_generation = 0;
};

}  // namespace archie::visual
