#include "KeyTyper.h"

#include <QMap>
#include <QWaylandClient>
#include <QWaylandInputMethodControl>
#include <QWaylandKeyboard>
#include <QWaylandSurface>
#include <QtWaylandCompositor/private/qwaylandkeyboard_p.h>

#include <xkbcommon/xkbcommon.h>

#include <algorithm>
#include <array>
#include <memory>

namespace {

using ScratchState = std::unique_ptr<xkb_state, decltype(&xkb_state_unref)>;

/// Keypad keys produce digits and operators too, but a terminal in application keypad
/// mode sends something else for them. The main block is what text means.
bool is_keypad(xkb_keysym_t sym)
{
    return sym >= XKB_KEY_KP_Space && sym <= XKB_KEY_KP_Equal;
}

/// Keys whose whole job is to select a level. Control, Alt and Super are deliberately not
/// among them: an application reads those as commands, never as part of a character.
bool is_level_key(xkb_keysym_t sym)
{
    return sym == XKB_KEY_Shift_L || sym == XKB_KEY_Shift_R || sym == XKB_KEY_ISO_Level3_Shift ||
           sym == XKB_KEY_ISO_Level5_Shift;
}

QWaylandKeyboardPrivate *keyboard_of(QWaylandSeat *seat)
{
    QWaylandKeyboard *keyboard = seat ? seat->keyboard() : nullptr;
    return keyboard ? QWaylandKeyboardPrivate::get(keyboard) : nullptr;
}

} // namespace

KeyTyper::KeyTyper(QWaylandSeat *seat)
    : m_seat(seat)
{
}

KeyTyper::~KeyTyper()
{
    releaseAll();
    if (m_keymap) {
        xkb_keymap_unref(m_keymap);
    }
}

bool KeyTyper::refresh()
{
    QWaylandKeyboardPrivate *keyboard = keyboard_of(m_seat);
    xkb_state *live = keyboard ? keyboard->xkbState() : nullptr;
    xkb_keymap *current = live ? xkb_state_get_keymap(live) : nullptr;
    if (!current) {
        return false;
    }
    const xkb_layout_index_t layout = xkb_state_serialize_layout(live, XKB_STATE_LAYOUT_EFFECTIVE);
    if (current != m_keymap || layout != m_layout) {
        // The reference keeps a replaced keymap's address from being reused under the cache.
        if (m_keymap) {
            xkb_keymap_unref(m_keymap);
        }
        m_keymap = xkb_keymap_ref(current);
        m_layout = layout;
        rebuild();
    }
    return true;
}

void KeyTyper::rebuild()
{
    m_strokes.clear();
    ScratchState scratch(xkb_state_new(m_keymap), &xkb_state_unref);
    if (!scratch) {
        return;
    }
    xkb_state_update_mask(scratch.get(), 0, 0, 0, 0, 0, m_layout);
    const std::uint64_t first = xkb_keymap_min_keycode(m_keymap);
    const std::uint64_t last = xkb_keymap_max_keycode(m_keymap);

    // Which modifier each level key sets, learned by pressing it on a scratch state.
    QMap<xkb_mod_mask_t, xkb_keycode_t> level_keys;
    for (std::uint64_t c = first; c <= last; ++c) {
        const auto code = static_cast<xkb_keycode_t>(c);
        const xkb_keysym_t *syms = nullptr;
        if (xkb_keymap_key_get_syms_by_level(m_keymap, code, m_layout, 0, &syms) != 1 || !is_level_key(syms[0])) {
            continue;
        }
        xkb_state_update_key(scratch.get(), code, XKB_KEY_DOWN);
        const xkb_mod_mask_t mask = xkb_state_serialize_mods(scratch.get(), XKB_STATE_MODS_DEPRESSED);
        xkb_state_update_key(scratch.get(), code, XKB_KEY_UP);
        if (mask != 0 && !level_keys.contains(mask)) {
            level_keys.insert(mask, code);
        }
    }

    std::array<xkb_mod_mask_t, 16> masks{};
    for (std::uint64_t c = first; c <= last; ++c) {
        const auto code = static_cast<xkb_keycode_t>(c);
        const xkb_level_index_t levels = xkb_keymap_num_levels_for_key(m_keymap, code, m_layout);
        for (xkb_level_index_t level = 0; level < levels; ++level) {
            const xkb_keysym_t *syms = nullptr;
            if (xkb_keymap_key_get_syms_by_level(m_keymap, code, m_layout, level, &syms) != 1 || is_keypad(syms[0])) {
                continue;
            }
            const char32_t unit = xkb_keysym_to_utf32(syms[0]);
            if (unit < 0x20 || unit == 0x7f) {
                continue; // not a character, or a control character
            }
            const std::size_t count =
                xkb_keymap_key_get_mods_for_level(m_keymap, code, m_layout, level, masks.data(), masks.size());
            for (std::size_t i = 0; i < count && i < masks.size(); ++i) {
                QList<std::uint32_t> held;
                if (masks[i] != 0) {
                    const auto whole = level_keys.constFind(masks[i]);
                    if (whole != level_keys.constEnd()) {
                        held.append(*whole);
                    } else {
                        xkb_mod_mask_t rest = masks[i];
                        for (auto it = level_keys.constBegin(); it != level_keys.constEnd() && rest != 0; ++it) {
                            if ((it.key() & rest) == it.key()) {
                                held.append(it.value());
                                rest &= ~it.key();
                            }
                        }
                        if (rest != 0) {
                            continue; // needs Lock, NumLock or a command modifier: not a way to type
                        }
                    }
                }
                // Confirm the stroke on the scratch state instead of trusting the arithmetic.
                for (const std::uint32_t k : held) {
                    xkb_state_update_key(scratch.get(), k, XKB_KEY_DOWN);
                }
                const char32_t produced = xkb_state_key_get_utf32(scratch.get(), code);
                for (auto k = held.crbegin(); k != held.crend(); ++k) {
                    xkb_state_update_key(scratch.get(), *k, XKB_KEY_UP);
                }
                if (produced != unit) {
                    continue;
                }
                const auto known = m_strokes.constFind(unit);
                if (known == m_strokes.constEnd() || held.size() < known->levels.size()) {
                    m_strokes.insert(unit, Stroke{code, held});
                }
                break;
            }
        }
    }
}

