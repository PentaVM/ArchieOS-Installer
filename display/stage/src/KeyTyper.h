#pragma once

// Typing on the stage's keyboard.
//
// An application reads the agent's text the way it reads a person's: as key events
// interpreted through the keymap the compositor handed it. So the agent types by pressing
// the keys that produce each character on that keymap, holding Shift or AltGr where the
// character sits on a higher level, and the seat's modifier state moves exactly as it
// would under a person's hands.
//
// A text with a character the keymap cannot produce goes, whole, through the application's
// text-input protocol when the application has enabled one. Otherwise it is not typed at
// all: the command is refused before its first character, and the receipt says
// `not_typeable` instead of claiming it ran.
//
// Qt's public seat API cannot do this. `QWaylandSeat::sendKeyEvent` sends a key without
// touching modifier state, so Ctrl+C reaches the application as a plain "c", and
// `sendUnicodeKeyPressEvent` silently does nothing for an application without text-input.
// This uses the private keyboard entry points that Qt's own handling of a physical key
// uses, in the same order. Those are version-bound; the build pins the Qt it compiled
// against, and a Qt that moves them fails to compile rather than to type.

#include <QHash>
#include <QList>
#include <QPointer>
#include <QWaylandSeat>

#include <cstdint>
#include <optional>

class QWaylandSurface;
struct xkb_keymap;

class KeyTyper final
{
public:
    /// How a text reaches an application. One route carries a whole text: an application
    /// reads key events and input-method events through different queues, so a text that
    /// mixed them could arrive reordered ("ñanúd" for "ñandú").
    enum class Route { Keys, TextInput };

    explicit KeyTyper(QWaylandSeat *seat);
    ~KeyTyper();
    KeyTyper(const KeyTyper &) = delete;
    KeyTyper &operator=(const KeyTyper &) = delete;

    /// Whether `unit` can reach `surface` at all: it is on the keymap, or the surface's
    /// application has bound a text-input protocol. Whether that protocol is enabled is only
    /// known once the surface holds keyboard focus, so `routeFor` can still decline.
    bool mayType(char32_t unit, QWaylandSurface *surface);
    /// The route for a whole text into the keyboard focus, as things stand: keys when every
    /// character has one, which is how a person types and what every application reads;
    /// otherwise the focus's input method, when it has one enabled. Nothing when neither.
    std::optional<Route> routeFor(const QList<char32_t> &text);
    /// Types one character by `route`. Returns false, having pressed nothing, when that route
    /// cannot carry it.
    bool type(char32_t unit, Route route);

    /// Whether a Qt key code has a key on the keymap.
    bool hasKey(int qtKey);
    /// Presses or releases the key for a Qt key code. Returns false, having pressed
    /// nothing, when the keymap has no such key.
    bool key(int qtKey, bool pressed);

    /// Releases every key this typer still holds.
    void releaseAll();

private:
    struct Stroke {
        std::uint32_t keycode = 0;
        /// Level keys to hold, in press order.
        QList<std::uint32_t> levels;
    };

    bool refresh();
    void rebuild();
    const Stroke *strokeFor(char32_t unit);
    bool textInputEnabled(QWaylandSurface *surface) const;
    void press(std::uint32_t keycode, bool pressed);

    QPointer<QWaylandSeat> m_seat;
    xkb_keymap *m_keymap = nullptr;
    std::uint32_t m_layout = 0;
    QHash<char32_t, Stroke> m_strokes;
    QList<std::uint32_t> m_held;
};
