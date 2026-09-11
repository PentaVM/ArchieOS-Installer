#pragma once

// Where a node goes, and why it is there.
//
// A layout can be arbitrary or it can mean something, and the difference is whether an
// operator learns anything from where things sit. Hashing an id into a position gives a
// scene that is stable and completely uninformative: two modules next to each other are
// neighbours because of their spelling. It looks like structure and carries none, which
// is worse than an obvious grid, because it invites the viewer to read a relationship
// that is not there.
//
// So position is derived from facts the node actually has.
//
// **Radius comes from dependency depth.** A module that depends on nothing sits in the
// innermost shell; one built on top of three layers of others sits three shells out.
// The scene then reads outward: foundations at the centre, the things standing on them
// around the edge. That is a real property of the registry, and it changes only when
// the registry changes.
//
// **Angle comes from the node's layout slot, through the golden angle.** This is the
// phyllotaxis trick, and it is the right one here for a reason that has nothing to do
// with looking organic: it distributes any number of points evenly *without knowing how
// many there will be*. A node keeps its slot for its whole life, so it keeps its angle
// for its whole life — new arrivals fill the gaps between existing nodes instead of
// pushing them around. Sorting nodes by name into a ring would look tidier and would
// move half the scene every time a module called "a.something" was installed.
//
// **Height comes from severity.** Anything wrong lifts out of the plane, so a problem is
// visible from any camera angle rather than only from the one that happens to be facing
// it.
//
// Nothing here is random, nothing is hashed, and the same node in the same state is
// always in the same place.

#include <cstdint>
#include <map>
#include <string>
#include <vector>

#include "archie/visual/camera.hpp"
#include "archie/visual/geo.hpp"
#include "archie/visual/graph.hpp"

namespace archie::visual {

struct LayoutPolicy {
    /// Where the globe is and how markers stand off it. Used for nodes that carry a
    /// real place; ignored by everything else.
    GlobePolicy globe{};

    /// Radius of the innermost shell, where nodes that depend on nothing live.
    float base_radius = 6.0F;
    /// Added radius per dependency level.
    float shell_spacing = 7.0F;
    /// 137.50776 degrees. The angle between successive seeds in a sunflower head, and
    /// the only rotation that keeps every prefix of the sequence evenly spread.
    float golden_angle_degrees = 137.50776F;
    /// Vertical lift per severity level.
    float height_per_severity = 1.6F;
    /// A gentle vertical spread within a shell so that nodes at the same radius and
    /// severity do not land in one flat ring. Derived from the slot, so it is as stable
    /// as the angle is.
    float slot_height_jitter = 0.35F;
};

/// Dependency depth for every node, and which of them are tangled.
struct DepthReport {
    std::map<std::string, int> depth;

    /// Nodes participating in a dependency cycle.
    ///
    /// A cycle is a bug in the module declarations, not in the display, and the display
    /// must neither hang on it nor hide it. Each member gets a finite depth so the scene
    /// still draws, and the cycle is reported so it can be shown as what it is.
    std::vector<std::string> in_cycle;
};

/// Longest dependency chain beneath each node.
///
/// Edges to nodes that are not in the scene are ignored: a dependency on something the
/// display cannot see cannot contribute a level the operator could interpret.
[[nodiscard]] DepthReport compute_depths(const std::map<std::string, SceneNode>& nodes);

/// Place one node. Pure: the same inputs always give the same point.
[[nodiscard]] Vec3 place(int slot, int depth, int severity, const LayoutPolicy& policy) noexcept;

/// Place every node in a scene.
[[nodiscard]] std::map<std::string, Vec3> lay_out(const std::map<std::string, SceneNode>& nodes,
                                                  const LayoutPolicy& policy);

/// Place every node using depths already computed.
///
/// The overload exists because a caller that needs the cycle report needs the depths
/// anyway, and walking the dependency graph twice for one frame is the kind of waste
/// that is invisible until someone measures it.
[[nodiscard]] std::map<std::string, Vec3> lay_out(const std::map<std::string, SceneNode>& nodes,
                                                  const DepthReport& depths,
                                                  const LayoutPolicy& policy);

}  // namespace archie::visual
