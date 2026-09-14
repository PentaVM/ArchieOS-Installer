#include "archie/visual/stage.hpp"

#include <algorithm>
#include <cmath>
#include <cstdlib>

namespace archie::visual {

// ---------------------------------------------------------------------------------------
// Surfaces
// ---------------------------------------------------------------------------------------

bool SurfaceTable::map(SurfaceId id, std::string app_id, IRect geometry, bool protected_surface) {
    if (geometry.empty() || m_surfaces.contains(id)) {
        return false;
    }
    Surface surface;
    surface.id = id;
    surface.app_id = std::move(app_id);
    surface.geometry = geometry;
    // Revisions come from one counter for the whole table, so a surface that is closed and
    // reopened under the same id never matches a command prepared against its old self.
    surface.geometry_revision = m_next_geometry_revision++;
    surface.mapped = true;
    surface.protected_surface = protected_surface;
    m_surfaces.emplace(id, std::move(surface));
    ++m_revision;
    return true;
}

bool SurfaceTable::unmap(SurfaceId id) {
    if (m_surfaces.erase(id) == 0) {
        return false;
    }
    if (m_focused == id) {
        m_focused.reset();
    }
    ++m_revision;
    return true;
}

bool SurfaceTable::move(SurfaceId id, IRect geometry) {
    const auto found = m_surfaces.find(id);
    if (found == m_surfaces.end() || geometry.empty()) {
        return false;
    }
    if (found->second.geometry == geometry) {
        return true;
    }
    found->second.geometry = geometry;
    found->second.geometry_revision = m_next_geometry_revision++;
    ++m_revision;
    return true;
}

bool SurfaceTable::focus(SurfaceId id) {
    const auto found = m_surfaces.find(id);
    if (found == m_surfaces.end() || !found->second.mapped) {
        return false;
    }
    if (m_focused != id) {
        m_focused = id;
        ++m_revision;
    }
    return true;
}

const Surface* SurfaceTable::find(SurfaceId id) const {
    const auto found = m_surfaces.find(id);
    return found == m_surfaces.end() ? nullptr : &found->second;
}

std::vector<const Surface*> SurfaceTable::mapped_surfaces() const {
    std::vector<const Surface*> out;
    for (const auto& [id, surface] : m_surfaces) {
        (void)id;
        if (surface.mapped) {
            out.push_back(&surface);
        }
    }
    return out;
}

// ---------------------------------------------------------------------------------------
// Authority
// ---------------------------------------------------------------------------------------

namespace {

constexpr std::size_t kMaxMotionSamples = 128;

}  // namespace

void InputAuthority::enter(AuthorityMode mode, AuthorityReason reason, std::int64_t now_ms) {
    m_mode = mode;
    m_reason = reason;
    m_since_ms = now_ms;
    m_motion.clear();
}

void InputAuthority::take_over(AuthorityReason reason, std::int64_t now_ms) {
    enter(AuthorityMode::Operator, reason, now_ms);
}

void InputAuthority::physical_motion(float dx, float dy, std::int64_t now_ms) {
    m_last_physical_ms = now_ms;
    if (m_mode != AuthorityMode::Autonomous) {
        return;
    }
    float distance = std::hypot(dx, dy);
    if (!std::isfinite(distance)) {
        // A device reporting nonsense is not a person reaching for the mouse.
        distance = 0.0F;
    }
    m_motion.push_back(MotionSample{now_ms, distance});
    while (m_motion.size() > kMaxMotionSamples) {
        m_motion.pop_front();
    }
    // A sliding window, not a tumbling one: a deliberate movement that happens to straddle
    // a window boundary is still one deliberate movement.
    float travelled = 0.0F;
    for (const auto& sample : m_motion) {
        if (now_ms - sample.at_ms <= m_policy.takeover_window_ms && sample.at_ms <= now_ms) {
            travelled += sample.distance;
        }
    }
    if (travelled >= m_policy.takeover_motion_px) {
        take_over(AuthorityReason::OperatorMotion, now_ms);
    }
}

void InputAuthority::physical_button(std::int64_t now_ms) {
    m_last_physical_ms = now_ms;
    if (m_mode == AuthorityMode::Autonomous) {
        take_over(AuthorityReason::OperatorButton, now_ms);
    }
}

void InputAuthority::physical_key(std::int64_t now_ms) {
    m_last_physical_ms = now_ms;
    if (m_mode == AuthorityMode::Autonomous) {
        take_over(AuthorityReason::OperatorKey, now_ms);
    }
}

void InputAuthority::pause_chord(std::int64_t now_ms) {
    m_last_physical_ms = now_ms;
    enter(AuthorityMode::Paused, AuthorityReason::PauseChord, now_ms);
}

void InputAuthority::resume(std::int64_t now_ms) {
    m_last_physical_ms = now_ms;
    if (m_mode != AuthorityMode::Autonomous) {
        enter(AuthorityMode::Autonomous, AuthorityReason::Resumed, now_ms);
    }
}

void InputAuthority::tick(std::int64_t now_ms) {
    if (m_mode != AuthorityMode::Operator || m_policy.operator_idle_resume_ms <= 0) {
        return;
    }
    // A clock that went backwards has not produced a quiet period.
    if (now_ms - m_last_physical_ms >= m_policy.operator_idle_resume_ms) {
        enter(AuthorityMode::Autonomous, AuthorityReason::OperatorIdle, now_ms);
    }
}

std::optional<std::int64_t> InputAuthority::resumes_at_ms() const noexcept {
    if (m_mode != AuthorityMode::Operator || m_policy.operator_idle_resume_ms <= 0) {
        return std::nullopt;
    }
    return m_last_physical_ms + m_policy.operator_idle_resume_ms;
}

// ---------------------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------------------

const char* refusal_code(CommandRefusal refusal) noexcept {
    switch (refusal) {
        case CommandRefusal::Paused:
            return "stage_paused";
        case CommandRefusal::OperatorHasControl:
            return "operator_has_control";
        case CommandRefusal::Expired:
            return "command_expired";
        case CommandRefusal::Replayed:
            return "command_replayed";
        case CommandRefusal::UnknownSurface:
            return "surface_unknown";
        case CommandRefusal::SurfaceUnmapped:
            return "surface_unmapped";
        case CommandRefusal::SurfaceProtected:
            return "surface_protected";
        case CommandRefusal::AppMismatch:
            return "app_mismatch";
        case CommandRefusal::SurfaceChanged:
            return "surface_changed";
        case CommandRefusal::OutsideSurface:
            return "point_outside_surface";
        case CommandRefusal::ScrollOutOfBounds:
            return "scroll_out_of_bounds";
        case CommandRefusal::TextOutOfBounds:
            return "text_out_of_bounds";
        case CommandRefusal::ChordOutOfBounds:
            return "chord_out_of_bounds";
        case CommandRefusal::NotTypeable:
            return "not_typeable";
    }
    return "command_refused";
}

std::optional<CommandRefusal> check_command(const StageCommand& command,
                                            const SurfaceTable& table,
                                            const InputAuthority& authority,
                                            std::uint64_t last_sequence, std::int64_t now_ms) {
    switch (authority.mode()) {
        case AuthorityMode::Paused:
            return CommandRefusal::Paused;
        case AuthorityMode::Operator:
            return CommandRefusal::OperatorHasControl;
        case AuthorityMode::Autonomous:
            break;
    }
    if (now_ms >= command.expires_at_ms) {
        return CommandRefusal::Expired;
    }
    if (command.sequence <= last_sequence) {
        return CommandRefusal::Replayed;
    }
    const Surface* surface = table.find(command.surface);
    if (surface == nullptr) {
        return CommandRefusal::UnknownSurface;
    }
    if (!surface->mapped) {
        return CommandRefusal::SurfaceUnmapped;
    }
    if (surface->protected_surface) {
        return CommandRefusal::SurfaceProtected;
    }
    if (surface->app_id.empty() || surface->app_id != command.app_id) {
        return CommandRefusal::AppMismatch;
    }
    if (surface->geometry_revision != command.geometry_revision) {
        return CommandRefusal::SurfaceChanged;
    }

    const auto inside = [surface](int x, int y) {
        return x >= 0 && y >= 0 && x < surface->geometry.w && y < surface->geometry.h;
    };
    switch (command.action) {
        case StageAction::Move:
        case StageAction::Click:
        case StageAction::DoubleClick:
            if (!inside(command.local_x, command.local_y)) {
                return CommandRefusal::OutsideSurface;
            }
            break;
        case StageAction::Drag:
            if (!inside(command.local_x, command.local_y) ||
                !inside(command.to_local_x, command.to_local_y)) {
                return CommandRefusal::OutsideSurface;
            }
            break;
        case StageAction::Scroll:
            if (!inside(command.local_x, command.local_y)) {
                return CommandRefusal::OutsideSurface;
            }
            if ((command.scroll_steps_x == 0 && command.scroll_steps_y == 0) ||
                std::abs(command.scroll_steps_x) > kMaxScrollSteps ||
                std::abs(command.scroll_steps_y) > kMaxScrollSteps) {
                return CommandRefusal::ScrollOutOfBounds;
            }
            break;
        case StageAction::Text:
            if (command.text_chars == 0 || command.text_chars > kMaxTextChars) {
                return CommandRefusal::TextOutOfBounds;
            }
            break;
        case StageAction::Key:
            if (command.key_count <= 0 || command.key_count > kMaxChordKeys) {
                return CommandRefusal::ChordOutOfBounds;
            }
            break;
        case StageAction::Focus:
            break;
    }
    return std::nullopt;
}

PointF to_output(const Surface& surface, int local_x, int local_y) noexcept {
    // The centre of the pixel, not its corner: Wayland coordinates are fractional, and a
    // point on a pixel's edge belongs to whichever neighbour rounding prefers.
    return PointF{static_cast<float>(surface.geometry.x + local_x) + 0.5F,
                  static_cast<float>(surface.geometry.y + local_y) + 0.5F};
}

// ---------------------------------------------------------------------------------------
// Trajectories
// ---------------------------------------------------------------------------------------

namespace {

constexpr float kMaxBowPx = 120.0F;

[[nodiscard]] float clamp01(float value) noexcept {
    return value < 0.0F ? 0.0F : (value > 1.0F ? 1.0F : value);
}

/// Minimum-jerk position profile: zero velocity and acceleration at both ends.
[[nodiscard]] float minimum_jerk(float tau) noexcept {
    const float t = clamp01(tau);
    return t * t * t * (10.0F + t * (-15.0F + 6.0F * t));
}

}  // namespace

PointerTrajectory PointerTrajectory::plan(PointF from, PointF to, std::int64_t start_ms,
                                          const TrajectoryPolicy& policy) {
    PointerTrajectory trajectory;
    trajectory.m_from = from;
    trajectory.m_to = to;
    trajectory.m_start_ms = start_ms;
    trajectory.m_control = to;

    const float dx = to.x - from.x;
    const float dy = to.y - from.y;
    const float distance = std::hypot(dx, dy);
    if (!(distance >= 0.5F)) {
        // Already there, or a coordinate that is not a number. No movement either way.
        trajectory.m_duration_ms = 0;
        return trajectory;
    }

    const float width = std::max(1.0F, policy.target_width_px);
    const float fitts = policy.fitts_a_ms + policy.fitts_b_ms * std::log2(distance / width + 1.0F);
    const auto bounded = std::clamp(static_cast<std::int64_t>(std::lround(fitts)),
                                    policy.min_duration_ms, policy.max_duration_ms);
    const float pace = std::clamp(policy.pace, 0.25F, 4.0F);
    trajectory.m_duration_ms = static_cast<std::int64_t>(std::lround(static_cast<float>(bounded) * pace));

    // A straight line reads as a machine and a random wobble reads as a glitch. A small,
    // consistent bow to one side reads as a hand.
    const float offset = std::min(kMaxBowPx, std::max(0.0F, policy.bow) * distance);
    const float nx = -dy / distance;
    const float ny = dx / distance;
    trajectory.m_control =
        PointF{(from.x + to.x) / 2.0F + nx * offset, (from.y + to.y) / 2.0F + ny * offset};
    return trajectory;
}

PointF PointerTrajectory::at(std::int64_t now_ms) const noexcept {
    if (now_ms <= m_start_ms) {
        return m_duration_ms == 0 && now_ms == m_start_ms ? m_to : m_from;
    }
    if (m_duration_ms == 0 || now_ms >= m_start_ms + m_duration_ms) {
        return m_to;
    }
    const float tau = static_cast<float>(now_ms - m_start_ms) / static_cast<float>(m_duration_ms);
    const float s = minimum_jerk(tau);
    const float u = 1.0F - s;
    return PointF{u * u * m_from.x + 2.0F * u * s * m_control.x + s * s * m_to.x,
                  u * u * m_from.y + 2.0F * u * s * m_control.y + s * s * m_to.y};
}

// ---------------------------------------------------------------------------------------
// Playback
// ---------------------------------------------------------------------------------------

Playback Playback::start(const StageCommand& command, const Surface& surface, PointF pointer_now,
                         std::int64_t now_ms, const PlaybackPolicy& policy) {
    Playback playback;
    playback.m_command = command;
    playback.m_started_ms = now_ms;
    playback.m_rest = pointer_now;
    playback.m_last_motion = pointer_now;

    auto& steps = playback.m_steps;
    const auto arrive = [&playback, &steps](const PointerTrajectory& trajectory) {
        playback.m_trajectories.push_back(trajectory);
        steps.push_back(InputStep{trajectory.end_ms(), StepKind::Motion, trajectory.to(),
                                  PointerButton::Left, 0, 0, 0});
    };
    const auto step = [&steps](std::int64_t at, StepKind kind, PointF point, PointerButton button,
                               std::size_t index) {
        steps.push_back(InputStep{at, kind, point, button, 0, 0, index});
    };

    const PointF target = to_output(surface, command.local_x, command.local_y);
    std::int64_t end = now_ms;

    switch (command.action) {
        case StageAction::Move: {
            const auto travel = PointerTrajectory::plan(pointer_now, target, now_ms, policy.trajectory);
            arrive(travel);
            end = travel.end_ms();
            break;
        }
        case StageAction::Click:
        case StageAction::DoubleClick: {
            const auto travel = PointerTrajectory::plan(pointer_now, target, now_ms, policy.trajectory);
            arrive(travel);
            std::int64_t at = travel.end_ms();
            const int clicks = command.action == StageAction::DoubleClick ? 2 : 1;
            for (int click = 0; click < clicks; ++click) {
                if (click > 0) {
                    at += policy.double_click_gap_ms;
                }
                step(at, StepKind::Press, target, command.button, 0);
                at += policy.press_dwell_ms;
                step(at, StepKind::Release, target, command.button, 0);
            }
            end = at;
            break;
        }
        case StageAction::Drag: {
            const PointF drop = to_output(surface, command.to_local_x, command.to_local_y);
            const auto reach = PointerTrajectory::plan(pointer_now, target, now_ms, policy.trajectory);
            arrive(reach);
            step(reach.end_ms(), StepKind::Press, target, command.button, 0);
            // A drag is carried, not flung: never faster than a quarter of a second.
            TrajectoryPolicy carry = policy.trajectory;
            carry.min_duration_ms = std::max<std::int64_t>(carry.min_duration_ms, 250);
            const auto haul =
                PointerTrajectory::plan(target, drop, reach.end_ms() + policy.drag_hold_ms, carry);
            arrive(haul);
            end = haul.end_ms() + policy.press_dwell_ms;
            step(end, StepKind::Release, drop, command.button, 0);
            break;
        }
        case StageAction::Scroll: {
            const auto travel = PointerTrajectory::plan(pointer_now, target, now_ms, policy.trajectory);
            arrive(travel);
            const int count = std::max(std::abs(command.scroll_steps_x), std::abs(command.scroll_steps_y));
            std::int64_t at = travel.end_ms();
            for (int i = 0; i < count; ++i) {
                at = travel.end_ms() + static_cast<std::int64_t>(i) * policy.wheel_interval_ms;
                InputStep wheel{at, StepKind::Wheel, target, PointerButton::Left, 0, 0,
                                static_cast<std::size_t>(i)};
                if (i < std::abs(command.scroll_steps_x)) {
                    wheel.wheel_x = command.scroll_steps_x > 0 ? 1 : -1;
                }
                if (i < std::abs(command.scroll_steps_y)) {
                    wheel.wheel_y = command.scroll_steps_y > 0 ? 1 : -1;
                }
                steps.push_back(wheel);
            }
            end = at;
            break;
        }
        case StageAction::Text: {
            step(now_ms, StepKind::FocusSurface, pointer_now, PointerButton::Left, 0);
            std::int64_t at = now_ms;
            for (std::size_t i = 0; i < command.text_chars; ++i) {
                at = now_ms + policy.focus_settle_ms + static_cast<std::int64_t>(i) * policy.key_interval_ms;
                step(at, StepKind::TextUnit, pointer_now, PointerButton::Left, i);
            }
            end = at;
            break;
        }
        case StageAction::Key: {
            step(now_ms, StepKind::FocusSurface, pointer_now, PointerButton::Left, 0);
            const auto keys = static_cast<std::size_t>(std::max(0, command.key_count));
            std::int64_t at = now_ms;
            for (std::size_t i = 0; i < keys; ++i) {
                at = now_ms + policy.focus_settle_ms + static_cast<std::int64_t>(i) * policy.key_interval_ms;
                step(at, StepKind::KeyPress, pointer_now, PointerButton::Left, i);
            }
            at += policy.press_dwell_ms;
            // Released in the reverse order, the way a chord is let go.
            for (std::size_t i = keys; i > 0; --i) {
                step(at, StepKind::KeyRelease, pointer_now, PointerButton::Left, i - 1);
                at += policy.key_interval_ms;
            }
            end = keys > 0 ? at - policy.key_interval_ms : now_ms;
            break;
        }
        case StageAction::Focus:
            step(now_ms, StepKind::FocusSurface, pointer_now, PointerButton::Left, 0);
            end = now_ms;
            break;
    }

    std::stable_sort(steps.begin(), steps.end(),
                     [](const InputStep& a, const InputStep& b) { return a.at_ms < b.at_ms; });
    playback.m_ends_ms = end;
    return playback;
}

PointF Playback::pointer_at(std::int64_t now_ms) const noexcept {
    PointF position = m_rest;
    for (const auto& trajectory : m_trajectories) {
        if (now_ms < trajectory.start_ms()) {
            break;
        }
        position = trajectory.at(now_ms);
    }
    return position;
}

std::optional<PointerTrajectory> Playback::trajectory_at(std::int64_t now_ms) const noexcept {
    for (const auto& trajectory : m_trajectories) {
        if (now_ms >= trajectory.start_ms() && now_ms < trajectory.end_ms()) {
            return trajectory;
        }
    }
    return std::nullopt;
}

std::vector<InputStep> Playback::due(std::int64_t now_ms) {
    std::vector<InputStep> out;
    if (m_state != PlaybackState::Running) {
        return out;
    }
    while (m_next < m_steps.size() && m_steps[m_next].at_ms <= now_ms) {
        const InputStep& next = m_steps[m_next++];
        switch (next.kind) {
            case StepKind::Motion:
                m_last_motion = next.point;
                m_emitted_motion = true;
                break;
            case StepKind::Press:
                m_button_held = true;
                m_held_button = next.button;
                break;
            case StepKind::Release:
                m_button_held = false;
                break;
            case StepKind::KeyPress:
                m_keys_down.push_back(next.index);
                break;
            case StepKind::KeyRelease:
                std::erase(m_keys_down, next.index);
                break;
            case StepKind::Wheel:
            case StepKind::TextUnit:
            case StepKind::FocusSurface:
                break;
        }
        out.push_back(next);
    }
    // Between the scheduled arrivals, sample the trajectory in flight. Scheduled steps go
    // first, so a frame that arrives late still presses where the pointer arrived rather
    // than where it has since been carried.
    if (const auto flight = trajectory_at(now_ms); flight.has_value() && now_ms > flight->start_ms()) {
        const PointF position = flight->at(now_ms);
        if (!m_emitted_motion || !(position == m_last_motion)) {
            out.push_back(InputStep{now_ms, StepKind::Motion, position, PointerButton::Left, 0, 0, 0});
            m_last_motion = position;
            m_emitted_motion = true;
        }
    }
    if (m_next == m_steps.size() && now_ms >= m_ends_ms) {
        m_state = PlaybackState::Finished;
    }
    return out;
}

std::vector<InputStep> Playback::cancel(std::int64_t now_ms) {
    std::vector<InputStep> out;
    if (m_state != PlaybackState::Running) {
        return out;
    }
    // An application left with a button held is in the middle of a drag nobody is
    // performing. Whatever interrupted the agent, the pointer and keyboard are handed
    // back clean.
    if (m_button_held) {
        out.push_back(InputStep{now_ms, StepKind::Release, m_last_motion, m_held_button, 0, 0, 0});
        m_button_held = false;
    }
    for (auto key = m_keys_down.rbegin(); key != m_keys_down.rend(); ++key) {
        out.push_back(InputStep{now_ms, StepKind::KeyRelease, m_last_motion, PointerButton::Left,
                                0, 0, *key});
    }
    m_keys_down.clear();
    m_state = PlaybackState::Cancelled;
    return out;
}

}  // namespace archie::visual