const KeyTyper::Stroke *KeyTyper::strokeFor(char32_t unit)
{
    if (!refresh()) {
        return nullptr;
    }
    const auto found = m_strokes.constFind(unit);
    return found == m_strokes.constEnd() ? nullptr : &found.value();
}

bool KeyTyper::textInputEnabled(QWaylandSurface *surface) const
{
    if (!surface || !surface->client() ||
        surface->client()->textInputProtocols() == QWaylandClient::TextInputProtocols()) {
        return false;
    }
    // The control class is not exported from the library, only its meta-object is reachable,
    // so its `enabled` property is read the way QML would read it.
    const QObject *control = surface->inputMethodControl();
    return control && control->property("enabled").toBool();
}

bool KeyTyper::mayType(char32_t unit, QWaylandSurface *surface)
{
    if (strokeFor(unit)) {
        return true;
    }
    return surface && surface->client() &&
           surface->client()->textInputProtocols() != QWaylandClient::TextInputProtocols();
}

std::optional<KeyTyper::Route> KeyTyper::routeFor(const QList<char32_t> &text)
{
    QWaylandSurface *focus = m_seat ? m_seat->keyboardFocus() : nullptr;
    if (!focus) {
        return std::nullopt;
    }
    const bool every_key = std::all_of(text.cbegin(), text.cend(),
                                       [this](char32_t unit) { return strokeFor(unit) != nullptr; });
    if (every_key) {
        return Route::Keys;
    }
    if (textInputEnabled(focus)) {
        return Route::TextInput;
    }
    return std::nullopt;
}

bool KeyTyper::type(char32_t unit, Route route)
{
    QWaylandSurface *focus = m_seat ? m_seat->keyboardFocus() : nullptr;
    if (!focus) {
        return false;
    }
    if (route == Route::TextInput) {
        if (!textInputEnabled(focus)) {
            return false;
        }
        m_seat->sendUnicodeKeyPressEvent(unit);
        m_seat->sendUnicodeKeyReleaseEvent(unit);
        return true;
    }
    const Stroke *found = strokeFor(unit);
    if (!found) {
        return false;
    }
    const Stroke stroke = *found; // a press can land a new keymap and rebuild the table
    for (const std::uint32_t k : stroke.levels) {
        press(k, true);
    }
    press(stroke.keycode, true);
    press(stroke.keycode, false);
    for (auto k = stroke.levels.crbegin(); k != stroke.levels.crend(); ++k) {
        press(*k, false);
    }
    return true;
}

bool KeyTyper::hasKey(int qtKey)
{
    return m_seat && m_seat->keyboard() && m_seat->keyboard()->keyToScanCode(qtKey) != 0;
}

bool KeyTyper::key(int qtKey, bool pressed)
{
    if (!m_seat || !m_seat->keyboard() || !m_seat->keyboardFocus()) {
        return false;
    }
    const uint code = m_seat->keyboard()->keyToScanCode(qtKey);
    if (code == 0) {
        return false;
    }
    if (!pressed && !m_held.contains(code)) {
        return true; // nothing of ours to release
    }
    press(code, pressed);
    return true;
}

void KeyTyper::releaseAll()
{
    while (!m_held.isEmpty()) {
        press(m_held.constLast(), false);
    }
}

void KeyTyper::press(std::uint32_t keycode, bool pressed)
{
    if (pressed) {
        m_held.append(keycode);
    } else {
        m_held.removeOne(keycode);
    }
    QWaylandKeyboardPrivate *keyboard = keyboard_of(m_seat);
    if (!keyboard) {
        return;
    }
    const uint32_t state = pressed ? QtWaylandServer::wl_keyboard::key_state_pressed
                                   : QtWaylandServer::wl_keyboard::key_state_released;
    // The order Qt follows for a physical key: record it, send it, let a pending keymap land
    // once no key is down, then move the modifier state and tell the application.
    keyboard->keyEvent(keycode, state);
    keyboard->sendKeyEvent(keycode, state);
    keyboard->maybeUpdateKeymap();
    keyboard->updateModifierState(keycode, state);
}
