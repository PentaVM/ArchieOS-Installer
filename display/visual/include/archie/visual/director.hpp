#pragma once

// Cinematics around whatever application the agent is using.
//
// The stage hosts applications this code has never heard of: a terminal, a CAD package, a
// browser, something installed yesterday. So nothing here knows what an application is.
// It knows rectangles, a pointer path, the moments a button goes down, how many
// characters are being typed, and who holds the input. From those alone it decides where
// the camera looks, where the agent's cursor is drawn, which clicks ripple, and what one
// line of caption says — the same decisions for every application, which is the only way
// "cinematic" can apply to software nobody wrote a scene for.
//
// Four lies shape it.
//
// The first is the camera that chases the cursor. A frame that re-aims on every small
// move is unwatchable, and it hides the thing a viewer needs most: where the pointer went
// relative to where it was. The camera frames the region an action spans, and it only
// re-frames when the next action would leave that frame. Short, precise actions get a
// closer shot and long moves a wider one, because that is what their paths require.
//
// The second is the cinematic that fights the operator. The moment a person takes over,
// the camera returns to the whole stage and stops following anything; a zoomed frame
// under someone's own hand is a screen that has stopped being theirs. The pause and the
// takeover are announced in the frame, never only in a log.
//
// The third is the caption that leaks. A line that says what was typed puts the text on a
// shared screen. Captions carry a verb, an application identifier and a character count,
// and there is no field that could hold anything else. The activity strip, which lets a
// person who looks up see the last few things the agent did, is made of the same captions.
//
// The fourth, as everywhere in this library, is motion that costs nothing to show and
// something to draw. Every frame says when the next one is needed: the tier's refresh
// while something moves, a single frame at the next deadline when the stage is still,
// and nothing at all when nothing will change.
//
// No clock, no Qt, no socket. Time is a parameter.

#include <cstddef>
#include <cstdint>
#include <deque>
#include <optional>
#include <string>
#include <vector>

#include "archie/visual/budget.hpp"
#include "archie/visual/stage.hpp"

namespace archie::visual {

/// What the camera shows: a centre on the output and a magnification of at least 1.
struct ViewRect {
    float cx = 0.0F;
    float cy = 0.0F;
    float zoom = 1.0F;

    friend bool operator==(const ViewRect&, const ViewRect&) = default;
};

/// The part of the output a view shows, as a floating rectangle.
struct VisibleRegion {
    float x = 0.0F;
    float y = 0.0F;
    float w = 0.0F;
    float h = 0.0F;
};

[[nodiscard]] VisibleRegion visible_region(const ViewRect& view, int output_w, int output_h) noexcept;

/// Where an output point lands on screen under a view.
[[nodiscard]] PointF to_screen(const ViewRect& view, int output_w, int output_h, PointF point) noexcept;

struct DirectorPolicy {
    int output_w = 1920;
    int output_h = 1080;
    /// Never magnify beyond this. Past it, pixels start to show and the viewer loses the
    /// application around the action.
    float max_zoom = 1.8F;
    /// Room kept around an action's path so the destination is never at the frame's edge.
    float action_margin_px = 180.0F;
    /// A camera move between framings.
    std::int64_t move_ms = 650;
    /// Returning to the whole stage when a person takes over. Fast, because they are
    /// already reaching for it.
    std::int64_t release_ms = 260;
    /// Quiet time after the last action before the camera goes back to the whole stage.
    std::int64_t settle_ms = 2'600;
    /// The camera keeps its framing for a new action whose path already fits inside the
    /// frame, unless that action needs a closer shot than this relative margin.
    float reframe_zoom_tolerance = 0.15F;
    std::int64_t ripple_ms = 620;
    float ripple_radius_px = 34.0F;
    /// How long the agent's cursor stays drawn after it last acted.
    std::int64_t cursor_linger_ms = 3'500;
    std::int64_t cursor_fade_ms = 400;
    /// How long a caption stays after its action ends.
    std::int64_t caption_hold_ms = 1'400;
    std::int64_t fade_ms = 200;
    /// How long a finished, stopped or refused action stays in the activity strip.
    std::int64_t activity_ms = 20'000;
    /// Accessibility: framing changes cut, clicks do not ripple, nothing fades. The
    /// agent's cursor still moves, because where the agent is pointing is information.
    bool reduced_motion = false;
};

enum class CaptionVerb {
    Moving,
    Clicking,
    DoubleClicking,
    Dragging,
    Scrolling,
    Typing,
    PressingKeys,
    Focusing,
    /// The agent drew a line: pressed, followed a way through the window, and let go.
    Drawing,
    /// The agent captured what a window shows.
    Observing,
    /// A person took over mid-action and the action was stopped.
    Interrupted,
    /// The stage refused the command.
    Refused,
};

/// One line about what the agent is doing. There is no text field, on purpose.
struct Caption {
    CaptionVerb verb = CaptionVerb::Moving;
    std::string app_id;
    std::size_t text_chars = 0;
    int key_count = 0;
    std::optional<CommandRefusal> refusal{};
    float opacity = 1.0F;
};

struct Ripple {
    PointF center{};
    float radius = 0.0F;
    float opacity = 0.0F;
};

inline constexpr std::size_t kMaxRipples = 8;
inline constexpr std::size_t kMaxActivity = 5;

/// Everything a renderer needs, already decided.
struct StageFrame {
    ViewRect view{};
    bool camera_moving = false;

