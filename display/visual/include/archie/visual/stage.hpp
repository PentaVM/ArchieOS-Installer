#pragma once

// The rules of a stage that Archie operates and a person watches.
//
// ArchieOS runs applications inside its own compositor, and Archie works them the way a
// person would: it moves a pointer, clicks, drags, scrolls and types. Nobody has to touch
// a mouse for anything to happen. That changes who the input belongs to, and this header
// is where that is decided — before any of it reaches Qt, a socket or an application.
//
// Four lies shape it.
//
// The first is an assistant that cannot be stopped. An agent that moves the pointer on
// its own is only acceptable if a person can take the machine back instantly, without
// knowing a command and without the agent's cooperation. So physical input always
// outranks the agent: a deliberate movement, any button or any key hands the stage to
// the operator in the same frame, and the agent's in-flight action is cancelled with its
// buttons and keys released. A dedicated chord pauses the agent until a person says
// otherwise. Neither can be produced by the agent, because agent input is delivered to
// applications directly and never passes through the physical-input path below.
//
// The second is a click that lands somewhere nobody authorised. A command is authorised
// against a surface as it was when the command was prepared — its application, its
// geometry revision, a point inside it. If the window has moved, resized, closed or
// turned out to belong to another application by the time the command runs, the command
// is refused rather than re-aimed. Coordinates are resolved at execution time, from the
// surface, never carried as absolute screen positions.
//
// The third is robotic motion. A pointer that teleports cannot be followed, so a person
// watching cannot tell what was clicked, and some applications only react to a hover
// that precedes a press. Movement follows a minimum-jerk profile along a slightly bowed
// path, with a duration from Fitts's law: short and precise moves are quick, long ones
// take longer, and none of it is random, so the same command always moves the same way.
//
// The fourth is the typed secret on a shared screen. This library never holds the text
// the agent types — only how many characters there are. The compositor holds the string
// for exactly as long as it takes to deliver it.
//
// No clock, no Qt, no socket. Time is a parameter.

#include <cstddef>
#include <cstdint>
#include <deque>
#include <map>
#include <optional>
#include <string>
#include <vector>

namespace archie::visual {

/// A point on the output, in logical pixels, origin top-left.
struct PointF {
    float x = 0.0F;
    float y = 0.0F;

    friend bool operator==(const PointF&, const PointF&) = default;
};

/// An integer rectangle on the output, in logical pixels.
struct IRect {
    int x = 0;
    int y = 0;
    int w = 0;
    int h = 0;

    [[nodiscard]] bool empty() const noexcept { return w <= 0 || h <= 0; }
    [[nodiscard]] PointF center() const noexcept {
        return PointF{static_cast<float>(x) + static_cast<float>(w) / 2.0F,
                      static_cast<float>(y) + static_cast<float>(h) / 2.0F};
    }

    friend bool operator==(const IRect&, const IRect&) = default;
};

using SurfaceId = std::uint32_t;

/// One application window on the stage, as the compositor knows it.
struct Surface {
    SurfaceId id = 0;
    /// The application's own identifier (`xdg_toplevel.app_id`). Empty when the client
    /// never set one, and then no command can name it.
    std::string app_id;
    IRect geometry{};
    /// Increases whenever `geometry` changes. A command carries the revision it was
    /// prepared against.
    std::uint64_t geometry_revision = 0;
    bool mapped = false;
    /// The compositor's own surfaces: approval prompts, the pause banner. The agent may
    /// never deliver input to one, because a prompt the agent can click is not a prompt.
    bool protected_surface = false;
};

/// Every surface on the stage. The compositor's view of the world, nothing more.
class SurfaceTable final {
public:
    /// Add a surface. Refused for an id already present or an empty geometry.
    bool map(SurfaceId id, std::string app_id, IRect geometry, bool protected_surface = false);
    /// Remove a surface. Returns whether it was present.
    bool unmap(SurfaceId id);
    /// Record a new geometry. The revision only moves when the geometry actually changed,
    /// so a compositor that re-reports the same size does not invalidate commands.
    bool move(SurfaceId id, IRect geometry);
    /// Keyboard focus. Refused for an unknown or unmapped surface.
    bool focus(SurfaceId id);

