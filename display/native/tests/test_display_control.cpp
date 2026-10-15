#include "DisplayControl.h"
#include <QSignalSpy>
#include <QTcpSocket>
#include <QtTest>

class DisplayControlContracts final : public QObject {
    Q_OBJECT
private slots:
    void existingVoiceCommandsAreDelivered() {
        DisplayControl control;
        QVERIFY(control.start(0));
        QSignalSpy delivered(&control, &DisplayControl::commandReceived);
        const QList<QByteArray> commands{
            "{\"action\":\"scene\",\"value\":\"world\"}\n",
            "{\"action\":\"cinematic\",\"value\":\"crown\"}\n",
            "{\"action\":\"overlay\",\"value\":\"pulses\",\"enabled\":false}\n"};
        for (const auto &command : commands) {
            QTcpSocket socket;
            socket.connectToHost(QHostAddress::LocalHost, control.port());
            QTRY_COMPARE(socket.state(), QAbstractSocket::ConnectedState);
            socket.write(command);
            QTRY_VERIFY(socket.bytesAvailable() > 0);
            QCOMPARE(socket.readAll(), QByteArray("ok\n"));
        }
        QCOMPARE(delivered.count(), 3);
        QCOMPARE(delivered[2][2].toBool(), false);
    }
    void shellDoesNotAcknowledgeAnUnavailableWorldView() {
        DisplayControl control(nullptr, true);
        QVERIFY(control.start(0));
        QSignalSpy delivered(&control, &DisplayControl::commandReceived);
        QTcpSocket socket;
        socket.connectToHost(QHostAddress::LocalHost, control.port());
        QTRY_COMPARE(socket.state(), QAbstractSocket::ConnectedState);
        socket.write("{\"action\":\"scene\",\"value\":\"world\"}\n");
        QTRY_VERIFY(socket.bytesAvailable() > 0);
        QVERIFY(socket.readAll().startsWith("error:"));
        QCOMPARE(delivered.count(), 0);
    }
    void overlaysRequireAnActualBoolean() {
        DisplayControl control;
        QVERIFY(control.start(0));
        QSignalSpy delivered(&control, &DisplayControl::commandReceived);
        QTcpSocket socket;
        socket.connectToHost(QHostAddress::LocalHost, control.port());
        QTRY_COMPARE(socket.state(), QAbstractSocket::ConnectedState);
        socket.write("{\"action\":\"overlay\",\"value\":\"shell\",\"enabled\":\"true\"}\n");
        QTRY_VERIFY(socket.bytesAvailable() > 0);
        QVERIFY(socket.readAll().startsWith("error:"));
        QCOMPARE(delivered.count(), 0);
    }
    void shellPanelsAreAllowlistedLocalViews() {
        DisplayControl control(nullptr, true);
        QVERIFY(control.start(0));
        QSignalSpy delivered(&control, &DisplayControl::commandReceived);
        QTcpSocket socket;
        socket.connectToHost(QHostAddress::LocalHost, control.port());
        QTRY_COMPARE(socket.state(), QAbstractSocket::ConnectedState);
        socket.write("{\"action\":\"panel\",\"value\":\"inspector\"}\n");
        QTRY_VERIFY(socket.bytesAvailable() > 0);
        QCOMPARE(socket.readAll(), QByteArray("ok\n"));
        QCOMPARE(delivered.count(), 1);
        QTcpSocket invalid;
        invalid.connectToHost(QHostAddress::LocalHost, control.port());
        QTRY_COMPARE(invalid.state(), QAbstractSocket::ConnectedState);
        invalid.write("{\"action\":\"panel\",\"value\":\"execute\"}\n");
        QTRY_VERIFY(invalid.bytesAvailable() > 0);
        QVERIFY(invalid.readAll().startsWith("error:"));
        QCOMPARE(delivered.count(), 1);
    }
};
QTEST_GUILESS_MAIN(DisplayControlContracts)
#include "test_display_control.moc"