    bool cursor_visible = false;
    PointF cursor{};
    float cursor_opacity = 0.0F;
    bool cursor_pressed = false;
    /// Where a drag was picked up, while it is being carried.
    std::optional<PointF> drag_from{};

    std::vector<Ripple> ripples;
    std::optional<Caption> caption{};
    /// What the agent did lately, newest first, at most `kMaxActivity`: each ended or
    /// refused action's caption as it stood at the end, fading out after `activity_ms`.
    std::vector<Caption> activity;

    /// Present whenever the agent does not hold the stage.
    std::optional<AuthorityMode> banner{};
    float banner_opacity = 0.0F;

    bool animate = false;
    /// When the stage next looks different. Absent means: nothing until new input.
    std::optional<std::int64_t> next_frame_at_ms{};
};

/// Decides the cinematic treatment of a stage. Holds the last action and when things
/// changed; derives every frame from those alone.
class StageDirector final {
public:
    StageDirector();
    explicit StageDirector(DirectorPolicy policy);

    void set_output(int width, int height, std::int64_t now_ms);

    /// The stage accepted a command and started playing it back.
    void command_started(const StageCommand& command, const Surface& surface,
                         const Playback& playback, std::int64_t now_ms);
    /// The playback ended, either by finishing or by being cancelled.
    void command_ended(PlaybackState outcome, std::int64_t now_ms);
    /// The stage refused a command before playing any of it.
    void command_refused(const StageCommand& command, CommandRefusal refusal, std::int64_t now_ms);
    /// The stage stopped a running playback it could not finish delivering — a character
    /// the keyboard cannot type, say. It ends like a cancellation, but the caption gives the
    /// reason instead of suggesting that a person stepped in.
    void command_failed(CommandRefusal refusal, std::int64_t now_ms);
    /// A framed surface moved or resized.
    void surface_moved(const Surface& surface, std::int64_t now_ms);
    void authority_changed(AuthorityMode mode, std::int64_t now_ms);

    /// Apply the deadlines that pass without any input: the camera going home.
    void tick(std::int64_t now_ms);

    /// The frame for `now_ms` under `tier`. Pure: the same inputs give the same frame.
    [[nodiscard]] StageFrame frame(std::int64_t now_ms, RenderTier tier) const;

    [[nodiscard]] ViewRect camera_at(std::int64_t now_ms) const noexcept;
    [[nodiscard]] ViewRect camera_target() const noexcept { return m_to; }
    [[nodiscard]] const DirectorPolicy& policy() const noexcept { return m_policy; }

private:
    [[nodiscard]] ViewRect overview() const noexcept;
    [[nodiscard]] ViewRect frame_region(float x0, float y0, float x1, float y1) const noexcept;
    /// Whether the current framing already serves an action spanning this region.
    [[nodiscard]] bool framing_serves(float x0, float y0, float x1, float y1,
                                      const ViewRect& wanted) const noexcept;
    void aim(const ViewRect& target, std::int64_t duration_ms, std::int64_t now_ms);
    [[nodiscard]] bool camera_moving_at(std::int64_t now_ms) const noexcept;
    void end_playback(PlaybackState outcome, std::optional<CommandRefusal> failure, std::int64_t now_ms);
    void remember(const Caption& caption, std::int64_t now_ms);

    DirectorPolicy m_policy{};

    ViewRect m_from{};
    ViewRect m_to{};
    std::int64_t m_move_since_ms = 0;
    std::int64_t m_move_ms = 0;

    AuthorityMode m_mode = AuthorityMode::Autonomous;
    std::int64_t m_mode_since_ms = 0;

    std::optional<Playback> m_playback{};
    /// The drawn travel of the last command whose input went ahead of it.
    std::vector<PointerTrajectory> m_glides{};
    std::optional<SurfaceId> m_framed_surface{};
    std::int64_t m_last_action_end_ms = 0;
    bool m_following = false;

    std::optional<PointF> m_last_cursor{};
    std::int64_t m_cursor_until_ms = 0;

    std::optional<Caption> m_caption{};
    std::int64_t m_caption_since_ms = 0;
    std::optional<std::int64_t> m_caption_until_ms{};

    struct Press {
        PointF at{};
        std::int64_t at_ms = 0;
    };
    std::deque<Press> m_presses;

    struct Done {
        Caption what{};
        std::int64_t at_ms = 0;
    };
    /// Newest first.
    std::deque<Done> m_activity;
};

}  // namespace archie::visual
