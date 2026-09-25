// The compositor's command codec against the shared golden command and hand-built
// variations of it: the C++ half of the contract `archie-stage::seal` tests in Rust.

#include <QByteArray>
#include <QFile>
#include <QJsonDocument>
#include <QJsonObject>
#include <QMessageAuthenticationCode>

#include "CommandCodec.h"
#include "harness.hpp"

using archie::stage::decode_command;
using archie::stage::parse_key;
using archie::visual::PointerButton;
using archie::visual::StageAction;

namespace {

struct Golden {
    QByteArray key;
    QByteArray line;
    std::int64_t now_ms = 0;
};

Golden golden() {
    QFile file(QStringLiteral(ARCHIE_FIXTURES "/stage-command-v1.json"));
    Golden g;
    if (!file.open(QIODevice::ReadOnly)) {
        std::printf("cannot read the golden fixture\n");
        return g;
    }
    const QJsonObject fixture = QJsonDocument::fromJson(file.readAll()).object();
    g.key = *parse_key(fixture.value(QStringLiteral("key_hex")).toString().toLatin1());
    g.line = fixture.value(QStringLiteral("line")).toString().toUtf8();
    g.now_ms = static_cast<std::int64_t>(fixture.value(QStringLiteral("now_ms")).toDouble());
    return g;
}

/// Seal a payload with the golden key, the way a sealer holding the key would.
QByteArray seal(const QByteArray& key, const QByteArray& payload) {
    QMessageAuthenticationCode code(QCryptographicHash::Sha256, key);
    code.addData(QByteArrayView("archie-stage/1\0", 15));
    code.addData(payload);
    QJsonObject envelope{{QStringLiteral("v"), 1},
                         {QStringLiteral("payload"), QString::fromUtf8(payload)},
                         {QStringLiteral("mac"), QString::fromLatin1(code.result().toHex())}};
    return QJsonDocument(envelope).toJson(QJsonDocument::Compact) + '\n';
}

QByteArray payload(const QByteArray& action, std::int64_t issued, std::int64_t expires) {
    return QByteArray(R"({"v":1,"seq":7,"issued_at_ms":)") + QByteArray::number(issued) +
           R"(,"expires_at_ms":)" + QByteArray::number(expires) +
           R"(,"operation_id":9,"surface":4,"app_id":"org.freecad.FreeCAD","geometry_revision":77,"action":)" +
           action + R"(,"trace_id":"t"})";
}

void the_golden_command_opens() {
    CASE("the shared golden command, sealed by openssl, opens as the same click");

    const Golden g = golden();
    const auto result = decode_command(g.line, g.key, g.now_ms);
    CHECK_BECAUSE(result.command.has_value(), "the compositor and archie-stage must agree on the MAC byte for byte");
    if (!result.command) {
        return;
    }
    const auto& c = result.command->command;
    CHECK_EQ(c.sequence, std::uint64_t{7});
    CHECK(c.action == StageAction::Click);
    CHECK_EQ(c.local_x, 10);
    CHECK_EQ(c.local_y, 20);
    CHECK(c.button == PointerButton::Left);
    CHECK_EQ(c.surface, archie::visual::SurfaceId{4});
    CHECK(c.app_id == "org.freecad.FreeCAD");
    CHECK_EQ(c.geometry_revision, std::uint64_t{77});
}

void nothing_unauthenticated_is_parsed() {
    CASE("a changed byte, another key, an oversized line or a bad envelope never opens");

    const Golden g = golden();
    QByteArray moved = g.line;
    moved.replace(R"(\"x\":10)", R"(\"x\":11)");
    CHECK(moved != g.line);
    CHECK(decode_command(moved, g.key, g.now_ms).error == QStringLiteral("command_mac_invalid"));
    CHECK(decode_command(g.line, QByteArray(32, '\x01'), g.now_ms).error == QStringLiteral("command_mac_invalid"));
    CHECK(decode_command(QByteArray(40000, ' '), g.key, g.now_ms).error == QStringLiteral("command_too_large"));
    CHECK(decode_command("not json\n", g.key, g.now_ms).error == QStringLiteral("command_malformed"));
    CHECK(decode_command(R"({"v":1,"payload":"{}","mac":"00","extra":1})", g.key, g.now_ms).error ==
          QStringLiteral("command_malformed"));
}

void a_command_is_for_now() {
    CASE("expired, long-lived and future commands are refused");

    const Golden g = golden();
    CHECK(decode_command(g.line, g.key, 1760000002000).error == QStringLiteral("command_expired"));
    const QByteArray long_lived = seal(g.key, payload(R"({"kind":"focus"})", 1000, 1000 + 10001));
    CHECK(decode_command(long_lived, g.key, 1500).error == QStringLiteral("command_ttl_invalid"));
    const QByteArray future = seal(g.key, payload(R"({"kind":"focus"})", 10'000, 12'000));
    CHECK(decode_command(future, g.key, 5'000).error == QStringLiteral("command_from_the_future"));
    const QByteArray fine = seal(g.key, payload(R"({"kind":"focus"})", 10'000, 12'000));
    CHECK(decode_command(fine, g.key, 10'500).command.has_value());
}

void a_verifying_payload_must_still_be_a_command() {
    CASE("a payload that verifies but is not exactly a command is refused");

    const Golden g = golden();
    const auto refused = [&](const QByteArray& action) {
        return decode_command(seal(g.key, payload(action, 10'000, 12'000)), g.key, 10'500);
    };
    CHECK(refused(R"({"kind":"click","x":1,"y":2,"button":"left","extra":true})").error == QStringLiteral("command_malformed"));
    CHECK(refused(R"({"kind":"click","x":-1,"y":2,"button":"left"})").error == QStringLiteral("command_malformed"));
    CHECK(refused(R"({"kind":"click","x":1.5,"y":2,"button":"left"})").error == QStringLiteral("command_malformed"));
    CHECK(refused(R"({"kind":"teleport","x":1,"y":2})").error == QStringLiteral("command_malformed"));
    CHECK_BECAUSE(refused(R"({"kind":"text","text":"rm -rf ./build\n"})").error == QStringLiteral("command_malformed"),
                  "a newline in text would run a command without the Enter key's approval");
    // Built at run time: the compiler rightly refuses an unpaired direction override in
    // source, which is the same trick this check stops.
    const QByteArray overridden = QJsonDocument(QJsonObject{{QStringLiteral("kind"), QStringLiteral("text")},
                                                             {QStringLiteral("text"), QStringLiteral("txt") + QChar(0x202E) + QStringLiteral("exe")}})
                                      .toJson(QJsonDocument::Compact);
    CHECK(refused(overridden).error == QStringLiteral("command_malformed"));
    CHECK(refused(R"({"kind":"key","modifiers":["super"],"key":"l"})").error == QStringLiteral("chord_reserved"));
    CHECK(refused(R"({"kind":"key","modifiers":["ctrl","alt"],"key":"delete"})").error == QStringLiteral("chord_reserved"));
    CHECK(refused(R"({"kind":"key","modifiers":["ctrl","ctrl"],"key":"s"})").error == QStringLiteral("command_malformed"));
    CHECK(refused(R"({"kind":"key","modifiers":[],"key":"Enter"})").error == QStringLiteral("command_malformed"));
    CHECK(refused(R"({"kind":"scroll","x":1,"y":1,"steps_x":0,"steps_y":61})").error == QStringLiteral("command_malformed"));
    CHECK_BECAUSE(refused(R"({"kind":"snapshot","x":0,"y":0,"width":10,"height":10})").error ==
                      QStringLiteral("command_malformed"),
                  "the part of a surface to capture is one region, not four loose numbers");
    CHECK_BECAUSE(refused(R"({"kind":"snapshot","region":{"x":1,"y":2,"width":0,"height":9}})").error ==
                      QStringLiteral("command_malformed"),
                  "a region with no pixels in it would read as the whole window");
    CHECK(refused(R"({"kind":"snapshot","region":{"x":-1,"y":2,"width":4,"height":9}})").error ==
          QStringLiteral("command_malformed"));
    CHECK(refused(R"({"kind":"snapshot","region":{"x":1,"y":2,"width":4}})").error ==
          QStringLiteral("command_malformed"));
    CHECK(refused(R"({"kind":"snapshot","region":{"x":1,"y":2,"width":4,"height":9,"why":"no"}})").error ==
          QStringLiteral("command_malformed"));
    CHECK_BECAUSE(refused(R"({"kind":"stroke","points":[{"x":1,"y":2}],"button":"left"})").error ==
                      QStringLiteral("command_malformed"),
                  "one point is not a line");
    CHECK(refused(R"({"kind":"stroke","points":[{"x":1,"y":2},{"x":3,"y":4,"z":5}],"button":"left"})").error ==
          QStringLiteral("command_malformed"));
    CHECK(refused(R"({"kind":"stroke","points":[{"x":1,"y":2},{"x":3,"y":4}]})").error ==
          QStringLiteral("command_malformed"));
    CHECK(refused(R"({"kind":"stroke","points":[[1,2],[3,4]],"button":"left"})").error ==
          QStringLiteral("command_malformed"));
    const auto stroke = refused(R"({"kind":"stroke","points":[{"x":1,"y":2},{"x":3,"y":4},{"x":5,"y":9}],"button":"left"})");
    CHECK(stroke.command.has_value());
    if (stroke.command) {
        const auto& drawn = stroke.command->command;
        CHECK(drawn.action == archie::visual::StageAction::Stroke);
        CHECK_EQ(drawn.path.size(), std::size_t{3});
        CHECK((drawn.path.front() == archie::visual::IPoint{1, 2}));
        CHECK((drawn.path.back() == archie::visual::IPoint{5, 9}));
        CHECK_BECAUSE(drawn.local_x == 1 && drawn.local_y == 2 && drawn.to_local_x == 5 &&
                          drawn.to_local_y == 9,
                      "where the line starts and ends, for everything that reads one point");
    }
    const auto look = refused(R"({"kind":"snapshot"})");
    CHECK(look.command.has_value() && look.command->command.action == archie::visual::StageAction::Snapshot);
    CHECK_BECAUSE(look.command && look.command->command.local_x == 0 && look.command->command.to_local_x == 0,
                  "naming no region is asking for the whole surface");
    // A part of the surface travels as two corners, which is what the stage re-checks.
    const auto part = refused(R"({"kind":"snapshot","region":{"x":12,"y":40,"width":80,"height":27}})");
    CHECK(part.command.has_value());
    if (part.command) {
        CHECK(part.command->command.action == archie::visual::StageAction::Snapshot);
        CHECK(part.command->command.local_x == 12 && part.command->command.local_y == 40);
        CHECK(part.command->command.to_local_x == 92 && part.command->command.to_local_y == 67);
    }
}

void text_and_chords_arrive_as_units() {
    CASE("typed text becomes code points and a chord becomes Qt keys, modifiers first");

    const Golden g = golden();
    const auto typing = decode_command(seal(g.key, payload(R"({"kind":"text","text":"hola ñandú 😀"})", 10'000, 12'000)),
                                       g.key, 10'500);
    CHECK(typing.command.has_value());
    if (typing.command) {
        CHECK(typing.command->command.action == StageAction::Text);
        CHECK_EQ(typing.command->command.text_chars, std::size_t{12});
        CHECK_EQ(typing.command->text.size(), qsizetype{12});
        CHECK(typing.command->text.last() == U'\U0001F600');
    }
    const auto chord = decode_command(seal(g.key, payload(R"({"kind":"key","modifiers":["ctrl","shift"],"key":"s"})", 10'000, 12'000)),
                                      g.key, 10'500);
    CHECK(chord.command.has_value());
    if (chord.command) {
        CHECK(chord.command->keys == (QList<int>{Qt::Key_Control, Qt::Key_Shift, Qt::Key_S}));
        CHECK_EQ(chord.command->command.key_count, 3);
    }
    CHECK(!parse_key("abc").has_value());
    CHECK(parse_key("  " + QByteArray(64, 'a') + "\n").has_value());
}

void a_held_key_is_one_key() {
    CASE("key_down and key_up carry exactly one key, and nothing else");

    const Golden g = golden();
    const auto decoded = [&](const QByteArray& action) {
        return decode_command(seal(g.key, payload(action, 10'000, 12'000)), g.key, 10'500);
    };
    const auto down = decoded(R"({"kind":"key_down","key":"w"})");
    CHECK(down.command.has_value());
    if (down.command) {
        CHECK(down.command->command.action == StageAction::KeyDown);
        CHECK(down.command->keys == (QList<int>{Qt::Key_W}));
        CHECK_EQ(down.command->command.key_count, 1);
    }
    const auto up = decoded(R"({"kind":"key_up","key":"arrow_left"})");
    CHECK(up.command.has_value());
    if (up.command) {
        CHECK(up.command->command.action == StageAction::KeyUp);
        CHECK(up.command->keys == (QList<int>{Qt::Key_Left}));
    }
    CHECK_BECAUSE(decoded(R"({"kind":"key_down","modifiers":["ctrl"],"key":"w"})").error ==
                      QStringLiteral("command_malformed"),
                  "a held chord is several holds, not one command with modifiers");
    CHECK(decoded(R"({"kind":"key_down","key":"Enter"})").error == QStringLiteral("command_malformed"));
    CHECK(decoded(R"({"kind":"key_up"})").error == QStringLiteral("command_malformed"));
}

}  // namespace

int main() {
    the_golden_command_opens();
    nothing_unauthenticated_is_parsed();
    a_command_is_for_now();
    a_verifying_payload_must_still_be_a_command();
    text_and_chords_arrive_as_units();
    a_held_key_is_one_key();
    return archie::visual::test::report();
}