    [[nodiscard]] const Surface* find(SurfaceId id) const;
    [[nodiscard]] std::optional<SurfaceId> focused() const noexcept { return m_focused; }
    [[nodiscard]] std::vector<const Surface*> mapped_surfaces() const;
    /// Increases on every accepted change to the table.
    [[nodiscard]] std::uint64_t revision() const noexcept { return m_revision; }

private:
    std::map<SurfaceId, Surface> m_surfaces;
    std::optional<SurfaceId> m_focused{};
    std::uint64_t m_revision = 0;
    std::uint64_t m_next_geometry_revision = 1;
};

/// Who the stage's input belongs to right now.
enum class AuthorityMode {
    /// The agent works. Physical input is not forwarded to applications; it is watched
    /// only for a person wanting the machine back.
    Autonomous,
    /// A person has taken over. Their input reaches applications and the agent's does
    /// not. Returns to `Autonomous` after a quiet period, if the policy allows it.
    Operator,
    /// A person stopped the agent. Physical input reaches applications, and the agent
    /// stays off until a person explicitly resumes it.
    Paused,
};

enum class AuthorityReason {
    Start,
    OperatorMotion,
    OperatorButton,
    OperatorKey,
    OperatorIdle,
    PauseChord,
    Resumed,
};

struct AuthorityPolicy {
    /// How far the pointer must travel within `takeover_window_ms` to count as a person
    /// reaching for it, rather than a desk being bumped.
    float takeover_motion_px = 48.0F;
    std::int64_t takeover_window_ms = 400;
    /// Quiet time after which an operator takeover hands the stage back to the agent.
    /// Zero means never: only an explicit resume does.
    std::int64_t operator_idle_resume_ms = 8'000;
};

/// The input authority state machine. Only the compositor's physical-input path calls
/// the `physical_*`, `pause_chord` and `resume` methods; agent commands never reach it.
class InputAuthority final {
public:
    InputAuthority() = default;
    explicit InputAuthority(AuthorityPolicy policy) : m_policy(policy) {}

    void physical_motion(float dx, float dy, std::int64_t now_ms);
    void physical_button(std::int64_t now_ms);
    void physical_key(std::int64_t now_ms);
    void pause_chord(std::int64_t now_ms);
    /// A person, at the machine, says the agent may continue.
    void resume(std::int64_t now_ms);
    /// Let time pass: an operator takeover may expire into autonomy.
    void tick(std::int64_t now_ms);

    [[nodiscard]] AuthorityMode mode() const noexcept { return m_mode; }
    [[nodiscard]] AuthorityReason reason() const noexcept { return m_reason; }
    [[nodiscard]] std::int64_t since_ms() const noexcept { return m_since_ms; }
    [[nodiscard]] bool accepts_agent_input() const noexcept {
        return m_mode == AuthorityMode::Autonomous;
    }
    /// Whether physical input should reach applications.
    [[nodiscard]] bool forwards_physical_input() const noexcept {
        return m_mode != AuthorityMode::Autonomous;
    }
    /// When an operator takeover will expire on its own, if it will.
    [[nodiscard]] std::optional<std::int64_t> resumes_at_ms() const noexcept;
    [[nodiscard]] const AuthorityPolicy& policy() const noexcept { return m_policy; }

private:
    void take_over(AuthorityReason reason, std::int64_t now_ms);
    void enter(AuthorityMode mode, AuthorityReason reason, std::int64_t now_ms);

    AuthorityPolicy m_policy{};
    AuthorityMode m_mode = AuthorityMode::Autonomous;
    AuthorityReason m_reason = AuthorityReason::Start;
    std::int64_t m_since_ms = 0;
    std::int64_t m_last_physical_ms = 0;

