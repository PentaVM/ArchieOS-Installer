#include "LumenField.h"
#include "XaviDesignBridge.h"
#include "LumenGeometry.h"
#include <cstring>
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
    void ribbonGeometryPreservesBothEndpointsAndBounds() {
        LumenGeometry line;
        line.setLines({-3.F, -2.F, -1.F, 7.F, 5.F, 4.F});
        line.setRibbons(true);
        QCOMPARE(line.primitiveType(), QQuick3DGeometry::PrimitiveType::Triangles);
        QCOMPARE(line.vertexData().size(), qsizetype(4 * 10 * sizeof(float)));
        QCOMPARE(line.indexData().size(), qsizetype(6 * sizeof(std::uint32_t)));
        float first[10];
        const auto vertices = line.vertexData();
        std::memcpy(first, vertices.constData(), sizeof(first));
        QCOMPARE(QVector3D(first[0], first[1], first[2]), QVector3D(-3, -2, -1));
        QCOMPARE(QVector3D(first[3], first[4], first[5]), QVector3D(7, 5, 4));
        QCOMPARE(line.boundsMin(), QVector3D(-3, -2, -1));
        QCOMPARE(line.boundsMax(), QVector3D(7, 5, 4));
        line.setRibbons(false);
        QCOMPARE(line.primitiveType(), QQuick3DGeometry::PrimitiveType::Lines);
        QCOMPARE(line.vertexData().size(), qsizetype(6 * sizeof(float)));
    }
    // Packet phase belongs to an edge ordinal, never its moving world coordinates.
    void ribbonSeedsRemainStableWhenEndpointsMove() {
        LumenGeometry line;
        line.setRibbons(true);
        line.setLines({0.F, 0.F, 0.F, 1.F, 2.F, 3.F, 4.F, 5.F, 6.F, 7.F, 8.F, 9.F});
        float before[80];
        QCOMPARE(line.vertexData().size(), qsizetype(sizeof(before)));
        std::memcpy(before, line.vertexData().constData(), sizeof(before));
        line.setLines({10.F, 10.F, 10.F, 11.F, 12.F, 13.F, 14.F, 15.F, 16.F, 17.F, 18.F, 19.F});
        float after[80];
        QCOMPARE(line.vertexData().size(), qsizetype(sizeof(after)));
        std::memcpy(after, line.vertexData().constData(), sizeof(after));
        QCOMPARE(after[8], before[8]);
        QCOMPARE(after[48], before[48]);
        QVERIFY(before[8] != before[48]);
        QVERIFY(before[0] != after[0]);
    }
    void deliveredHybridAssetsDecodeToRequestedCpuBuffers() {
        XaviHybridData data;
        QCOMPARE(data.size(), QSize(1, 1));
        data.setResolution(64);
        data.setEnabled(true);
        QCOMPARE(data.size(), QSize(64, 64));
        QCOMPARE(data.textureData().size(), qsizetype(64 * 64 * 4));
        const auto structure = data.textureData();
        data.setSurface(true);
        QCOMPARE(data.size(), QSize(64, 64));
        QCOMPARE(data.textureData().size(), qsizetype(64 * 64 * 4));
        QVERIFY(data.textureData() != structure);
        data.setEnabled(false);
        QCOMPARE(data.size(), QSize(1, 1));
        QCOMPARE(data.textureData().size(), qsizetype(4));
    }
    void rustMeshUsesTexturedLayoutAndKeepsOriginalFallback() {
        XaviGeometry geometry;
        geometry.setStyle(1);
        QVERIFY(!geometry.vertexData().isEmpty());
        QCOMPARE(geometry.stride(), int(8 * sizeof(float)));
        QVERIFY(!geometry.indexData().isEmpty());
        geometry.setStyle(8);
        QVERIFY(geometry.vertexData().isEmpty());
        geometry.setStyle(0);
        QVERIFY(geometry.vertexData().isEmpty());
    }
    void deliveredPresetsShareValidatedCatalogAndKeepOsDensity() {
        LumenField field;
        const auto names = xaviPresetNames();
        QCOMPARE(names.size(), 8);
        QCOMPARE(field.settings().value(QStringLiteral("visibleLimit")).toInt(), 360);
        QCOMPARE(field.look().value(QStringLiteral("nodeStyleId")).toInt(), 0);
        QCOMPARE(field.settings().value(QStringLiteral("nodeSize")).toDouble(), 2.15);
        for (const auto &name : names) {
            QVERIFY2(field.presets().contains(name), qPrintable(name));
            const QString error = field.loadProfile(name);
            QVERIFY2(error.isEmpty(), qPrintable(name + QStringLiteral(": ") + error));
            QCOMPARE(field.profile(), name);
            QCOMPARE(field.settings().value(QStringLiteral("visibleLimit")).toInt(), 360);
            QVERIFY(field.look().value(QStringLiteral("nodeStyleId")).toInt() > 0);
        }
        QVERIFY(field.loadProfile(QStringLiteral("field")).isEmpty());
        const auto before = field.settings();
        field.setSetting(QStringLiteral("visualNodeStyle"), QStringLiteral("#ffffff"));
        field.setSetting(QStringLiteral("shellStyle"), QStringLiteral("execute"));
        QCOMPARE(field.settings(), before);
        field.setSetting(QStringLiteral("visualDetailFrequency"), 99.0);
        QCOMPARE(field.settings().value(QStringLiteral("visualDetailFrequency")).toDouble(), 3.0);
    }
    void invalidPresetDoesNotPartiallyApplyOrEraseComparison() {
        LumenField field;
        QVERIFY(field.applyProfile(QStringLiteral("neural")));
        field.toggleCompare();
        const auto before = field.settings();
        const auto profile = field.profile();
        QVERIFY(!field.loadProfile(QStringLiteral("invalid-test")).isEmpty());
        QCOMPARE(field.settings(), before);
        QCOMPARE(field.profile(), profile);
        QVERIFY(field.comparing());
        QVERIFY(!field.loadProfile(QStringLiteral("../field")).isEmpty());
        QVERIFY(field.loadProfile(QStringLiteral("field")).isEmpty());
        QVERIFY(!field.comparing());
        QCOMPARE(field.settings().value(QStringLiteral("labels")).toBool(), true);
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
