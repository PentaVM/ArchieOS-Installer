#pragma once

#include <QObject>
#include <QProcess>
#include <QTimer>

// A fixed read-only bridge command. Programs and paths are configured by the trusted
// host CLI; QML can start/stop it, but cannot choose a command or shell expression.
//
// Core's emergency pause is the one control the shell offers: `archie --json ai pause |
// resume | status`, a fixed argument list on the operator socket, which admits only the
// owner of the node's inference profile. Pausing only takes authority away; resuming needs
// that same owner, checked by the daemon, not here. The node is ARCHIE_ROOT's, as the CLI's.
class ShellRuntime final : public QObject {
    Q_OBJECT
    Q_PROPERTY(bool running READ running WRITE setRunning NOTIFY runningChanged)
    Q_PROPERTY(QString error READ error NOTIFY errorChanged)
    // "running", "paused" or "unknown" (no answer from the daemon yet, or it refused).
    Q_PROPERTY(QString aiPolicy READ aiPolicy NOTIFY aiPolicyChanged)
    Q_PROPERTY(QString aiPolicyNote READ aiPolicyNote NOTIFY aiPolicyChanged)
    Q_PROPERTY(bool aiPolicyBusy READ aiPolicyBusy NOTIFY aiPolicyChanged)
public:
    explicit ShellRuntime(QString program, QString lanFile, QString workspacesRoot, QObject *parent = nullptr);
    ~ShellRuntime() override;
    bool running() const;
    QString error() const { return m_error; }
    void setRunning(bool enabled);
    QString aiPolicy() const { return m_aiPolicy; }
    QString aiPolicyNote() const { return m_aiPolicyNote; }
    bool aiPolicyBusy() const { return m_aiProcess != nullptr; }
    Q_INVOKABLE void aiPolicyRefresh() { aiPolicyRequest(QStringLiteral("status")); }
    Q_INVOKABLE void aiPause() { aiPolicyRequest(QStringLiteral("pause")); }
    Q_INVOKABLE void aiResume() { aiPolicyRequest(QStringLiteral("resume")); }
signals:
    void frameReady(const QString &line);
    void runningChanged();
    void errorChanged();
    void aiPolicyChanged();
private:
    void start();
    void readFrames();
    void setError(const QString &message);
    void reconnect();
    void aiPolicyRequest(const QString &verb);
    QProcess *m_aiProcess = nullptr;
    QString m_aiPolicy = QStringLiteral("unknown");
    QString m_aiPolicyNote;
    QProcess m_process;
    QTimer m_restart;
    QTimer m_stop;
    QByteArray m_pending;
    QString m_error;
    bool m_wanted = false;
};