    struct MotionSample {
        std::int64_t at_ms = 0;
        float distance = 0.0F;
    };
    std::deque<MotionSample> m_motion;
};

/// What the agent asks the stage to do.
enum class StageAction {
    Move,
    Click,
    DoubleClick,
    Drag,
    Scroll,
    Text,
    Key,
    Focus,
    /// Capture what the surface shows. It changes nothing in the application, and it is
    /// still a command: pixels are a disclosure, so looking is authorised like touching.
    Snapshot,
};

enum class PointerButton {
    Left,
    Middle,
    Right,
};

inline constexpr std::size_t kMaxTextChars = 2'000;
inline constexpr int kMaxScrollSteps = 60;
inline constexpr int kMaxChordKeys = 4;

/// A command that the control plane has already authorised and sealed. This is the part
/// the compositor re-checks against the stage as it is when the command runs.
///
/// There is no text here. For `Text` only the number of characters travels this far.
struct StageCommand {
    std::uint64_t sequence = 0;
    std::int64_t expires_at_ms = 0;
    StageAction action = StageAction::Move;
    SurfaceId surface = 0;
    std::string app_id;
    std::uint64_t geometry_revision = 0;
    /// Target, in the surface's own logical pixels.
    int local_x = 0;
    int local_y = 0;
    /// Drag end, in the surface's own logical pixels.
    int to_local_x = 0;
    int to_local_y = 0;
    PointerButton button = PointerButton::Left;
    int scroll_steps_x = 0;
    int scroll_steps_y = 0;
    std::size_t text_chars = 0;
    int key_count = 0;
};

/// Why the compositor would not run a command. Stable codes: they end up in receipts.
enum class CommandRefusal {
    Paused,
    OperatorHasControl,
    Expired,
    Replayed,
    UnknownSurface,
    SurfaceUnmapped,
    SurfaceProtected,
    AppMismatch,
    SurfaceChanged,
    OutsideSurface,
    ScrollOutOfBounds,
    TextOutOfBounds,
    ChordOutOfBounds,
    /// The keyboard the stage presents cannot produce a character or key of the command,
    /// and the application offers no text-input path for it either. Only the compositor
    /// knows its keymap, so `check_command` never returns this; the compositor does, before
    /// a playback starts or when one has to stop.
    NotTypeable,
    /// The compositor could not render or store the capture a snapshot asked for.
    CaptureFailed,
};

[[nodiscard]] const char* refusal_code(CommandRefusal refusal) noexcept;

/// The re-check. Ordered so that a paused or operator-held stage refuses before it
/// reveals anything about which surfaces exist.
[[nodiscard]] std::optional<CommandRefusal> check_command(const StageCommand& command,
                                                          const SurfaceTable& table,
                                                          const InputAuthority& authority,
                                                          std::uint64_t last_sequence,
                                                          std::int64_t now_ms);

/// Output coordinates of a surface-local point.
[[nodiscard]] PointF to_output(const Surface& surface, int local_x, int local_y) noexcept;

struct TrajectoryPolicy {
    /// Fitts's law, T = a + b * log2(distance / width + 1).
    float fitts_a_ms = 180.0F;
    float fitts_b_ms = 120.0F;
    std::int64_t min_duration_ms = 160;
    std::int64_t max_duration_ms = 900;
    /// The effective target width when nothing better is known.
    float target_width_px = 24.0F;
    /// Largest sideways bow of the path, as a fraction of its length.
    float bow = 0.08F;
    /// Slows everything down for presentation, or speeds it up. 1 is a brisk person.
    float pace = 1.0F;
};

/// One pointer movement. Deterministic: the same inputs give the same path.
class PointerTrajectory final {
public:
    [[nodiscard]] static PointerTrajectory plan(PointF from, PointF to, std::int64_t start_ms,
                                                const TrajectoryPolicy& policy);

    /// Where the pointer is at `now_ms`. Before the start it is at `from`; after the end,
    /// exactly at `to`.
    [[nodiscard]] PointF at(std::int64_t now_ms) const noexcept;
    [[nodiscard]] std::int64_t start_ms() const noexcept { return m_start_ms; }
    [[nodiscard]] std::int64_t end_ms() const noexcept { return m_start_ms + m_duration_ms; }
    [[nodiscard]] std::int64_t duration_ms() const noexcept { return m_duration_ms; }
    [[nodiscard]] PointF from() const noexcept { return m_from; }
    [[nodiscard]] PointF to() const noexcept { return m_to; }

private:
    PointF m_from{};
    PointF m_to{};
    PointF m_control{};
    std::int64_t m_start_ms = 0;
    std::int64_t m_duration_ms = 0;
};

/// One thing the compositor must do to an application, at a time.
enum class StepKind {
    /// Move the pointer to `point`. Emitted while a trajectory runs.
    Motion,
    Press,
    Release,
    Wheel,
    /// Deliver the character at `index` of the text the compositor holds.
    TextUnit,
    KeyPress,
    KeyRelease,
    /// Give keyboard focus to the command's surface.
    FocusSurface,
    /// Capture the command's surface.
    Capture,
};

struct InputStep {
    std::int64_t at_ms = 0;
    StepKind kind = StepKind::Motion;
    PointF point{};
    PointerButton button = PointerButton::Left;
    int wheel_x = 0;
    int wheel_y = 0;
    /// Character index for `TextUnit`, key index within the chord for key steps.
    std::size_t index = 0;

