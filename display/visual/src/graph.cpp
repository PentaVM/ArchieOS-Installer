#include "archie/visual/graph.hpp"

#include <algorithm>
#include <cmath>
#include <set>

namespace archie::visual {
namespace {

/// Magnitude is a float coming from a provider, and providers jitter in the low
/// decimals. Comparing exactly would bump a node's revision on most polls, which would
/// make the renderer restart every transition forever — the precise failure this class
/// exists to prevent. The tolerance is far below anything a viewer could perceive as a
/// change in size.
constexpr float kMagnitudeEpsilon = 1e-4F;

}  // namespace

bool NodeSpec::visually_equivalent(const NodeSpec& other) const noexcept {
    if (location.has_value() != other.location.has_value()) {
        return false;
    }
    if (location.has_value() && !location->same_place_as(*other.location)) {
        return false;
    }
    return kind == other.kind && label == other.label && severity == other.severity &&
           freshness == other.freshness && authoritative == other.authoritative &&
           version == other.version && explanation == other.explanation &&
           depends_on == other.depends_on &&
           std::fabs(magnitude - other.magnitude) <= kMagnitudeEpsilon;
}

const SceneNode* SceneGraph::find(const std::string& id) const noexcept {
    const auto it = m_nodes.find(id);
    return it == m_nodes.end() ? nullptr : &it->second;
}

int SceneGraph::take_slot() {
    for (std::size_t i = 0; i < m_slot_used.size(); ++i) {
        if (!m_slot_used[i]) {
            m_slot_used[i] = true;
            return static_cast<int>(i);
        }
    }
    m_slot_used.push_back(true);
    return static_cast<int>(m_slot_used.size()) - 1;
}

void SceneGraph::release_slot(int slot) {
    if (slot >= 0 && static_cast<std::size_t>(slot) < m_slot_used.size()) {
        m_slot_used[static_cast<std::size_t>(slot)] = false;
    }
}

GraphDelta SceneGraph::apply(const std::vector<NodeSpec>& specs, std::int64_t now_ms) {
    GraphDelta delta{};
    std::set<std::string> seen;

    for (const NodeSpec& spec : specs) {
        if (spec.id.empty()) {
            // A node with no identity cannot be tracked, animated or inspected, and
            // inventing an id would make it a brand new node on every single poll.
            ++delta.anonymous_dropped;
            continue;
        }
        if (!seen.insert(spec.id).second) {
            // First occurrence wins. Two nodes sharing an identity would put the same
            // thing on screen twice and make "which one did I click" unanswerable.
            delta.duplicate_ids.push_back(spec.id);
            continue;
        }

        const auto it = m_nodes.find(spec.id);
        if (it == m_nodes.end()) {
            SceneNode node{};
            node.spec = spec;
            node.slot = take_slot();
            node.presence = NodePresence::Entering;
            node.first_seen_ms = now_ms;
            node.last_seen_ms = now_ms;
            node.revision = ++m_revision_counter;
            m_nodes.emplace(spec.id, std::move(node));
            ++m_generation;
            delta.added.push_back(spec.id);
            continue;
        }

        SceneNode& node = it->second;
        const bool was_departing = node.presence == NodePresence::Departing;

        node.last_seen_ms = now_ms;
        if (!node.spec.visually_equivalent(spec)) {
            NodeChange change{};
            change.id = spec.id;
            change.severity_from = node.spec.severity;
            change.severity_to = spec.severity;
            change.freshness_from = node.spec.freshness;
            change.freshness_to = spec.freshness;

            node.spec = spec;
            node.revision = ++m_revision_counter;
            ++m_generation;
            delta.updated.push_back(std::move(change));
        }

        if (was_departing) {
            // The same node, not a new one. It keeps its slot, its first_seen and its
            // revision, so the renderer resumes it rather than replaying an entry for
            // something that never actually left.
            node.presence = NodePresence::Present;
            ++m_generation;
            delta.returned.push_back(spec.id);
        }
        // An `Entering` node is deliberately left alone here. How long an arrival lasts
        // is a property of the animation, not of when the next payload happens to
        // arrive, and `collect` settles it on the clock.
    }

    for (auto& [id, node] : m_nodes) {
        if (seen.count(id) != 0) {
            continue;
        }
        if (node.presence != NodePresence::Departing) {
            node.presence = NodePresence::Departing;
            ++m_generation;
            delta.departing.push_back(id);
        }
    }

    return delta;
}

GraphDelta SceneGraph::collect(std::int64_t now_ms) {
    GraphDelta delta{};

    for (auto it = m_nodes.begin(); it != m_nodes.end();) {
        SceneNode& node = it->second;

        if (node.presence == NodePresence::Entering) {
            // The entry has had its time. A presence change only, so it produces no
            // delta entry: nothing about the scene is wrong, the node has simply
            // finished arriving.
            if (now_ms >= node.first_seen_ms &&
                now_ms - node.first_seen_ms >= m_policy.entry_duration_ms) {
                node.presence = NodePresence::Present;
                ++m_generation;
            }
            ++it;
            continue;
        }

        if (node.presence != NodePresence::Departing) {
            ++it;
            continue;
        }

        if (now_ms < node.last_seen_ms) {
            // The clock went backwards. Elapsed time cannot be computed, and the wrong
            // answer here is to forget a node because of arithmetic on a bad clock, or
            // to keep it forever because its last sighting is now in the future.
            // Restart the grace from here instead.
            node.last_seen_ms = now_ms;
            ++it;
            continue;
        }

        if (now_ms - node.last_seen_ms >= m_policy.departure_grace_ms) {
            release_slot(node.slot);
            ++m_generation;
            delta.removed.push_back(it->first);
            it = m_nodes.erase(it);
        } else {
            ++it;
        }
    }

    return delta;
}

std::vector<std::string> SceneGraph::dependents_of(const std::string& id) const {
    std::vector<std::string> dependents;
    for (const auto& [candidate, node] : m_nodes) {
        if (candidate == id) {
            continue;
        }
        for (const std::string& dependency : node.spec.depends_on) {
            if (dependency == id) {
                dependents.push_back(candidate);
                break;
            }
        }
    }
    return dependents;
}

std::vector<std::pair<std::string, std::string>> SceneGraph::edges() const {
    std::vector<std::pair<std::string, std::string>> result;
    for (const auto& [id, node] : m_nodes) {
        for (const std::string& dependency : node.spec.depends_on) {
            if (dependency == id) {
                // A self-edge is a declaration bug upstream, and drawing it as a loop
                // would present it as a relationship rather than a mistake.
                continue;
            }
            if (m_nodes.count(dependency) != 0) {
                result.emplace_back(id, dependency);
            }
        }
    }
    return result;
}

}  // namespace archie::visual
