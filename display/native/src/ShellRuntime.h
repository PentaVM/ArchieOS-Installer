#pragma once

#include <QObject>
#include <QProcess>
#include <QTimer>

// A fixed read-only bridge command. Programs and paths are configured by the trusted
// host CLI; QML can start/stop it, but cannot choose a command or shell expression.
class ShellRuntime final : public QObject {
    Q_OBJECT
    Q_PROPERTY(bool running READ running WRITE setRunning NOTIFY runningChanged)
    Q_PROPERTY(QString error READ error NOTIFY errorChanged)
public:
    explicit ShellRuntime(QString program, QString lanFile, QString workspacesRoot, QObject *parent = nullptr);
    ~ShellRuntime() override;
    bool running() const;
    QString error() const { return m_error; }
    void setRunning(bool enabled);
signals:
    void frameReady(const QString &line);
    void runningChanged();
    void errorChanged();
private:
    void start();
    void readFrames();
    void setError(const QString &message);
    void reconnect();
    QProcess m_process;
    QTimer m_restart;
    QTimer m_stop;
    QByteArray m_pending;
    QString m_error;
    bool m_wanted = false;
};
