#include "ShellRuntime.h"
#include <QJsonDocument>
#include <QJsonObject>
#include <QStandardPaths>
#include <QStringDecoder>

namespace {
constexpr qsizetype maximumFrameBytes = 1024 * 1024;
constexpr qsizetype maximumErrorBytes = 4096;
}

ShellRuntime::ShellRuntime(QString program, QString lanFile, QString workspacesRoot, QObject *parent)
    : QObject(parent) {
    m_process.setProgram(program);
    m_process.setArguments({QStringLiteral("watch"), QStringLiteral("--lan-file"), lanFile,
                           QStringLiteral("--workspaces-root"), workspacesRoot});
    m_restart.setSingleShot(true);
    m_restart.setInterval(3000);
    m_stop.setSingleShot(true);
    m_stop.setInterval(500);
    connect(&m_restart, &QTimer::timeout, this, &ShellRuntime::start);
    connect(&m_stop, &QTimer::timeout, this, [this] { m_process.kill(); });
    connect(&m_process, &QProcess::stateChanged, this, [this] { emit runningChanged(); });
    connect(&m_process, &QProcess::started, this, [this] { setError({}); });
    connect(&m_process, &QProcess::readyReadStandardOutput, this, &ShellRuntime::readFrames);
    connect(&m_process, &QProcess::readyReadStandardError, this, [this] {
        const auto diagnostic = m_process.readAllStandardError().right(maximumErrorBytes);
        if (m_wanted && !diagnostic.isEmpty()) setError(QString::fromUtf8(diagnostic).trimmed());
    });
    connect(&m_process, &QProcess::errorOccurred, this, [this](QProcess::ProcessError error) {
        if (!m_wanted) return;
        setError(m_process.errorString());
        if (error == QProcess::FailedToStart) reconnect();
    });
    connect(&m_process, qOverload<int, QProcess::ExitStatus>(&QProcess::finished), this,
            [this](int code, QProcess::ExitStatus status) {
        m_stop.stop();
        m_pending.clear();
        if (!m_wanted) return;
        if (m_error.isEmpty()) setError(QStringLiteral("shell bridge exited (%1, %2)")
            .arg(code).arg(status == QProcess::CrashExit ? QStringLiteral("crash") : QStringLiteral("exit")));
        reconnect();
    });
}

void ShellRuntime::aiPolicyRequest(const QString &verb)
{
    if (m_aiProcess) return; // one request at a time; the next click waits for this answer
    const QString archie = QStandardPaths::findExecutable(QStringLiteral("archie"));
    if (archie.isEmpty()) {
        m_aiPolicy = QStringLiteral("unknown");
        m_aiPolicyNote = QStringLiteral("La CLI archie no está instalada en este equipo.");
        emit aiPolicyChanged();
        return;
    }
    m_aiProcess = new QProcess(this);
    m_aiProcess->setProgram(archie);
    m_aiProcess->setArguments({QStringLiteral("--json"), QStringLiteral("ai"), verb});
    m_aiProcess->setProcessChannelMode(QProcess::SeparateChannels);
    auto *process = m_aiProcess;
    QTimer::singleShot(15000, process, [process] { process->kill(); });
    connect(process, qOverload<int, QProcess::ExitStatus>(&QProcess::finished), this,
            [this, process, verb](int code, QProcess::ExitStatus status) {
        const QByteArray out = process->readAllStandardOutput().left(4096);
        const QString err = QString::fromUtf8(process->readAllStandardError().right(400)).trimmed();
        const QJsonObject answer = QJsonDocument::fromJson(out.trimmed()).object();
        if (status == QProcess::NormalExit && code == 0 && answer.contains(QStringLiteral("paused"))) {
            const bool paused = answer.value(QStringLiteral("paused")).toBool();
            const bool recorded = answer.value(QStringLiteral("recorded")).toBool();
            m_aiPolicy = paused ? QStringLiteral("paused") : QStringLiteral("running");
            m_aiPolicyNote = paused == recorded ? QString()
                : (paused ? QStringLiteral("Pausada, pero sin registro: un reinicio la levantaría.")
                          : QStringLiteral("En marcha, pero queda un registro de pausa: un reinicio pausaría."));
        } else {
            m_aiPolicy = QStringLiteral("unknown");
            m_aiPolicyNote = (verb == QLatin1String("status") ? QStringLiteral("Core no respondió: ")
                                                              : QStringLiteral("Core rechazó la orden: "))
                + (err.isEmpty() ? QStringLiteral("sin respuesta") : err.split(QLatin1Char('\n')).last());
        }
        m_aiProcess = nullptr;
        process->deleteLater();
        emit aiPolicyChanged();
    });
    connect(process, &QProcess::errorOccurred, this, [this, process](QProcess::ProcessError error) {
        if (error != QProcess::FailedToStart) return;
        m_aiPolicy = QStringLiteral("unknown");
        m_aiPolicyNote = QStringLiteral("No se pudo ejecutar archie: ") + process->errorString();
        m_aiProcess = nullptr;
        process->deleteLater();
        emit aiPolicyChanged();
    });
    process->start();
    emit aiPolicyChanged();
}

ShellRuntime::~ShellRuntime()
{
    m_wanted = false;
    m_restart.stop();
    if (m_aiProcess) {
        m_aiProcess->kill();
        m_aiProcess->waitForFinished(500);
    }
    if (m_process.state() != QProcess::NotRunning) {
        m_process.kill();
        m_process.waitForFinished(500);
    }
}

bool ShellRuntime::running() const { return m_process.state() == QProcess::Running; }

void ShellRuntime::setError(const QString &message)
{
    if (message == m_error) return;
    m_error = message;
    emit errorChanged();
}

void ShellRuntime::setRunning(bool enabled)
{
    if (m_wanted == enabled) return;
    m_wanted = enabled;
    if (enabled) {
        start();
    } else {
        m_restart.stop();
        m_pending.clear();
        if (m_process.state() != QProcess::NotRunning) {
            m_process.terminate();
            m_stop.start();
        }
    }
}

void ShellRuntime::start()
{
    if (!m_wanted || m_process.state() != QProcess::NotRunning) return;
    m_pending.clear();
    m_process.start();
}

void ShellRuntime::reconnect()
{
    if (m_wanted && !m_restart.isActive()) m_restart.start();
}

void ShellRuntime::readFrames()
{
    while (m_process.bytesAvailable() > 0) {
        const QByteArray chunk = m_process.read(64 * 1024);
        if (!m_wanted) continue;
        m_pending.append(chunk);
        while (true) {
            const qsizetype end = m_pending.indexOf('\n');
            if (end < 0) break;
            if (end > maximumFrameBytes) break;
            QByteArray line = m_pending.left(end);
            m_pending.remove(0, end + 1);
            if (line.endsWith('\r')) line.chop(1);
            QStringDecoder decode(QStringDecoder::Utf8);
            const QString text = decode(line);
            if (decode.hasError()) {
                setError(QStringLiteral("shell bridge emitted invalid UTF-8"));
                m_pending.clear();
                m_process.kill();
                return;
            }
            if (!line.isEmpty()) emit frameReady(text);
        }
        if (m_pending.size() > maximumFrameBytes) {
            setError(QStringLiteral("shell bridge frame exceeded 1 MiB"));
            m_pending.clear();
            m_process.kill();
            return;
        }
    }
}
