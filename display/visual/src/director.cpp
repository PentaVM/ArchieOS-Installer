#include "archie/visual/director.hpp"

#include <algorithm>
#include <cmath>
#include <limits>

#include "archie/visual/camera.hpp"

namespace archie::visual {

namespace {

constexpr std::size_t kMaxRememberedPresses = 32;

[[nodiscard]] float out_cubic(float t) noexcept {
    const float c = t < 0.0F ? 0.0F : (t > 1.0F ? 1.0F : t);
    const float inv = 1.0F - c;
    return 1.0F - inv * inv * inv;
}

[[nodiscard]] CaptionVerb verb_of(StageAction action) noexcept {
    switch (action) {
        case StageAction::Move:
            return CaptionVerb::Moving;
        case StageAction::Click:
            return CaptionVerb::Clicking;
        case StageAction::DoubleClick:
            return CaptionVerb::DoubleClicking;
        case StageAction::Drag:
            return CaptionVerb::Dragging;
        case StageAction::Scroll:
            return CaptionVerb::Scrolling;
        case StageAction::Text:
            return CaptionVerb::Typing;
        case StageAction::Key:
        case StageAction::KeyDown:
        case StageAction::KeyUp:
        case StageAction::ButtonDown:
        case StageAction::ButtonUp:
            return CaptionVerb::PressingKeys;
        case StageAction::Focus:
            return CaptionVerb::Focusing;
        case StageAction::Snapshot:
            return CaptionVerb::Observing;
        case StageAction::Stroke:
            return CaptionVerb::Drawing;
    }
    return CaptionVerb::Moving;
}

/// Whether a pointer button is down at `now_ms`, according to a schedule.
[[nodiscard]] std::optional<PointF> held_since(const std::vector<InputStep>& schedule,
                                               std::int64_t now_ms) {
    std::optional<PointF> held;
    for (const auto& step : schedule) {
        if (step.at_ms > now_ms) {
            break;
        }
        if (step.kind == StepKind::Press) {
            held = step.point;
        } else if (step.kind == StepKind::Release) {
            held.reset();
        }
    }
    return held;
}

}  // namespace

VisibleRegion visible_region(const ViewRect& view, int output_w, int output_h) noexcept {
    const float zoom = std::max(1.0F, view.zoom);
    const float w = static_cast<float>(output_w) / zoom;
    const float h = static_cast<float>(output_h) / zoom;
    return VisibleRegion{view.cx - w / 2.0F, view.cy - h / 2.0F, w, h};
}

PointF to_screen(const ViewRect& view, int output_w, int output_h, PointF point) noexcept {
    const VisibleRegion region = visible_region(view, output_w, output_h);
    const float zoom = std::max(1.0F, view.zoom);
    return PointF{(point.x - region.x) * zoom, (point.y - region.y) * zoom};
}

StageDirector::StageDirector() : StageDirector(DirectorPolicy{}) {}

StageDirector::StageDirector(DirectorPolicy policy) : m_policy(policy) {
    m_from = overview();
    m_to = m_from;
}

ViewRect StageDirector::overview() const noexcept {
    return ViewRect{static_cast<float>(m_policy.output_w) / 2.0F,
                    static_cast<float>(m_policy.output_h) / 2.0F, 1.0F};
}

void StageDirector::set_output(int width, int height, std::int64_t now_ms) {
    if (width <= 0 || height <= 0 ||
        (width == m_policy.output_w && height == m_policy.output_h)) {
        return;
    }
    m_policy.output_w = width;
    m_policy.output_h = height;
    // A framing computed for another output size frames nothing in particular on this
    // one. Cut to the whole stage; the next action will frame itself.
    m_from = overview();
    m_to = m_from;
    m_move_since_ms = now_ms;
    m_move_ms = 0;
    m_following = false;
}

ViewRect StageDirector::frame_region(float x0, float y0, float x1, float y1) const noexcept {
    const float w = static_cast<float>(m_policy.output_w);
    const float h = static_cast<float>(m_policy.output_h);
    const float margin = std::max(0.0F, m_policy.action_margin_px);
    x0 -= margin;
    y0 -= margin;
    x1 += margin;
    y1 += margin;
    const float region_w = std::max(1.0F, x1 - x0);
    const float region_h = std::max(1.0F, y1 - y0);
    float zoom = std::min(w / region_w, h / region_h);
    zoom = std::clamp(zoom, 1.0F, std::max(1.0F, m_policy.max_zoom));

    const float view_w = w / zoom;
    const float view_h = h / zoom;
    // The view never shows past the stage's edge: there is nothing out there to frame,
    // and a black band says the camera has lost its subject.
    const float cx = std::clamp((x0 + x1) / 2.0F, view_w / 2.0F, w - view_w / 2.0F);
    const float cy = std::clamp((y0 + y1) / 2.0F, view_h / 2.0F, h - view_h / 2.0F);
    return ViewRect{cx, cy, zoom};
}

bool StageDirector::framing_serves(float x0, float y0, float x1, float y1,
                                   const ViewRect& wanted) const noexcept {
    const VisibleRegion current = visible_region(m_to, m_policy.output_w, m_policy.output_h);
    // Half the margin is enough for an action that is already on screen: it only has to
    // stay clear of the frame's edge, not be re-centred.
    const float inset = std::max(0.0F, m_policy.action_margin_px) / 2.0F;
    const bool fits = x0 - inset >= current.x && y0 - inset >= current.y &&
                      x1 + inset <= current.x + current.w && y1 + inset <= current.y + current.h;
    const bool close_enough = wanted.zoom <= m_to.zoom * (1.0F + m_policy.reframe_zoom_tolerance);
    return fits && close_enough;
}

void StageDirector::aim(const ViewRect& target, std::int64_t duration_ms, std::int64_t now_ms) {
    // Obeyed from wherever the camera actually is, never from where the last move began.
    m_from = camera_at(now_ms);
    m_to = target;
    m_move_since_ms = now_ms;
    m_move_ms = m_policy.reduced_motion ? 0 : std::max<std::int64_t>(0, duration_ms);
}

ViewRect StageDirector::camera_at(std::int64_t now_ms) const noexcept {
    if (m_move_ms <= 0 || now_ms >= m_move_since_ms + m_move_ms) {
        return m_to;
    }
    if (now_ms <= m_move_since_ms) {
        return m_from;
    }
    const float t =
        ease(static_cast<float>(now_ms - m_move_since_ms) / static_cast<float>(m_move_ms));
    // Zoom interpolates in log space, so a move from 1x to 1.8x and back feels the same
    // speed in both directions.
    const float zoom = std::exp(std::log(m_from.zoom) + (std::log(m_to.zoom) - std::log(m_from.zoom)) * t);
    return ViewRect{m_from.cx + (m_to.cx - m_from.cx) * t, m_from.cy + (m_to.cy - m_from.cy) * t, zoom};
}

bool StageDirector::camera_moving_at(std::int64_t now_ms) const noexcept {
    return m_move_ms > 0 && now_ms >= m_move_since_ms && now_ms < m_move_since_ms + m_move_ms &&
           !(m_from == m_to);
}

void StageDirector::command_started(const StageCommand& command, const Surface& surface,
                                    const Playback& playback, std::int64_t now_ms) {
    m_playback = playback;
    m_framed_surface = surface.id;

    Caption caption;
    caption.verb = verb_of(command.action);
    caption.app_id = surface.app_id;
    caption.text_chars = command.action == StageAction::Text ? command.text_chars : 0;
    caption.key_count = command.action == StageAction::Key ? command.key_count : 0;
    m_caption = caption;
    m_caption_since_ms = now_ms;
    m_caption_until_ms.reset();

    for (const auto& step : playback.schedule()) {
        if (step.kind == StepKind::Press) {
            m_presses.push_back(Press{step.point, step.at_ms});
        }
    }
    while (m_presses.size() > kMaxRememberedPresses) {
        m_presses.pop_front();
    }

    m_cursor_until_ms = std::max(m_cursor_until_ms, playback.ends_ms() + m_policy.cursor_linger_ms);
    m_last_action_end_ms = playback.ends_ms();

    if (m_mode != AuthorityMode::Autonomous) {
        return;
    }

    // What the action spans: where the pointer starts, and every point it is scheduled
    // to arrive at or press on.
    float x0 = std::numeric_limits<float>::max();
    float y0 = std::numeric_limits<float>::max();
    float x1 = std::numeric_limits<float>::lowest();
    float y1 = std::numeric_limits<float>::lowest();
    const auto include = [&](PointF point) {
        x0 = std::min(x0, point.x);
        y0 = std::min(y0, point.y);
        x1 = std::max(x1, point.x);
        y1 = std::max(y1, point.y);
    };
    bool pointer_action = false;
    for (const auto& step : playback.schedule()) {
        switch (step.kind) {
            case StepKind::Motion:
            case StepKind::Press:
            case StepKind::Release:
            case StepKind::Wheel:
                include(step.point);
                pointer_action = true;
                break;
            case StepKind::TextUnit:
            case StepKind::KeyPress:
            case StepKind::KeyRelease:
            case StepKind::FocusSurface:
            case StepKind::Capture:
                break;
        }
    }
    if (pointer_action) {
        include(playback.pointer_at(now_ms));
    } else {
        // Typing lands wherever the application put its caret, which this code cannot
        // see, and a capture takes the whole window. Frame all of it.
        include(PointF{static_cast<float>(surface.geometry.x), static_cast<float>(surface.geometry.y)});
        include(PointF{static_cast<float>(surface.geometry.x + surface.geometry.w),
                       static_cast<float>(surface.geometry.y + surface.geometry.h)});
    }

    const ViewRect wanted = frame_region(x0, y0, x1, y1);
    m_following = true;
    if (framing_serves(x0, y0, x1, y1, wanted)) {
        return;
    }
    aim(wanted, m_policy.move_ms, now_ms);
}

void StageDirector::command_ended(PlaybackState outcome, std::int64_t now_ms) {
    end_playback(outcome, std::nullopt, now_ms);
}

void StageDirector::command_failed(CommandRefusal refusal, std::int64_t now_ms) {
    end_playback(PlaybackState::Cancelled, refusal, now_ms);
}

void StageDirector::end_playback(PlaybackState outcome, std::optional<CommandRefusal> failure,
                                 std::int64_t now_ms) {
    if (!m_playback.has_value()) {
        return;
    }
    if (outcome == PlaybackState::Cancelled) {
        // Presses that were scheduled but never happened must not ripple.
        std::erase_if(m_presses, [now_ms](const Press& press) { return press.at_ms > now_ms; });
        if (m_caption.has_value()) {
            m_caption->verb = failure.has_value() ? CaptionVerb::Refused : CaptionVerb::Interrupted;
            m_caption->refusal = failure;
        }
    }
    m_last_cursor = m_playback->pointer_at(now_ms);
    m_cursor_until_ms = now_ms + m_policy.cursor_linger_ms;
    m_last_action_end_ms = now_ms;
    m_caption_until_ms = now_ms + m_policy.caption_hold_ms;
    m_playback.reset();
    if (m_caption.has_value()) {
        remember(*m_caption, now_ms);
    }
}

void StageDirector::command_refused(const StageCommand& command, CommandRefusal refusal,
                                    std::int64_t now_ms) {
    Caption caption;
    caption.verb = CaptionVerb::Refused;
    caption.app_id = command.app_id;
    caption.refusal = refusal;
    m_caption = caption;
    m_caption_since_ms = now_ms;
    m_caption_until_ms = now_ms + m_policy.caption_hold_ms;
    remember(caption, now_ms);
}

void StageDirector::remember(const Caption& caption, std::int64_t now_ms) {
    Done done;
    done.what = caption;
    done.what.opacity = 1.0F;
    done.at_ms = now_ms;
    m_activity.push_front(std::move(done));
    while (m_activity.size() > kMaxActivity) {
        m_activity.pop_back();
    }
}

void StageDirector::surface_moved(const Surface& surface, std::int64_t now_ms) {
    if (!m_following || m_framed_surface != surface.id || m_mode != AuthorityMode::Autonomous) {
        return;
    }
    const auto& g = surface.geometry;
    aim(frame_region(static_cast<float>(g.x), static_cast<float>(g.y),
                     static_cast<float>(g.x + g.w), static_cast<float>(g.y + g.h)),
        m_policy.move_ms, now_ms);
}

void StageDirector::authority_changed(AuthorityMode mode, std::int64_t now_ms) {
    if (mode == m_mode) {
        return;
    }
    m_mode = mode;
    m_mode_since_ms = now_ms;
    if (mode != AuthorityMode::Autonomous) {
        // A person's own hand gets the whole stage, at once.
        aim(overview(), m_policy.release_ms, now_ms);
        m_following = false;
    }
}

void StageDirector::tick(std::int64_t now_ms) {
    if (m_mode != AuthorityMode::Autonomous || !m_following || m_playback.has_value()) {
        return;
    }
    if (now_ms >= m_last_action_end_ms + m_policy.settle_ms) {
        aim(overview(), m_policy.move_ms, now_ms);
        m_following = false;
    }
}

StageFrame StageDirector::frame(std::int64_t now_ms, RenderTier tier) const {
    StageFrame out;
    out.view = camera_at(now_ms);
    if (tier == RenderTier::Headless) {
        // Nothing is presented. The state stays correct for the moment a screen returns,
        // and no frame is requested.
        return out;
    }
    const RenderProfile profile = profile_of(tier);
    const bool reduced = m_policy.reduced_motion;
    const bool autonomous = m_mode == AuthorityMode::Autonomous;
    out.camera_moving = camera_moving_at(now_ms);

    bool changing = out.camera_moving;
    std::optional<std::int64_t> deadline;
    const auto consider = [&deadline, now_ms](std::int64_t at) {
        if (at > now_ms && (!deadline.has_value() || at < *deadline)) {
            deadline = at;
        }
    };

    // The agent's cursor. Only while the agent holds the stage: under a person's hand
    // there is exactly one pointer on screen, and it is theirs.
    std::optional<PointF> cursor = m_last_cursor;
    std::optional<PointF> held;
    if (m_playback.has_value()) {
        cursor = m_playback->pointer_at(now_ms);
        held = held_since(m_playback->schedule(), now_ms);
        if (now_ms < m_playback->ends_ms()) {
            changing = true;
        }
    }
    if (autonomous && cursor.has_value() && now_ms < m_cursor_until_ms) {
        out.cursor_visible = true;
        out.cursor = *cursor;
        out.cursor_opacity = 1.0F;
        if (!m_playback.has_value() && !reduced && m_policy.cursor_fade_ms > 0) {
            const std::int64_t fade_start = m_cursor_until_ms - m_policy.cursor_fade_ms;
            if (now_ms >= fade_start) {
                out.cursor_opacity = static_cast<float>(m_cursor_until_ms - now_ms) /
                                     static_cast<float>(m_policy.cursor_fade_ms);
                changing = true;
            } else {
                consider(fade_start);
            }
        }
        consider(m_cursor_until_ms);
        out.cursor_pressed = held.has_value();
        if (held.has_value() && m_playback.has_value() &&
            m_playback->command().action == StageAction::Drag) {
            out.drag_from = held;
        }
    }

    // Ripples: a press that happened, expanding once. Nothing ripples on its own, under
    // reduced motion, or when the machine needs its GPU back.
    if (!reduced && tier != RenderTier::Conserve && m_policy.ripple_ms > 0) {
        for (auto press = m_presses.rbegin(); press != m_presses.rend(); ++press) {
            if (press->at_ms > now_ms) {
                consider(press->at_ms);
                continue;
            }
            if (now_ms >= press->at_ms + m_policy.ripple_ms || out.ripples.size() >= kMaxRipples) {
                continue;
            }
            const float progress = static_cast<float>(now_ms - press->at_ms) /
                                   static_cast<float>(m_policy.ripple_ms);
            out.ripples.push_back(
                Ripple{press->at, m_policy.ripple_radius_px * out_cubic(progress), 1.0F - progress});
            changing = true;
        }
    }

    // The caption.
    if (m_caption.has_value()) {
        const bool active = m_playback.has_value();
        const bool held_on = m_caption_until_ms.has_value() && now_ms < *m_caption_until_ms;
        if (active || held_on) {
            Caption caption = *m_caption;
            caption.opacity = 1.0F;
            if (!reduced && m_policy.fade_ms > 0) {
                const std::int64_t age = now_ms - m_caption_since_ms;
                if (age >= 0 && age < m_policy.fade_ms) {
                    caption.opacity = static_cast<float>(age) / static_cast<float>(m_policy.fade_ms);
                    changing = true;
                }
                if (!active) {
                    const std::int64_t fade_start = *m_caption_until_ms - m_policy.fade_ms;
                    if (now_ms >= fade_start) {
                        caption.opacity = std::min(
                            caption.opacity, static_cast<float>(*m_caption_until_ms - now_ms) /
                                                 static_cast<float>(m_policy.fade_ms));
                        changing = true;
                    } else {
                        consider(fade_start);
                    }
                }
            }
            if (!active) {
                consider(*m_caption_until_ms);
            }
            out.caption = caption;
        }
    }

    // The activity strip: what the agent did lately, whoever holds the stage now — a person
    // who has just taken over is exactly who wants to know.
    for (const Done& done : m_activity) {
        const std::int64_t until = done.at_ms + m_policy.activity_ms;
        if (now_ms < done.at_ms || now_ms >= until) {
            continue;
        }
        Caption entry = done.what;
        entry.opacity = 1.0F;
        if (!reduced && m_policy.fade_ms > 0) {
            const std::int64_t fade_start = until - m_policy.fade_ms;
            if (now_ms >= fade_start) {
                entry.opacity = static_cast<float>(until - now_ms) / static_cast<float>(m_policy.fade_ms);
                changing = true;
            } else {
                consider(fade_start);
            }
        }
        consider(until);
        out.activity.push_back(std::move(entry));
    }

    // The banner.
    if (!autonomous) {
        out.banner = m_mode;
        out.banner_opacity = 1.0F;
        if (!reduced && m_policy.fade_ms > 0) {
            const std::int64_t age = now_ms - m_mode_since_ms;
            if (age >= 0 && age < m_policy.fade_ms) {
                out.banner_opacity = static_cast<float>(age) / static_cast<float>(m_policy.fade_ms);
                changing = true;
            }
        }
    }

    // The camera going home is a deadline that needs a frame even if nothing else moves.
    if (autonomous && m_following && !m_playback.has_value()) {
        consider(m_last_action_end_ms + m_policy.settle_ms);
    }

    out.animate = changing;
    if (changing) {
        out.next_frame_at_ms = now_ms + std::max(1, profile.frame_interval_ms);
    } else {
        out.next_frame_at_ms = deadline;
    }
    return out;
}

}  // namespace archie::visual
