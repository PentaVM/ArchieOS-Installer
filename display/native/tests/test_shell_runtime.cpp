#include "ShellRuntime.h"
#include <QCoreApplication>
#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QSignalSpy>
#include <QtTest>

class ShellRuntimeContracts final : public QObject {
    Q_OBJECT
private slots:
    void framesAreCompleteAndArgumentsStayLiteral() {
        ShellRuntime runtime(QCoreApplication::applicationFilePath(),
                             QStringLiteral("fixture:frames & literal"), QStringLiteral("/safe workspaces"));
        QSignalSpy frames(&runtime, &ShellRuntime::frameReady);
        runtime.setRunning(true);
        QTRY_VERIFY_WITH_TIMEOUT(frames.count() >= 2, 4000);
        QCOMPARE(frames[0][0].toString(), QStringLiteral("{\"schema\":1}"));
        const auto args = QJsonDocument::fromJson(frames[1][0].toString().toUtf8()).array();
        QCOMPARE(args[0].toString(), QStringLiteral("watch"));
        QCOMPARE(args[2].toString(), QStringLiteral("fixture:frames & literal"));
        QCOMPARE(args[4].toString(), QStringLiteral("/safe workspaces"));
        runtime.setRunning(false);
        QTRY_VERIFY(!runtime.running());
        const int count = frames.count();
        QTest::qWait(150);
        QCOMPARE(frames.count(), count);
    }
    void anOversizedUnterminatedFrameIsRejected() {
        ShellRuntime runtime(QCoreApplication::applicationFilePath(), QStringLiteral("fixture:oversize"), QString());
        QSignalSpy frames(&runtime, &ShellRuntime::frameReady);
        runtime.setRunning(true);
        QTRY_VERIFY_WITH_TIMEOUT(!runtime.error().isEmpty(), 4000);
        QCOMPARE(frames.count(), 0);
        runtime.setRunning(false);
    }
    void missingProgramIsReportedWithoutBlocking() {
        ShellRuntime runtime(QStringLiteral("/missing/archie-shell-bridge"), QString(), QString());
        runtime.setRunning(true);
        QTRY_VERIFY_WITH_TIMEOUT(!runtime.error().isEmpty(), 2000);
        QVERIFY(!runtime.running());
        runtime.setRunning(false);
    }
    void unexpectedExitReconnectsAndStopCancelsReconnect() {
        ShellRuntime runtime(QCoreApplication::applicationFilePath(), QStringLiteral("fixture:exit"), QString());
        QSignalSpy frames(&runtime, &ShellRuntime::frameReady);
        runtime.setRunning(true);
        QTRY_VERIFY_WITH_TIMEOUT(frames.count() >= 2, 6500);
        runtime.setRunning(false);
        const int count = frames.count();
        QTest::qWait(3200);
        QCOMPARE(frames.count(), count);
    }
};

int main(int argc, char **argv) {
    QCoreApplication app(argc, argv);
    const auto args = app.arguments();
    if (args.contains(QStringLiteral("watch"))) {
        QFile output;
        if (!output.open(stdout, QIODevice::WriteOnly)) return 2;
        const int at = args.indexOf(QStringLiteral("--lan-file"));
        const QString fixture = at >= 0 ? args.value(at + 1) : QString();
        if (fixture == QStringLiteral("fixture:oversize")) {
            output.write(QByteArray(1024 * 1024 + 1, 'x'));
            output.flush();
            QTest::qWait(2000);
        } else if (fixture == QStringLiteral("fixture:exit")) {
            output.write("{\"schema\":1}\n");
            output.flush();
        } else {
            output.write("{\"schema\""); output.flush();
            QTest::qWait(20);
            output.write(":1}\n");
            QJsonArray literal;
            for (const auto &arg : args.mid(1)) literal.append(arg);
            output.write(QJsonDocument(literal).toJson(QJsonDocument::Compact) + '\n');
            output.flush();
            QTest::qWait(2000);
        }
        return 0;
    }
    ShellRuntimeContracts tests;
    return QTest::qExec(&tests, argc, argv);
}
#include "test_shell_runtime.moc"
