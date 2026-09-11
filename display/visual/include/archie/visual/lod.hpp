#pragma once

// How much of the scene is drawn, and which parts of it.
//
// Two different questions live here and they are easy to confuse.
//
// The first is *how* to draw a node that is already on screen: a node forty metres away
// does not need the geometry, label and status ring it gets when the camera is on top
// of it. That is semantic zoom, and it is a pure function of distance.
//
// The second is *whether* to draw it at all. The render budget caps how many nodes may
// be submitted, and a busy node has more state than that cap allows, so something has
// to be left out. The naive answer — take the top N by importance each frame — produces
// the single most recognisable symptom of a status display built in a hurry: nodes
// blinking in and out along the cutoff, several times a second, drawing the eye to
// whichever node happens to be flickering rather than to whichever one matters.
//
// So incumbency counts. A node already on screen has to be beaten by a clear margin
// before it is displaced, which costs a little accuracy at the boundary and buys a
// scene that holds still. Two things override that: the node the operator is inspecting
// is never evicted for any reason, and nothing is ever silently dropped — whatever does
// not fit is counted, so the scene can say "and 47 more" rather than quietly lying
// about how much is there.

#include <cstdint>
#include <set>
#include <string>
#include <vector>

#include "archie/visual/freshness.hpp"

namespace archie::visual {

/// How much of a node is drawn. Semantic, not a polygon count: each step removes a
/// *kind* of information, so the operator loses detail in a predictable order.
enum class DetailLevel {
    /// A shape and a colour. Position and severity survive; nothing else does.
    Silhouette,
    /// Shape, colour, and the status ring. Readable at a glance, no text.
    Simplified,
    /// Everything a node normally shows, including its label.
    Full,
    /// Full, plus the detail only worth drawing for something being looked at directly:
    /// per-field values, edge highlighting, provenance.
    Inspect,
};

struct LodPolicy {
    /// Distances at which a node drops to the next level down.
    float inspect_within = 6.0F;
    float full_within = 30.0F;
    float simplified_within = 90.0F;

    /// How much better a challenger must be to displace a node already on screen,
    /// measured in severity levels.
    ///
    /// Half a level means a node has to be genuinely more important, not merely a
    /// rounding error more important. Zero would restore the flicker this exists to
    /// prevent; one whole level would let a node that is now clearly worse hold its
    /// place against something the operator needs to see.
    double incumbency_margin = 0.5;
};

/// What the scene knows about a node when deciding whether to draw it.
struct LodCandidate {
    std::string id;
    int severity = 0;
    float magnitude = 0.0F;
    /// Distance from the camera, in scene units.
    float distance = 0.0F;
    Freshness freshness = Freshness::Live;
};

struct VisibleNode {
    std::string id;
    DetailLevel detail = DetailLevel::Simplified;
};

struct LodPlan {
    /// What to draw, in a deterministic order.
    std::vector<VisibleNode> visible;

    /// How many candidates did not fit. Never silently zero: the scene is expected to
    /// surface this, because a display that shows 400 of 447 nodes without saying so is
    /// telling the operator the machine is quieter than it is.
    int aggregated = 0;

    [[nodiscard]] std::size_t drawn() const noexcept { return visible.size(); }
};

/// Semantic zoom for one node, independent of whether it made the cut.
[[nodiscard]] DetailLevel detail_for(float distance, bool focused, const LodPolicy& policy) noexcept;

/// Chooses what to draw, and remembers what it chose.
///
/// Stateful on purpose: the previous frame's visible set is the whole basis of the
/// stability guarantee, and a planner that forgot it between frames could not provide
/// one.
class LodPlanner final {
public:
    LodPlanner() = default;
    explicit LodPlanner(LodPolicy policy) : m_policy(policy) {}

    /// Plan one frame.
    ///
    /// `budget` is the render tier's cap on visible nodes. `focused` is the node the
    /// operator is inspecting, or empty.
    LodPlan plan(const std::vector<LodCandidate>& candidates, int budget,
                 const std::string& focused);

    [[nodiscard]] const std::set<std::string>& visible_set() const noexcept { return m_visible; }

    /// Forget the previous frame. Used when the scene changes wholesale — a mode
    /// change — where carrying incumbency across would privilege nodes that were
    /// important in a view the operator has left.
    void reset() noexcept { m_visible.clear(); }

private:
    LodPolicy m_policy{};
    std::set<std::string> m_visible{};
};

}  // namespace archie::visual
