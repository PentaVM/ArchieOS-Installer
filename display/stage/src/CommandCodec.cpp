#include "CommandCodec.h"

#include <QCryptographicHash>
#include <QHash>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QMessageAuthenticationCode>
#include <QSet>
#include <QStringList>

#include <cmath>

namespace archie::stage {

namespace {

using archie::visual::PointerButton;
using archie::visual::StageAction;
using archie::visual::StageCommand;

constexpr double kMaxExactInteger = 9007199254740992.0;  // 2^53
constexpr int kMaxCoordinate = 32'767;

DecodeResult fail(const char* code) {
    DecodeResult result;
    result.error = QString::fromLatin1(code);
    return result;
}

bool exact_keys(const QJsonObject& object, std::initializer_list<const char*> expected) {
    if (object.size() != static_cast<qsizetype>(expected.size())) {
        return false;
    }
    for (const char* key : expected) {
        if (!object.contains(QLatin1String(key))) {
            return false;
        }
    }
    return true;
}

std::optional<std::int64_t> integer(const QJsonValue& value, double min, double max) {
    if (!value.isDouble()) {
        return std::nullopt;
    }
    const double d = value.toDouble();
    if (!std::isfinite(d) || std::floor(d) != d || d < min || d > max) {
        return std::nullopt;
    }
    return static_cast<std::int64_t>(d);
}

std::optional<int> coordinate(const QJsonValue& value) {
    const auto v = integer(value, 0, kMaxCoordinate);
    return v ? std::optional<int>(static_cast<int>(*v)) : std::nullopt;
}

std::optional<PointerButton> button(const QJsonValue& value) {
    const QString name = value.toString();
    if (name == QLatin1String("left")) return PointerButton::Left;
    if (name == QLatin1String("middle")) return PointerButton::Middle;
    if (name == QLatin1String("right")) return PointerButton::Right;
    return std::nullopt;
}

bool constant_time_equal(const QByteArray& a, const QByteArray& b) {
    if (a.size() != b.size()) {
        return false;
    }
    unsigned char diff = 0;
    for (qsizetype i = 0; i < a.size(); ++i) {
        diff |= static_cast<unsigned char>(a[i] ^ b[i]);
    }
    return diff == 0;
}

bool is_bidi_or_control(char32_t c) {
    if (c < 0x20 || (c >= 0x7F && c < 0xA0)) {
        return true;
    }
    return c == 0x061C || c == 0x200E || c == 0x200F || (c >= 0x202A && c <= 0x202E) ||
           (c >= 0x2066 && c <= 0x2069);
}

std::optional<int> named_key(const QString& name) {
    static const QHash<QString, int> keys = {
        {QStringLiteral("enter"), Qt::Key_Return},
        {QStringLiteral("tab"), Qt::Key_Tab},
        {QStringLiteral("escape"), Qt::Key_Escape},
        {QStringLiteral("backspace"), Qt::Key_Backspace},
        {QStringLiteral("delete"), Qt::Key_Delete},
        {QStringLiteral("insert"), Qt::Key_Insert},
        {QStringLiteral("home"), Qt::Key_Home},
        {QStringLiteral("end"), Qt::Key_End},
        {QStringLiteral("page_up"), Qt::Key_PageUp},
        {QStringLiteral("page_down"), Qt::Key_PageDown},
        {QStringLiteral("arrow_up"), Qt::Key_Up},
        {QStringLiteral("arrow_down"), Qt::Key_Down},
        {QStringLiteral("arrow_left"), Qt::Key_Left},
        {QStringLiteral("arrow_right"), Qt::Key_Right},
        {QStringLiteral("space"), Qt::Key_Space},
        {QStringLiteral("f1"), Qt::Key_F1},   {QStringLiteral("f2"), Qt::Key_F2},
        {QStringLiteral("f3"), Qt::Key_F3},   {QStringLiteral("f4"), Qt::Key_F4},
        {QStringLiteral("f5"), Qt::Key_F5},   {QStringLiteral("f6"), Qt::Key_F6},
        {QStringLiteral("f7"), Qt::Key_F7},   {QStringLiteral("f8"), Qt::Key_F8},
        {QStringLiteral("f9"), Qt::Key_F9},   {QStringLiteral("f10"), Qt::Key_F10},
        {QStringLiteral("f11"), Qt::Key_F11}, {QStringLiteral("f12"), Qt::Key_F12},
    };
    const auto found = keys.constFind(name);
    if (found != keys.constEnd()) {
        return *found;
    }
    if (name.size() == 1) {
        const QChar c = name.at(0);
        if (c >= QLatin1Char('a') && c <= QLatin1Char('z')) {
            return Qt::Key_A + (c.unicode() - 'a');
        }
        if (c >= QLatin1Char('0') && c <= QLatin1Char('9')) {
            return Qt::Key_0 + (c.unicode() - '0');
        }
    }
    return std::nullopt;
}

}  // namespace

std::optional<QByteArray> parse_key(const QByteArray& file_contents) {
    const QByteArray hex = file_contents.trimmed();
    if (hex.size() != 64) {
        return std::nullopt;
    }
    for (char c : hex) {
        const bool ok = (c >= '0' && c <= '9') || (c >= 'a' && c <= 'f') || (c >= 'A' && c <= 'F');
        if (!ok) {
            return std::nullopt;
        }
    }
    return QByteArray::fromHex(hex);
}

DecodeResult decode_command(const QByteArray& raw_line, const QByteArray& key, std::int64_t now_ms) {
    if (raw_line.size() > kMaxEnvelopeBytes) {
        return fail("command_too_large");
    }
    QByteArray line = raw_line;
    if (line.endsWith('\n')) {
        line.chop(1);
    }
    QJsonParseError parse_error{};
    const QJsonDocument envelope_doc = QJsonDocument::fromJson(line, &parse_error);
    if (parse_error.error != QJsonParseError::NoError || !envelope_doc.isObject()) {
        return fail("command_malformed");
    }
    const QJsonObject envelope = envelope_doc.object();
    if (!exact_keys(envelope, {"v", "payload", "mac"}) || !envelope.value(QLatin1String("payload")).isString() ||
        !envelope.value(QLatin1String("mac")).isString()) {
        return fail("command_malformed");
    }
    if (integer(envelope.value(QLatin1String("v")), 0, 65535) != std::optional<std::int64_t>{1}) {
        return fail("command_version");
    }
    const QByteArray payload = envelope.value(QLatin1String("payload")).toString().toUtf8();
    const QByteArray mac_hex = envelope.value(QLatin1String("mac")).toString().toLatin1();
    const auto mac = parse_key(mac_hex);  // 32 bytes of hex, the same shape as a key
    if (!mac.has_value() || key.size() != 32) {
        return fail("command_malformed");
    }

    QMessageAuthenticationCode code(QCryptographicHash::Sha256, key);
    code.addData(QByteArrayView("archie-stage/1\0", 15));
    code.addData(payload);
    // Before the payload is parsed at all.
    if (!constant_time_equal(code.result(), *mac)) {
        return fail("command_mac_invalid");
    }

    const QJsonDocument payload_doc = QJsonDocument::fromJson(payload, &parse_error);
    if (parse_error.error != QJsonParseError::NoError || !payload_doc.isObject()) {
        return fail("command_malformed");
    }
    const QJsonObject p = payload_doc.object();
    if (!exact_keys(p, {"v", "seq", "issued_at_ms", "expires_at_ms", "operation_id", "surface", "app_id",
                        "geometry_revision", "action", "trace_id"})) {
        return fail("command_malformed");
    }
    if (integer(p.value(QLatin1String("v")), 0, 65535) != std::optional<std::int64_t>{1}) {
        return fail("command_version");
    }
    const auto seq = integer(p.value(QLatin1String("seq")), 1, kMaxExactInteger);
    const auto issued = integer(p.value(QLatin1String("issued_at_ms")), 0, kMaxExactInteger);
    const auto expires = integer(p.value(QLatin1String("expires_at_ms")), 0, kMaxExactInteger);
    const auto surface = integer(p.value(QLatin1String("surface")), 0, 4294967295.0);
    const auto revision = integer(p.value(QLatin1String("geometry_revision")), 0, kMaxExactInteger);
    if (!seq || !issued || !expires || !surface || !revision ||
        !integer(p.value(QLatin1String("operation_id")), 0, kMaxExactInteger) ||
        !p.value(QLatin1String("app_id")).isString() || !p.value(QLatin1String("trace_id")).isString() ||
        !p.value(QLatin1String("action")).isObject()) {
        return fail("command_malformed");
    }
    if (*expires <= *issued || *expires - *issued > kMaxCommandTtlMs) {
        return fail("command_ttl_invalid");
    }
    if (now_ms >= *expires) {
        return fail("command_expired");
    }
    if (*issued > now_ms + kClockSkewMs) {
        return fail("command_from_the_future");
    }
    const QString app_id = p.value(QLatin1String("app_id")).toString();
    if (app_id.isEmpty() || app_id.size() > 255) {
        return fail("command_malformed");
    }

    DecodedCommand decoded;
    decoded.issued_at_ms = *issued;
    StageCommand& c = decoded.command;
    c.sequence = static_cast<std::uint64_t>(*seq);
    c.expires_at_ms = *expires;
    c.surface = static_cast<archie::visual::SurfaceId>(*surface);
    c.app_id = app_id.toStdString();
    c.geometry_revision = static_cast<std::uint64_t>(*revision);

    const QJsonObject action = p.value(QLatin1String("action")).toObject();
    const QString kind = action.value(QLatin1String("kind")).toString();
    const auto point = [&action](const char* x, const char* y) -> std::optional<std::pair<int, int>> {
        const auto px = coordinate(action.value(QLatin1String(x)));
        const auto py = coordinate(action.value(QLatin1String(y)));
        if (!px || !py) return std::nullopt;
        return std::pair<int, int>{*px, *py};
    };

    if (kind == QLatin1String("move")) {
        const auto at = point("x", "y");
        if (!exact_keys(action, {"kind", "x", "y"}) || !at) return fail("command_malformed");
        c.action = StageAction::Move;
        c.local_x = at->first;
        c.local_y = at->second;
    } else if (kind == QLatin1String("click") || kind == QLatin1String("double_click")) {
        const auto at = point("x", "y");
        const auto b = button(action.value(QLatin1String("button")));
        if (!exact_keys(action, {"kind", "x", "y", "button"}) || !at || !b) return fail("command_malformed");
        c.action = kind == QLatin1String("click") ? StageAction::Click : StageAction::DoubleClick;
        c.local_x = at->first;
        c.local_y = at->second;
        c.button = *b;
    } else if (kind == QLatin1String("drag")) {
        const auto from = point("x", "y");
        const auto to = point("to_x", "to_y");
        const auto b = button(action.value(QLatin1String("button")));
        if (!exact_keys(action, {"kind", "x", "y", "to_x", "to_y", "button"}) || !from || !to || !b) {
            return fail("command_malformed");
        }
        c.action = StageAction::Drag;
        c.local_x = from->first;
        c.local_y = from->second;
        c.to_local_x = to->first;
        c.to_local_y = to->second;
        c.button = *b;
    } else if (kind == QLatin1String("scroll")) {
        const auto at = point("x", "y");
        const auto sx = integer(action.value(QLatin1String("steps_x")), -archie::visual::kMaxScrollSteps,
                                archie::visual::kMaxScrollSteps);
        const auto sy = integer(action.value(QLatin1String("steps_y")), -archie::visual::kMaxScrollSteps,
                                archie::visual::kMaxScrollSteps);
        if (!exact_keys(action, {"kind", "x", "y", "steps_x", "steps_y"}) || !at || !sx || !sy) {
            return fail("command_malformed");
        }
        c.action = StageAction::Scroll;
        c.local_x = at->first;
        c.local_y = at->second;
        c.scroll_steps_x = static_cast<int>(*sx);
        c.scroll_steps_y = static_cast<int>(*sy);
    } else if (kind == QLatin1String("text")) {
        if (!exact_keys(action, {"kind", "text"}) || !action.value(QLatin1String("text")).isString()) {
            return fail("command_malformed");
        }
        const QList<uint> units = action.value(QLatin1String("text")).toString().toUcs4();
        if (units.isEmpty() || units.size() > static_cast<qsizetype>(archie::visual::kMaxTextChars)) {
            return fail("command_malformed");
        }
        for (uint unit : units) {
            if (is_bidi_or_control(static_cast<char32_t>(unit))) {
                return fail("command_malformed");
            }
            decoded.text.append(static_cast<char32_t>(unit));
        }
        c.action = StageAction::Text;
        c.text_chars = static_cast<std::size_t>(units.size());
    } else if (kind == QLatin1String("key")) {
        if (!exact_keys(action, {"kind", "modifiers", "key"}) || !action.value(QLatin1String("modifiers")).isArray() ||
            !action.value(QLatin1String("key")).isString()) {
            return fail("command_malformed");
        }
        QSet<QString> seen;
        for (const QJsonValue& value : action.value(QLatin1String("modifiers")).toArray()) {
            const QString modifier = value.toString();
            if (seen.contains(modifier)) return fail("command_malformed");
            seen.insert(modifier);
            if (modifier == QLatin1String("ctrl")) decoded.keys.append(Qt::Key_Control);
            else if (modifier == QLatin1String("shift")) decoded.keys.append(Qt::Key_Shift);
            else if (modifier == QLatin1String("alt")) decoded.keys.append(Qt::Key_Alt);
            else if (modifier == QLatin1String("super")) return fail("chord_reserved");
            else return fail("command_malformed");
        }
        if (seen.contains(QStringLiteral("ctrl")) && seen.contains(QStringLiteral("alt"))) {
            return fail("chord_reserved");
        }
        const auto key_code = named_key(action.value(QLatin1String("key")).toString());
        if (!key_code) return fail("command_malformed");
        decoded.keys.append(*key_code);
        c.action = StageAction::Key;
        c.key_count = static_cast<int>(decoded.keys.size());
    } else if (kind == QLatin1String("focus")) {
        if (!exact_keys(action, {"kind"})) return fail("command_malformed");
        c.action = StageAction::Focus;
    } else if (kind == QLatin1String("snapshot")) {
        if (!exact_keys(action, {"kind"})) return fail("command_malformed");
        c.action = StageAction::Snapshot;
    } else {
        return fail("command_malformed");
    }

    DecodeResult result;
    result.command = std::move(decoded);
    return result;
}

}  // namespace archie::stage
