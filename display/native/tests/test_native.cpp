#include "LumenField.h"
#include <QCoreApplication>
#include <QSignalSpy>
#include <QTcpServer>
#include <QTcpSocket>
#include <QtTest>

class NativeContracts final : public QObject {
    Q_OBJECT
private slots:
    void zeroBudgetProducesNoFrames() {
        LumenField field;
        QVERIFY(field.setProperty("frameIntervalMs", 0));
        QSignalSpy frames(&field, &LumenField::frameAdvanced);
        field.setActive(true);
        QTest::qWait(80);
        QCOMPARE(frames.count(), 0);
        field.setActive(false);
    }
    void reducedMotionPreservesProfileSettings() {
        LumenField field;
        field.applyProfile(QStringLiteral("crown"));
        const auto before = field.settings();
        QVERIFY(field.setProperty("reducedMotion", true));
        QCOMPARE(field.settings(), before);
        field.toggleCompare();
        QVERIFY(field.comparing());
        field.toggleCompare();
        QCOMPARE(field.settings(), before);
    }
    void existingProfilesAndOverlaysRemainAvailable() {
        LumenField field;
        for (const auto &profile : {"field", "crown", "atlas", "dense"}) {
            field.applyProfile(QString::fromLatin1(profile));
            QCOMPARE(field.profile(), QString::fromLatin1(profile));
            field.setSetting(QStringLiteral("pulses"), false);
            QCOMPARE(field.settings().value(QStringLiteral("pulses")).toBool(), false);
        }
    }
    void newPresetsAreAdditiveAndRestoreOriginalDefaults() {
        LumenField field;
        const auto original=field.settings();
        for (const auto &profile : {"sphere", "orb", "neural"}) {
            field.applyProfile(QString::fromLatin1(profile));
            QCOMPARE(field.profile(), QString::fromLatin1(profile));
            QVERIFY(field.settings()!=original);
        }
        field.applyProfile(QStringLiteral("field"));
        QCOMPARE(field.settings(),original);
        field.applyProfile(QStringLiteral("crown"));
        QCOMPARE(field.profile(),QStringLiteral("crown"));
    }
    void onlyOneViewportCanOwnTheController() {
        LumenField field;
        QObject first, second;
        QVERIFY(field.acquireView(&first));
        QVERIFY(!field.acquireView(&second));
        field.releaseView(&first);
        QVERIFY(field.acquireView(&second));
    }
    void obsoleteLensResponsesDoNotPublish() {
        QTcpServer server;
        QVERIFY(server.listen(QHostAddress::LocalHost, 0));
        LumenField field(nullptr, QUrl(QStringLiteral("http://127.0.0.1:%1/api/orb/lumen").arg(server.serverPort())));
        QSignalSpy scenes(&field, &LumenField::sceneChanged);
        field.setActive(true);
        QTRY_VERIFY(server.hasPendingConnections());
        auto *old = server.nextPendingConnection();
        QTRY_VERIFY(old->bytesAvailable() > 0);
        old->readAll();
        field.setLens(QStringLiteral("folders"));
        const auto respond = [](QTcpSocket *socket, const QByteArray &name) {
            const QByteArray body = "{\"nodes\":[{\"id\":\"" + name + "\",\"name\":\"" + name
                + "\",\"kind\":\"root\",\"position\":[0,0,0]}],\"root_id\":\"" + name
                + "\",\"focus_id\":\"" + name + "\"}";
            socket->write("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: "
                + QByteArray::number(body.size()) + "\r\n\r\n" + body);
            socket->disconnectFromHost();
        };
        respond(old, "obsolete");
        QTRY_VERIFY(server.hasPendingConnections());
        QCOMPARE(scenes.count(), 0);
        auto *fresh = server.nextPendingConnection();
        QTRY_VERIFY(fresh->bytesAvailable() > 0);
        const auto request = fresh->readAll();
        QVERIFY(request.contains("lens=folders"));
        respond(fresh, "current");
        QTRY_COMPARE(scenes.count(), 1);
        QVERIFY(field.loaded());
        QCOMPARE(field.scene().value(QStringLiteral("focusId")).toString(), QStringLiteral("current"));
        field.setActive(false);
    }
    void deactivationStopsFrameTicksAndQueries() {
        QTcpServer server;
        QVERIFY(server.listen(QHostAddress::LocalHost, 0));
        LumenField field(nullptr, QUrl(QStringLiteral("http://127.0.0.1:%1/api/orb/lumen").arg(server.serverPort())));
        QSignalSpy frames(&field, &LumenField::frameAdvanced);
        field.setActive(true);
        QTRY_VERIFY(frames.count() > 0);
        field.setActive(false);
        const int count = frames.count();
        QTest::qWait(100);
        QCOMPARE(frames.count(), count);
    }
    void quietMotionDoesNotOverwriteRotationPreference() {
        QTcpServer server;
        QVERIFY(server.listen(QHostAddress::LocalHost, 0));
        LumenField field(nullptr, QUrl(QStringLiteral("http://127.0.0.1:%1/api/orb/lumen").arg(server.serverPort())));
        field.setActive(true);
        QTRY_VERIFY(field.spin() > 0);
        field.setReducedMotion(true);
        const float spin = field.spin();
        QTest::qWait(70);
        QCOMPARE(field.spin(), spin);
        QVERIFY(field.settings().value(QStringLiteral("rotate")).toBool());
        field.setActive(false);
    }
};

QTEST_GUILESS_MAIN(NativeContracts)
#include "test_native.moc"
