#include "archie/visual/scene.hpp"

namespace archie::visual {

float ModeDirector::progress() const noexcept {
    if (!morphing() || m_policy.morph_ms <= 0) {
        return 1.0F;
    }
    return static_cast<float>(m_elapsed_ms) / static_cast<float>(m_policy.morph_ms);
}

void ModeDirector::start(SceneMode from, SceneMode to, std::int64_t elapsed_ms) {
    m_from = from;
    m_to = to;
    m_elapsed_ms = elapsed_ms;

    if (from == to || m_policy.morph_ms <= 0) {
        // A zero-length morph is a cut. Settle rather than dividing by it.
        m_from = to;
        m_elapsed_ms = 0;
    }
}

void ModeDirector::request(SceneMode mode) {
    if (!morphing()) {
        if (mode == m_from) {
            // Already there. Re-entering the mode you are in would replay an arrival for
            // a view that never went anywhere.
            return;
        }
        start(m_from, mode, 0);
        return;
    }

    if (mode == m_to) {
        // Already on the way. Restarting the morph here would make a second keypress
        // undo the progress of the first, so the harder the operator asks the slower
        // they arrive.
        m_queued.reset();
        return;
    }

    if (mode == m_from) {
        // Turning back. Reversing in place is both shorter and truthful: the scene
        // retraces the path it is already on, rather than completing a journey nobody
        // wants any more and then making a second one back.
        const std::int64_t remaining = m_policy.morph_ms - m_elapsed_ms;
        m_queued.reset();
        start(m_to, m_from, remaining < 0 ? 0 : remaining);
        return;
    }

    // Somewhere else entirely. Queue it, replacing whatever was queued: an operator
    // pressing four keys in a second wants the fourth view, not a tour of all of them.
    m_queued = mode;
}

void ModeDirector::advance(std::int64_t delta_ms) {
    if (delta_ms <= 0 || !morphing()) {
        return;
    }

    m_elapsed_ms += delta_ms;
    if (m_elapsed_ms < m_policy.morph_ms) {
        return;
    }

    // Arrived. `m_elapsed_ms` is deliberately left where it is: `start` is the only
    // place it is ever assigned and the only place it is meaningful, and `progress`
    // reports a settled view as complete without consulting it.
    m_from = m_to;

    if (m_queued.has_value()) {
        const SceneMode next = *m_queued;
        m_queued.reset();
        // No guard against `next == m_from` here: it cannot happen — queueing requires a
        // mode different from the one in flight, and turning back clears the queue — and
        // `start` collapses a morph to the mode already showing in any case.
        start(m_from, next, 0);
    }
}

}  // namespace archie::visual
