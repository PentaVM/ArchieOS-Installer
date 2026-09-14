#pragma once

// Opening a sealed stage command, the compositor's half of `archie-stage::seal`.
//
// A command line is `{"v":1,"payload":"<command JSON>","mac":"<64 hex>"}`, and `mac` is
// HMAC-SHA256 over "archie-stage/1\0" followed by the exact payload bytes. The MAC is
// checked before a single byte of the payload is parsed: an unauthenticated payload is
// not input. After that the payload must be exactly a command — no missing fields, no
// extra ones, no reserved chords, no control characters in text — and only then does it
// become an `archie::visual::StageCommand` for the stage's own checks.
//
// QtCore only, so the same code is exercised by a test that needs no display.

#include <QByteArray>
#include <QList>
#include <QString>

#include <cstdint>
#include <optional>

#include "archie/visual/stage.hpp"

namespace archie::stage {

inline constexpr int kMaxEnvelopeBytes = 32 * 1024;
inline constexpr std::int64_t kMaxCommandTtlMs = 10'000;
inline constexpr std::int64_t kClockSkewMs = 2'000;

/// A command that verified and parsed. The text and keys are held here, outside the pure
/// stage model, for exactly as long as the playback that delivers them.
struct DecodedCommand {
    archie::visual::StageCommand command;
    /// Code points to type, for a `Text` action.
    QList<char32_t> text;
    /// Qt key codes in press order — modifiers first, the key last — for a `Key` action.
    QList<int> keys;
    std::int64_t issued_at_ms = 0;
};

struct DecodeResult {
    std::optional<DecodedCommand> command;
    /// A stable code when `command` is empty. Never contains payload content.
    QString error;
};

/// Verify and parse one line. `now_ms` is wall-clock time, the same clock the control
/// plane stamped the command with.
[[nodiscard]] DecodeResult decode_command(const QByteArray& line, const QByteArray& key,
                                          std::int64_t now_ms);

/// Parse a key file: 64 hex characters, surrounding whitespace allowed.
[[nodiscard]] std::optional<QByteArray> parse_key(const QByteArray& file_contents);

}  // namespace archie::stage
