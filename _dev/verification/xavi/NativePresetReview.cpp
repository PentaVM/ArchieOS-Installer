// Link against the audited display's objects/resources, excluding src/main.cpp.o.
// This exercises the production LumenField without starting a producer or daemon.
#include "LumenField.h"
#include <QFile>
#include <QGuiApplication>
#include <QJsonDocument>
#include <QJsonObject>
#include <QTextStream>
#include <cmath>

int main(int argc, char **argv)
{
    QGuiApplication app(argc, argv);
    LumenField field;
    const QVariantMap baseline = field.settings();
    QTextStream out(stdout);
    int checks = 0;
    int failures = 0;
    const auto check = [&](bool ok, const QString &message) {
        ++checks;
        if (!ok) { ++failures; out << "FAIL " << message << '\n'; }
    };
    for (const QString &name : {QStringLiteral("field"), QStringLiteral("sphere"),
                                QStringLiteral("orb"), QStringLiteral("neural")}) {
        QFile resource(QStringLiteral(":/archie/presets/") + name + QStringLiteral(".json"));
        check(resource.open(QIODevice::ReadOnly), name + QStringLiteral(" embedded resource"));
        const auto document = QJsonDocument::fromJson(resource.readAll());
        check(document.isObject(), name + QStringLiteral(" valid JSON"));
        check(document.object().value(QStringLiteral("name")).toString() == name,
              name + QStringLiteral(" resource identity"));
        field.applyProfile(name);
        check(field.profile() == name, name + QStringLiteral(" selected"));
        const auto settings = document.object().value(QStringLiteral("settings")).toObject();
        for (auto it = settings.begin(); it != settings.end(); ++it) {
            check(baseline.contains(it.key()), name + QStringLiteral(" recognised key ") + it.key());
            check(std::abs(field.settings().value(it.key()).toDouble() - it.value().toDouble()) < 1e-9,
                  name + QStringLiteral(" applied value ") + it.key());
        }
        const auto applied = field.settings();
        field.setSetting(QStringLiteral("nodeSize"), 1.2);
        field.resetProfile();
        check(field.settings() == applied, name + QStringLiteral(" reset restores preset"));
    }
    field.applyProfile(QStringLiteral("field"));
    check(field.settings() == baseline, QStringLiteral("field restores original defaults"));
    field.applyProfile(QStringLiteral("not-a-preset"));
    check(field.profile() == QStringLiteral("field") && field.settings() == baseline,
          QStringLiteral("unknown preset falls back to field"));
    out << "NATIVE_PRESET_REVIEW " << checks << " checks, " << failures << " failures\n";
    return failures ? 1 : 0;
}