    friend bool operator==(const InputStep&, const InputStep&) = default;
};

struct PlaybackPolicy {
    TrajectoryPolicy trajectory{};
    /// Time between press and release of a click.
    std::int64_t press_dwell_ms = 55;
    /// Time between the two clicks of a double click.
    std::int64_t double_click_gap_ms = 90;
    /// Time between wheel steps.
    std::int64_t wheel_interval_ms = 24;
    /// Time between typed characters.
    std::int64_t key_interval_ms = 18;
    /// Time between giving a surface keyboard focus and its first key. An application takes
    /// focus asynchronously, and turns on its input method only once it has it, so a first
    /// character sent at once can arrive before there is anywhere for it to go.
    std::int64_t focus_settle_ms = 120;
    /// Hold before a drag starts moving, so the application registers the press.
    std::int64_t drag_hold_ms = 80;
    /// How long a capture stays on screen as an action before its receipt: long enough for
    /// a person watching to see that the agent looked, and for the frame to be rendered.
    std::int64_t capture_hold_ms = 450;

    /// The tempo of an agent at work, rather than one demonstrating itself: the pointer
    /// covers a path in a quarter of the time, a press lasts two frames, and a capture
    /// does not hold the stage at all. Every movement is still drawn, and nothing about
    /// what is checked, sealed or refused changes with it.
    [[nodiscard]] static PlaybackPolicy agent_tempo() noexcept;
};

enum class PlaybackState {
    Running,
    Finished,
    Cancelled,
};

/// The schedule of input steps for one checked command.
///
/// Built once from the surface as it is when the command starts. The compositor asks it
/// for the steps due at each frame; motion is sampled at those frames.
class Playback final {
public:
    [[nodiscard]] static Playback start(const StageCommand& command, const Surface& surface,
                                        PointF pointer_now, std::int64_t now_ms,
                                        const PlaybackPolicy& policy);

    /// Every step due by `now_ms` that has not been emitted yet, in order. While the
    /// pointer is travelling this includes one `Motion` step at the current position.
    [[nodiscard]] std::vector<InputStep> due(std::int64_t now_ms);

    /// Stop now. Returns the steps that leave the application in a clean state: a
    /// release for every button still held and every key still down.
    [[nodiscard]] std::vector<InputStep> cancel(std::int64_t now_ms);

    [[nodiscard]] PlaybackState state() const noexcept { return m_state; }
    [[nodiscard]] const StageCommand& command() const noexcept { return m_command; }
    /// The pointer's current position under this playback.
    [[nodiscard]] PointF pointer_at(std::int64_t now_ms) const noexcept;
    [[nodiscard]] std::optional<PointerTrajectory> trajectory_at(std::int64_t now_ms) const noexcept;
    [[nodiscard]] std::int64_t started_ms() const noexcept { return m_started_ms; }
    [[nodiscard]] std::int64_t ends_ms() const noexcept { return m_ends_ms; }
    /// Whether a pointer button is held at `now_ms` by steps already emitted.
    [[nodiscard]] bool button_held() const noexcept { return m_button_held; }
    /// The whole schedule, for tests and for drawing.
    [[nodiscard]] const std::vector<InputStep>& schedule() const noexcept { return m_steps; }

private:
    StageCommand m_command{};
    std::vector<PointerTrajectory> m_trajectories;
    std::vector<InputStep> m_steps;
    std::size_t m_next = 0;
    std::int64_t m_started_ms = 0;
    std::int64_t m_ends_ms = 0;
    PointF m_last_motion{};
    bool m_emitted_motion = false;
    PointF m_rest{};
    PlaybackState m_state = PlaybackState::Running;
    bool m_button_held = false;
    PointerButton m_held_button = PointerButton::Left;
    std::vector<std::size_t> m_keys_down;
};

}  // namespace archie::visual
