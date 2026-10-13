#include "DisplayControl.h"

#include <QHostAddress>
#include <QJsonDocument>
#include <QJsonObject>
#include <QTcpSocket>

#include <memory>

DisplayControl::DisplayControl(QObject *parent) : QObject(parent)
{
    connect(&m_server, &QTcpServer::newConnection, this, [this] {
        while (QTcpSocket *socket = m_server.nextPendingConnection()) {
            socket->setParent(this);
            auto buffer = std::make_shared<QByteArray>();
            connect(socket, &QTcpSocket::readyRead, socket, [this, socket, buffer] {
                buffer->append(socket->readAll());
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
                                const bool valid =
                    (action == QLatin1String("scene")
                     && (value == QLatin1String("orb") || value == QLatin1String("forge")
                         || value == QLatin1String("world")))
                    || (action == QLatin1String("preset")
                        && (value == QLatin1String("field") || value == QLatin1String("sphere")
                            || value == QLatin1String("orb") || value == QLatin1String("neural")));
                if (valid) {
                    emit commandReceived(action, value);
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

bool DisplayControl::start()
{
    return m_server.listen(QHostAddress::LocalHost, 7668);
}
