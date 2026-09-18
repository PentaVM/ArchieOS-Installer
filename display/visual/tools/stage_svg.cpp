// `archie-visual-stage` — watch the agent work three applications, and count the cost.
//
// Runs a scripted session against the stage model the way the compositor drives it: a
// command is checked against the stage as it is, played back step by step, and the
// director is asked for a frame on input and at the time it names, never on a timer of
// its own. Midway, a person reaches for the mouse and the agent's action is stopped;
// later a pause chord holds the agent until it is resumed.
//
// Every frame drawn is sampled into an SVG whose SMIL animation replays exactly those
// frames: the camera framing, the agent's cursor and its path, ripples, captions and the
// takeover banners. The applications are rectangles because, to the director, that is
// all any application is — which is the point: the same treatment for software nobody
// wrote a scene for.
//
// Usage:  archie-visual-stage [out.svg] [normal|conserve|ultra] [--reduced-motion]

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <map>
#include <optional>
#include <string>
#include <vector>

#include "archie/visual/director.hpp"
#include "archie/visual/stage.hpp"

using namespace archie::visual;

namespace {

constexpr int kW = 1920;
constexpr int kH = 1080;
constexpr std::int64_t kEndMs = 16'000;

enum class EventKind { Command, OperatorReach, PauseChord, Resume };

struct Event {
    std::int64_t at_ms = 0;
    EventKind kind = EventKind::Command;
    StageAction action = StageAction::Move;
    SurfaceId surface = 0;
    int x = 0;
    int y = 0;
    int to_x = 0;
    int to_y = 0;
    std::size_t chars = 0;
    int keys = 0;
    int scroll_y = 0;
    const char* note = "";
};

Event command(std::int64_t at, StageAction action, SurfaceId surface, int x, int y, const char* note) {
    Event e;
    e.at_ms = at;
    e.kind = EventKind::Command;
    e.action = action;
    e.surface = surface;
    e.x = x;
    e.y = y;
    e.note = note;
    return e;
}

Event physical(std::int64_t at, EventKind kind, const char* note) {
    Event e;
    e.at_ms = at;
    e.kind = kind;
    e.note = note;
    return e;
}

constexpr SurfaceId kCad = 1;
constexpr SurfaceId kTerminal = 2;
constexpr SurfaceId kFiles = 3;

std::vector<Event> scenario() {
    std::vector<Event> events;
    events.push_back(command(900, StageAction::Click, kTerminal, 330, 400,
                             "Archie clicks into the terminal's prompt"));
    Event typing = command(1'700, StageAction::Text, kTerminal, 0, 0,
                           "It types a 24-character command (the text never reaches the display model)");
    typing.chars = 24;
    events.push_back(typing);
    Event enter = command(2'450, StageAction::Key, kTerminal, 0, 0, "and presses Enter");
    enter.keys = 1;
    events.push_back(enter);
    events.push_back(command(3'700, StageAction::Click, kCad, 96, 34,
                             "Across to the CAD package's toolbar: a long move, a wide shot"));
    Event drag = command(4'900, StageAction::Drag, kCad, 380, 420, "It drags a sketch edge");
    drag.to_x = 760;
    drag.to_y = 610;
    events.push_back(drag);
    Event scroll = command(6'600, StageAction::Scroll, kFiles, 340, 240, "Scrolls the file browser");
    scroll.scroll_y = -4;
    events.push_back(scroll);
    events.push_back(command(7'800, StageAction::DoubleClick, kFiles, 140, 180,
                             "Opens a file with a double click"));
    events.push_back(command(8'900, StageAction::Click, kCad, 980, 700,
                             "Starts back towards the CAD viewport..."));
    events.push_back(physical(9'150, EventKind::OperatorReach,
                              "...and a person reaches for the mouse: the move stops, the stage is theirs"));
    events.push_back(command(10'000, StageAction::Click, kCad, 500, 500,
                             "A command arriving now is refused: the operator has control"));
    events.push_back(command(12'400, StageAction::Click, kCad, 980, 700,
                             "After their quiet period the agent resumes and finishes the click"));
    events.push_back(physical(14'000, EventKind::PauseChord,
                              "The pause chord: the agent stops until a person resumes it"));
    return events;
}

const char* label_of(const std::string& app_id) {
    if (app_id == "org.freecad.FreeCAD") return "FreeCAD";
    if (app_id == "org.codeberg.dnkl.foot") return "foot";
    if (app_id == "org.gnome.Nautilus") return "Files";
    return app_id.c_str();
}

std::string caption_text(const Caption& caption) {
    const char* app = label_of(caption.app_id);
    char buffer[160];
    switch (caption.verb) {
        case CaptionVerb::Moving:
            std::snprintf(buffer, sizeof buffer, "moviendo el puntero · %s", app);
            break;
        case CaptionVerb::Clicking:
            std::snprintf(buffer, sizeof buffer, "clic · %s", app);
            break;
        case CaptionVerb::DoubleClicking:
            std::snprintf(buffer, sizeof buffer, "doble clic · %s", app);
            break;
        case CaptionVerb::Dragging:
            std::snprintf(buffer, sizeof buffer, "arrastrando · %s", app);
            break;
        case CaptionVerb::Drawing:
            std::snprintf(buffer, sizeof buffer, "dibujando · %s", app);
            break;
        case CaptionVerb::Scrolling:
            std::snprintf(buffer, sizeof buffer, "desplazando · %s", app);
            break;
        case CaptionVerb::Typing:
            std::snprintf(buffer, sizeof buffer, "escribiendo %zu caracteres · %s", caption.text_chars, app);
            break;
        case CaptionVerb::PressingKeys:
            std::snprintf(buffer, sizeof buffer, "pulsando %d tecla%s · %s", caption.key_count,
                          caption.key_count == 1 ? "" : "s", app);
            break;
        case CaptionVerb::Focusing:
            std::snprintf(buffer, sizeof buffer, "enfocando · %s", app);
            break;
        case CaptionVerb::Observing:
            std::snprintf(buffer, sizeof buffer, "mirando · %s", app);
            break;
        case CaptionVerb::Interrupted:
            std::snprintf(buffer, sizeof buffer, "acción detenida · tomaste el control");
            break;
        case CaptionVerb::Refused:
            std::snprintf(buffer, sizeof buffer, "rechazado (%s) · %s",
                          caption.refusal ? refusal_code(*caption.refusal) : "?", app);
            break;
    }
    return buffer;
}

struct Sample {
    std::int64_t at_ms = 0;
    StageFrame frame;
    AuthorityMode mode = AuthorityMode::Autonomous;
};

struct Run {
    std::vector<Sample> samples;
    int frames = 0;
    int refused = 0;
    int preempted = 0;
};

Run simulate(const std::vector<Event>& events, SurfaceTable& table, const DirectorPolicy& director_policy,
             RenderTier tier) {
    AuthorityPolicy authority_policy;
    authority_policy.operator_idle_resume_ms = 2'500;
    InputAuthority authority(authority_policy);
    StageDirector director(director_policy);
    PlaybackPolicy playback_policy;

    Run run;
    std::optional<Playback> playback;
    PointF pointer{960.0F, 540.0F};
    std::uint64_t sequence = 0;
    std::uint64_t last_sequence = 0;
    std::size_t next_event = 0;
    AuthorityMode mode = authority.mode();

    const auto follow_authority = [&](std::int64_t now) {
        if (authority.mode() == mode) {
            return;
        }
        mode = authority.mode();
        director.authority_changed(mode, now);
        if (mode != AuthorityMode::Autonomous && playback.has_value()) {
            (void)playback->cancel(now);
            director.command_ended(PlaybackState::Cancelled, now);
            pointer = playback->pointer_at(now);
            playback.reset();
            ++run.preempted;
        }
    };

    std::int64_t now = 0;
    while (now <= kEndMs) {
        while (next_event < events.size() && events[next_event].at_ms <= now) {
            const Event& e = events[next_event++];
            switch (e.kind) {
                case EventKind::Command: {
                    const Surface* surface = table.find(e.surface);
                    StageCommand c;
                    c.sequence = ++sequence;
                    c.expires_at_ms = now + 2'000;
                    c.action = e.action;
                    c.surface = e.surface;
                    c.app_id = surface != nullptr ? surface->app_id : "";
                    c.geometry_revision = surface != nullptr ? surface->geometry_revision : 0;
                    c.local_x = e.x;
                    c.local_y = e.y;
                    c.to_local_x = e.to_x;
                    c.to_local_y = e.to_y;
                    c.text_chars = e.chars;
                    c.key_count = e.keys;
                    c.scroll_steps_y = e.scroll_y;
                    if (playback.has_value()) {
                        std::fprintf(stderr, "scenario overlaps a running command at %lld ms\n",
                                     static_cast<long long>(now));
                    }
                    if (const auto refusal = check_command(c, table, authority, last_sequence, now)) {
                        director.command_refused(c, *refusal, now);
                        ++run.refused;
                        break;
                    }
                    last_sequence = c.sequence;
                    playback = Playback::start(c, *surface, pointer, now, playback_policy);
                    director.command_started(c, *surface, *playback, now);
                    break;
                }
                case EventKind::OperatorReach:
                    authority.physical_motion(36.0F, 12.0F, now);
                    authority.physical_motion(40.0F, 10.0F, now);
                    break;
                case EventKind::PauseChord:
                    authority.pause_chord(now);
                    break;
                case EventKind::Resume:
                    authority.resume(now);
                    break;
            }
            follow_authority(now);
        }

        if (playback.has_value()) {
            for (const auto& step : playback->due(now)) {
                if (step.kind == StepKind::Motion) {
                    pointer = step.point;
                }
            }
            if (playback->state() == PlaybackState::Finished) {
                director.command_ended(PlaybackState::Finished, now);
                playback.reset();
            }
        }
        authority.tick(now);
        follow_authority(now);
        director.tick(now);

        Sample sample;
        sample.at_ms = now;
        sample.frame = director.frame(now, tier);
        sample.mode = authority.mode();
        run.samples.push_back(sample);
        ++run.frames;

        std::optional<std::int64_t> wake = sample.frame.next_frame_at_ms;
        const auto earliest = [&wake, now](std::int64_t at) {
            if (at > now && (!wake.has_value() || at < *wake)) {
                wake = at;
            }
        };
        if (next_event < events.size()) earliest(events[next_event].at_ms);
        if (playback.has_value()) {
            for (const auto& step : playback->schedule()) {
                earliest(step.at_ms);
            }
        }
        if (const auto resume = authority.resumes_at_ms()) earliest(*resume);
        if (!wake.has_value()) {
            break;
        }
        now = *wake;
    }
    return run;
}

std::string fmt(double value) {
    char buffer[32];
    std::snprintf(buffer, sizeof buffer, "%.1f", value);
    std::string s = buffer;
    if (s.size() > 2 && s.compare(s.size() - 2, 2, ".0") == 0) {
        s.resize(s.size() - 2);
    }
    return s;
}

/// A discrete SMIL track that only records values that change.
struct Track {
    std::vector<std::pair<std::int64_t, std::string>> keys;
    void set(std::int64_t at, std::string value) {
        if (!keys.empty() && keys.back().second == value) {
            return;
        }
        keys.push_back({at, std::move(value)});
    }
    [[nodiscard]] std::string animate(const char* attribute, std::int64_t end_ms) const {
        if (keys.empty()) {
            return {};
        }
        std::string values;
        std::string times;
        for (std::size_t i = 0; i < keys.size(); ++i) {
            if (i > 0) {
                values += ';';
                times += ';';
            }
            values += keys[i].second;
            char t[24];
            std::snprintf(t, sizeof t, "%.5f",
                          std::min(1.0, static_cast<double>(keys[i].first) / static_cast<double>(end_ms)));
            times += i == 0 ? "0" : t;
        }
        char dur[32];
        std::snprintf(dur, sizeof dur, "%.3fs", static_cast<double>(end_ms) / 1000.0);
        return std::string("<animate attributeName=\"") + attribute + "\" values=\"" + values +
               "\" keyTimes=\"" + times + "\" dur=\"" + dur +
               "\" calcMode=\"discrete\" repeatCount=\"indefinite\"/>";
    }
};

void window(std::FILE* out, const Surface& s, const char* title, const char* accent) {
    const auto& g = s.geometry;
    std::fprintf(out,
                 "<g><rect x=\"%d\" y=\"%d\" width=\"%d\" height=\"%d\" rx=\"10\" fill=\"#0d141d\" stroke=\"#2a3a4b\"/>"
                 "<rect x=\"%d\" y=\"%d\" width=\"%d\" height=\"34\" rx=\"10\" fill=\"#131d29\"/>"
                 "<circle cx=\"%d\" cy=\"%d\" r=\"5\" fill=\"%s\"/>"
                 "<text x=\"%d\" y=\"%d\" fill=\"#93a2b8\" font-family=\"monospace\" font-size=\"15\">%s · %s</text>",
                 g.x, g.y, g.w, g.h, g.x, g.y, g.w, g.x + 18, g.y + 17, accent, g.x + 32, g.y + 22, title,
                 s.app_id.c_str());
    for (int line = 0; line < (g.h - 70) / 34; ++line) {
        const int width = 120 + ((line * 173 + static_cast<int>(s.id) * 97) % std::max(1, g.w - 220));
        std::fprintf(out, "<rect x=\"%d\" y=\"%d\" width=\"%d\" height=\"9\" rx=\"4\" fill=\"#1a2633\"/>",
                     g.x + 24, g.y + 58 + line * 34, std::min(width, g.w - 48));
    }
    std::fprintf(out, "</g>");
}

void write_svg(const char* path, const Run& run, const SurfaceTable& table) {
    std::FILE* out = std::fopen(path, "w");
    if (out == nullptr) {
        std::fprintf(stderr, "cannot write %s\n", path);
        return;
    }
    const std::int64_t end = std::max<std::int64_t>(kEndMs, run.samples.empty() ? 1 : run.samples.back().at_ms + 1);

    Track view_box;
    Track cursor_move;
    Track cursor_opacity;
    Track halo;
    Track drag_opacity;
    Track drag_x1;
    Track drag_y1;
    Track drag_x2;
    Track drag_y2;
    Track banner_operator;
    Track banner_paused;
    std::map<std::string, Track> captions;
    std::map<std::string, std::pair<Track, Track>> ripples;  // radius, opacity

    for (const auto& sample : run.samples) {
        const StageFrame& f = sample.frame;
        const VisibleRegion r = visible_region(f.view, kW, kH);
        view_box.set(sample.at_ms, fmt(r.x) + " " + fmt(r.y) + " " + fmt(r.w) + " " + fmt(r.h));
        cursor_move.set(sample.at_ms, "translate(" + fmt(f.cursor.x) + " " + fmt(f.cursor.y) + ")");
        cursor_opacity.set(sample.at_ms, f.cursor_visible ? fmt(f.cursor_opacity) : "0");
        halo.set(sample.at_ms, f.cursor_visible && f.cursor_pressed ? "0.9" : "0");
        drag_opacity.set(sample.at_ms, f.drag_from ? "0.8" : "0");
        if (f.drag_from) {
            drag_x1.set(sample.at_ms, fmt(f.drag_from->x));
            drag_y1.set(sample.at_ms, fmt(f.drag_from->y));
            drag_x2.set(sample.at_ms, fmt(f.cursor.x));
            drag_y2.set(sample.at_ms, fmt(f.cursor.y));
        }
        banner_operator.set(sample.at_ms,
                            f.banner == std::optional<AuthorityMode>{AuthorityMode::Operator} ? fmt(f.banner_opacity) : "0");
        banner_paused.set(sample.at_ms,
                          f.banner == std::optional<AuthorityMode>{AuthorityMode::Paused} ? fmt(f.banner_opacity) : "0");

        const std::string shown = f.caption ? caption_text(*f.caption) : "";
        for (auto& [text, track] : captions) {
            if (text != shown) track.set(sample.at_ms, "0");
        }
        if (!shown.empty()) {
            captions[shown].set(sample.at_ms, fmt(f.caption->opacity));
        }

        std::vector<std::string> alive;
        for (const auto& ripple : f.ripples) {
            const std::string key = fmt(ripple.center.x) + "," + fmt(ripple.center.y);
            auto& [radius, opacity] = ripples[key];
            radius.set(sample.at_ms, fmt(ripple.radius));
            opacity.set(sample.at_ms, fmt(ripple.opacity));
            alive.push_back(key);
        }
        for (auto& [key, tracks] : ripples) {
            if (std::find(alive.begin(), alive.end(), key) == alive.end()) {
                tracks.second.set(sample.at_ms, "0");
            }
        }
    }

    std::fprintf(out,
                 "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1280\" height=\"720\" viewBox=\"0 0 %d %d\">"
                 "<rect width=\"%d\" height=\"%d\" fill=\"#04070c\"/>",
                 kW, kH, kW, kH);
    std::fprintf(out, "<svg x=\"0\" y=\"0\" width=\"%d\" height=\"%d\" viewBox=\"0 0 %d %d\" preserveAspectRatio=\"xMidYMid slice\">%s",
                 kW, kH, kW, kH, view_box.animate("viewBox", end).c_str());
    std::fprintf(out, "<defs><pattern id=\"grid\" width=\"48\" height=\"48\" patternUnits=\"userSpaceOnUse\">"
                      "<path d=\"M48 0H0V48\" fill=\"none\" stroke=\"#0c1520\" stroke-width=\"1\"/></pattern></defs>"
                      "<rect width=\"%d\" height=\"%d\" fill=\"url(#grid)\"/>",
                 kW, kH);
    window(out, *table.find(kCad), "CAD", "#6fe3f5");
    window(out, *table.find(kTerminal), "Terminal", "#47d18b");
    window(out, *table.find(kFiles), "Archivos", "#f0b429");

    for (const auto& [key, tracks] : ripples) {
        const auto comma = key.find(',');
        std::fprintf(out, "<circle cx=\"%s\" cy=\"%s\" r=\"0\" fill=\"none\" stroke=\"#6fe3f5\" stroke-width=\"3\" opacity=\"0\">%s%s</circle>",
                     key.substr(0, comma).c_str(), key.substr(comma + 1).c_str(),
                     tracks.first.animate("r", end).c_str(), tracks.second.animate("opacity", end).c_str());
    }
    std::fprintf(out, "<line stroke=\"#6fe3f5\" stroke-width=\"2\" stroke-dasharray=\"8 6\" opacity=\"0\">%s%s%s%s%s</line>",
                 drag_opacity.animate("opacity", end).c_str(), drag_x1.animate("x1", end).c_str(),
                 drag_y1.animate("y1", end).c_str(), drag_x2.animate("x2", end).c_str(),
                 drag_y2.animate("y2", end).c_str());
    // The cursor's position as an animateTransform: SMIL cannot animate the transform
    // attribute through <animate>. Opacity lives on the outer group, position on the inner.
    std::string translate_values;
    std::string translate_times;
    for (std::size_t i = 0; i < cursor_move.keys.size(); ++i) {
        if (i > 0) {
            translate_values += ';';
            translate_times += ';';
        }
        const std::string& v = cursor_move.keys[i].second;  // "translate(x y)"
        translate_values += v.substr(10, v.size() - 11);
        char t[24];
        std::snprintf(t, sizeof t, "%.5f",
                      std::min(1.0, static_cast<double>(cursor_move.keys[i].first) / static_cast<double>(end)));
        translate_times += i == 0 ? "0" : t;
    }
    char dur[32];
    std::snprintf(dur, sizeof dur, "%.3fs", static_cast<double>(end) / 1000.0);
    std::fprintf(out,
                 "<g opacity=\"0\">%s<g>"
                 "<animateTransform attributeName=\"transform\" type=\"translate\" values=\"%s\" keyTimes=\"%s\" dur=\"%s\" calcMode=\"discrete\" repeatCount=\"indefinite\"/>"
                 "<circle r=\"22\" fill=\"#6fe3f5\" opacity=\"0\">%s</circle>"
                 "<path d=\"M0 0 L0 30 L8 23 L14 36 L20 33 L14 21 L25 21 Z\" fill=\"#e6edf7\" stroke=\"#04070c\" stroke-width=\"2\"/>"
                 "</g></g>",
                 cursor_opacity.animate("opacity", end).c_str(), translate_values.c_str(),
                 translate_times.c_str(), dur, halo.animate("opacity", end).c_str());
    std::fprintf(out, "</svg>");

    // Screen space: never zoomed.
    std::fprintf(out, "<text x=\"36\" y=\"54\" fill=\"#e6edf7\" font-family=\"monospace\" font-size=\"22\" letter-spacing=\"4\">ARCHIE·OS STAGE</text>");
    for (const auto& [text, track] : captions) {
        std::fprintf(out,
                     "<g opacity=\"0\">%s<rect x=\"%d\" y=\"%d\" width=\"%d\" height=\"56\" rx=\"8\" fill=\"#0b1119\" stroke=\"#3c5169\"/>"
                     "<text x=\"%d\" y=\"%d\" fill=\"#e6edf7\" font-family=\"sans-serif\" font-size=\"24\" text-anchor=\"middle\">Archie · %s</text></g>",
                     track.animate("opacity", end).c_str(), kW / 2 - 420, kH - 110, 840, kW / 2, kH - 73, text.c_str());
    }
    std::fprintf(out,
                 "<g opacity=\"0\">%s<rect x=\"%d\" y=\"86\" width=\"%d\" height=\"64\" rx=\"8\" fill=\"#2a1f07\" stroke=\"#f0b429\"/>"
                 "<text x=\"%d\" y=\"128\" fill=\"#f0b429\" font-family=\"sans-serif\" font-size=\"26\" text-anchor=\"middle\">CONTROL HUMANO · Archie espera a que termines</text></g>",
                 banner_operator.animate("opacity", end).c_str(), kW / 2 - 470, 940, kW / 2);
    std::fprintf(out,
                 "<g opacity=\"0\">%s<rect x=\"%d\" y=\"86\" width=\"%d\" height=\"64\" rx=\"8\" fill=\"#2b1216\" stroke=\"#f2626b\"/>"
                 "<text x=\"%d\" y=\"128\" fill=\"#f2626b\" font-family=\"sans-serif\" font-size=\"26\" text-anchor=\"middle\">PAUSADO · Archie no actuará hasta que lo reanudes</text></g>",
                 banner_paused.animate("opacity", end).c_str(), kW / 2 - 500, 1000, kW / 2);
    std::fprintf(out, "</svg>\n");
    std::fclose(out);
}

/// One frame as a still, with every value baked in: for rasterisers that do not play SMIL.
void write_snapshot(const std::string& path, const Sample& sample, const SurfaceTable& table) {
    std::FILE* out = std::fopen(path.c_str(), "w");
    if (out == nullptr) {
        std::fprintf(stderr, "cannot write %s\n", path.c_str());
        return;
    }
    const StageFrame& f = sample.frame;
    const VisibleRegion r = visible_region(f.view, kW, kH);
    std::fprintf(out,
                 "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1280\" height=\"720\" viewBox=\"0 0 %d %d\">"
                 "<rect width=\"%d\" height=\"%d\" fill=\"#04070c\"/>",
                 kW, kH, kW, kH);
    std::fprintf(out, "<svg x=\"0\" y=\"0\" width=\"%d\" height=\"%d\" viewBox=\"%s %s %s %s\" preserveAspectRatio=\"xMidYMid slice\">",
                 kW, kH, fmt(r.x).c_str(), fmt(r.y).c_str(), fmt(r.w).c_str(), fmt(r.h).c_str());
    std::fprintf(out, "<defs><pattern id=\"grid\" width=\"48\" height=\"48\" patternUnits=\"userSpaceOnUse\">"
                      "<path d=\"M48 0H0V48\" fill=\"none\" stroke=\"#0c1520\" stroke-width=\"1\"/></pattern></defs>"
                      "<rect width=\"%d\" height=\"%d\" fill=\"url(#grid)\"/>",
                 kW, kH);
    window(out, *table.find(kCad), "CAD", "#6fe3f5");
    window(out, *table.find(kTerminal), "Terminal", "#47d18b");
    window(out, *table.find(kFiles), "Archivos", "#f0b429");
    for (const auto& ripple : f.ripples) {
        std::fprintf(out, "<circle cx=\"%s\" cy=\"%s\" r=\"%s\" fill=\"none\" stroke=\"#6fe3f5\" stroke-width=\"3\" opacity=\"%s\"/>",
                     fmt(ripple.center.x).c_str(), fmt(ripple.center.y).c_str(), fmt(ripple.radius).c_str(),
                     fmt(ripple.opacity).c_str());
    }
    if (f.drag_from) {
        std::fprintf(out, "<line x1=\"%s\" y1=\"%s\" x2=\"%s\" y2=\"%s\" stroke=\"#6fe3f5\" stroke-width=\"2\" stroke-dasharray=\"8 6\" opacity=\"0.8\"/>",
                     fmt(f.drag_from->x).c_str(), fmt(f.drag_from->y).c_str(), fmt(f.cursor.x).c_str(),
                     fmt(f.cursor.y).c_str());
    }
    if (f.cursor_visible) {
        std::fprintf(out,
                     "<g opacity=\"%s\" transform=\"translate(%s %s)\">"
                     "<circle r=\"22\" fill=\"#6fe3f5\" opacity=\"%s\"/>"
                     "<path d=\"M0 0 L0 30 L8 23 L14 36 L20 33 L14 21 L25 21 Z\" fill=\"#e6edf7\" stroke=\"#04070c\" stroke-width=\"2\"/></g>",
                     fmt(f.cursor_opacity).c_str(), fmt(f.cursor.x).c_str(), fmt(f.cursor.y).c_str(),
                     f.cursor_pressed ? "0.9" : "0");
    }
    std::fprintf(out, "</svg>");
    char clock[64];
    std::snprintf(clock, sizeof clock, "t = %.2f s · zoom %.2f", static_cast<double>(sample.at_ms) / 1000.0,
                  static_cast<double>(f.view.zoom));
    std::fprintf(out, "<text x=\"36\" y=\"54\" fill=\"#e6edf7\" font-family=\"monospace\" font-size=\"22\" letter-spacing=\"4\">ARCHIE·OS STAGE</text>"
                      "<text x=\"%d\" y=\"54\" fill=\"#5b6879\" font-family=\"monospace\" font-size=\"20\" text-anchor=\"end\">%s</text>",
                 kW - 36, clock);
    if (f.caption) {
        std::fprintf(out,
                     "<g opacity=\"%s\"><rect x=\"%d\" y=\"%d\" width=\"%d\" height=\"56\" rx=\"8\" fill=\"#0b1119\" stroke=\"#3c5169\"/>"
                     "<text x=\"%d\" y=\"%d\" fill=\"#e6edf7\" font-family=\"sans-serif\" font-size=\"24\" text-anchor=\"middle\">Archie · %s</text></g>",
                     fmt(f.caption->opacity).c_str(), kW / 2 - 420, kH - 110, 840, kW / 2, kH - 73,
                     caption_text(*f.caption).c_str());
    }
    if (f.banner == std::optional<AuthorityMode>{AuthorityMode::Operator}) {
        std::fprintf(out,
                     "<g opacity=\"%s\"><rect x=\"%d\" y=\"86\" width=\"940\" height=\"64\" rx=\"8\" fill=\"#2a1f07\" stroke=\"#f0b429\"/>"
                     "<text x=\"%d\" y=\"128\" fill=\"#f0b429\" font-family=\"sans-serif\" font-size=\"26\" text-anchor=\"middle\">CONTROL HUMANO · Archie espera a que termines</text></g>",
                     fmt(f.banner_opacity).c_str(), kW / 2 - 470, kW / 2);
    } else if (f.banner == std::optional<AuthorityMode>{AuthorityMode::Paused}) {
        std::fprintf(out,
                     "<g opacity=\"%s\"><rect x=\"%d\" y=\"86\" width=\"1000\" height=\"64\" rx=\"8\" fill=\"#2b1216\" stroke=\"#f2626b\"/>"
                     "<text x=\"%d\" y=\"128\" fill=\"#f2626b\" font-family=\"sans-serif\" font-size=\"26\" text-anchor=\"middle\">PAUSADO · Archie no actuará hasta que lo reanudes</text></g>",
                     fmt(f.banner_opacity).c_str(), kW / 2 - 500, kW / 2);
    }
    std::fprintf(out, "</svg>\n");
    std::fclose(out);
}

}  // namespace

int main(int argc, char** argv) {
    const char* path = "stage-cinematics.svg";
    const char* snapshots = nullptr;
    RenderTier tier = RenderTier::Normal;
    DirectorPolicy policy;
    for (int i = 1; i < argc; ++i) {
        if (std::strcmp(argv[i], "--snapshots") == 0 && i + 1 < argc) {
            snapshots = argv[++i];
        } else if (std::strcmp(argv[i], "conserve") == 0) {
            tier = RenderTier::Conserve;
        } else if (std::strcmp(argv[i], "ultra") == 0) {
            tier = RenderTier::Ultra;
        } else if (std::strcmp(argv[i], "normal") == 0) {
            tier = RenderTier::Normal;
        } else if (std::strcmp(argv[i], "--reduced-motion") == 0) {
            policy.reduced_motion = true;
        } else {
            path = argv[i];
        }
    }

    SurfaceTable table;
    table.map(kCad, "org.freecad.FreeCAD", IRect{48, 90, 1100, 940});
    table.map(kTerminal, "org.codeberg.dnkl.foot", IRect{1188, 90, 684, 450});
    table.map(kFiles, "org.gnome.Nautilus", IRect{1188, 580, 684, 450});

    const auto events = scenario();
    const Run run = simulate(events, table, policy, tier);
    write_svg(path, run, table);
    if (snapshots != nullptr) {
        // Moments worth looking at one by one: a ripple, typing, the long move, a carried
        // drag, the takeover, the refusal, the resumed click, the pause.
        for (const std::int64_t at : {1'720, 2'100, 4'300, 6'000, 7'500, 9'300, 10'100, 12'950, 14'400}) {
            const Sample* best = nullptr;
            for (const auto& sample : run.samples) {
                if (sample.at_ms <= at) {
                    best = &sample;
                }
            }
            if (best != nullptr) {
                char name[64];
                std::snprintf(name, sizeof name, "/stage-%05lld.svg", static_cast<long long>(at));
                write_snapshot(std::string(snapshots) + name, *best, table);
            }
        }
    }

    std::printf("scenario:\n");
    for (const auto& e : events) {
        std::printf("  %6.2f s  %s\n", static_cast<double>(e.at_ms) / 1000.0, e.note);
    }
    const int fixed = static_cast<int>(kEndMs * 60 / 1000);
    std::printf("\nframes drawn: %d over %.0f s (a fixed 60 fps loop: %d)\n", run.frames,
                static_cast<double>(kEndMs) / 1000.0, fixed);
    std::printf("commands refused: %d   actions interrupted by a person: %d\n", run.refused, run.preempted);
    std::printf("wrote %s\n", path);
    return 0;
}
