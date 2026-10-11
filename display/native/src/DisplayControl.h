#pragma once

#include <QObject>
#include <QTcpServer>

// Loopback-only visual controls for Archie's live voice. No OS or Stage actions enter
// this channel; the renderer accepts only the allowlisted scene, profile and layer names.
class DisplayControl final : public QObject
{
    Q_OBJECT

public:
    explicit DisplayControl(QObject *parent = nullptr);
    bool start();

signals:
    void commandReceived(const QString &action, const QString &value, bool enabled);

private:
    QTcpServer m_server;
};
