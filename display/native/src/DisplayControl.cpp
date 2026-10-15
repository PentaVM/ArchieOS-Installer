#include "DisplayControl.h"

#include <QHostAddress>
#include <QJsonDocument>
#include <QJsonObject>
#include <QTcpSocket>
#include <QTimer>

#include <memory>

DisplayControl::DisplayControl(QObject *parent, bool shellMode) : QObject(parent), m_shellMode(shellMode)
{
    connect(&m_server, &QTcpServer::newConnection, this, [this] {
        while (QTcpSocket *socket = m_server.nextPendingConnection()) {
            socket->setParent(this);
            socket->setReadBufferSize(4097);
            QTimer::singleShot(3000, socket, [socket] {
                if (socket->state() == QAbstractSocket::ConnectedState) socket->disconnectFromHost();
            });
            auto buffer = std::make_shared<QByteArray>();
            connect(socket, &QTcpSocket::readyRead, socket, [this, socket, buffer] {
                buffer->append(socket->read(4097 - buffer->size()));
                if (buffer->size() > 4096) {
                    socket->write("error: request too large\n");
                    socket->disconnectFromHost();
                    return;
                }
                const qsizetype end = buffer->indexOf('\n');
                if (end < 0) return;
                const QJsonDocument document = QJsonDocument::fromJson(buffer->left(end));
                const QJsonObject command = document.object();
                const QString action = command.value(QStringLiteral("action")).toString();
                const QString value = command.value(QStringLiteral("value")).toString();
                if (m_shellMode && action == QLatin1String("scene") && value == QLatin1String("world")) {
                    socket->write("error: World is available in Observatory, not in the shell\n");
                    socket->disconnectFromHost();
                    return;
                }
                const bool hasEnabled = command.value(QStringLiteral("enabled")).isBool();
                const bool valid =
                    (action == QLatin1String("preset")
                        && (value == QLatin1String("field") || value == QLatin1String("sphere")
                            || value == QLatin1String("orb") || value == QLatin1String("neural"))) ||
                    (action == QLatin1String("scene")
                     && (value == QLatin1String("orb") || value == QLatin1String("forge")
                         || value == QLatin1String("world")))
                    || (action == QLatin1String("cinematic")
                        && (value == QLatin1String("field") || value == QLatin1String("crown")
                            || value == QLatin1String("atlas") || value == QLatin1String("dense")))
                    || (action == QLatin1String("overlay") && hasEnabled
                        && (value == QLatin1String("hud") || value == QLatin1String("shell")
                            || value == QLatin1String("ambient") || value == QLatin1String("guides")
                            || value == QLatin1String("links") || value == QLatin1String("pulses")
                            || value == QLatin1String("bloom")))
                    || (m_shellMode && action == QLatin1String("panel")
                        && (value == QLatin1String("forge") || value == QLatin1String("inspector")
                            || value == QLatin1String("settings") || value == QLatin1String("sentry")
                            || value == QLatin1String("activity") || value == QLatin1String("notifications")
                            || value == QLatin1String("search")));
                if (valid) {
                    emit commandReceived(action, value, command.value(QStringLiteral("enabled")).toBool());
                    socket->write("ok\n");
                } else {
                    socket->write("error: invalid visual command\n");
                }
                socket->disconnectFromHost();
            });
            connect(socket, &QTcpSocket::disconnected, socket, &QTcpSocket::deleteLater);
        }
    });
}

bool DisplayControl::start(quint16 port)
{
    return m_server.listen(QHostAddress::LocalHost, port);
}
