#include "StageController.h"

#include <QBuffer>
#include <QCryptographicHash>
#include <QDateTime>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QKeyEvent>
#include <QMouseEvent>
#include <QQuickItem>
#include <QSaveFile>
#include <QQuickWindow>
#include <QWaylandClient>
#include <QWaylandCompositor>
#include <QWaylandQuickItem>
#include <QWaylandSeat>
#include <QWaylandSurface>
#include <QWaylandView>

#include <algorithm>
#include <cmath>

using namespace archie::visual;

namespace {

constexpr int kMaxClients = 8;
constexpr std::int64_t kInputIntervalMs = 16;
/// How long a capture may keep rendering past its hold before it counts as failed.
constexpr std::int64_t kCaptureTimeoutMs = 2'000;
/// How many damage rectangles are worth naming one by one before the bounding box will do.
constexpr int kMaxDamageRects = 16;

QString mode_name(AuthorityMode mode)
{
    switch (mode) {
    case AuthorityMode::Autonomous:
        return QStringLiteral("autonomous");
    case AuthorityMode::Operator:
        return QStringLiteral("operator");
    case AuthorityMode::Paused:
        return QStringLiteral("paused");
    }
    return QStringLiteral("paused");
}

QString reason_code(AuthorityReason reason)
{
    switch (reason) {
    case AuthorityReason::Start:
        return QStringLiteral("start");
    case AuthorityReason::OperatorMotion:
        return QStringLiteral("operator_motion");
    case AuthorityReason::OperatorButton:
        return QStringLiteral("operator_button");
    case AuthorityReason::OperatorKey:
        return QStringLiteral("operator_key");
    case AuthorityReason::OperatorIdle:
        return QStringLiteral("operator_idle");
    case AuthorityReason::PauseChord:
        return QStringLiteral("pause_chord");
    case AuthorityReason::Resumed:
        return QStringLiteral("resumed");
    }
    return QStringLiteral("start");
}

QString verb_code(CaptionVerb verb)
{
    switch (verb) {
    case CaptionVerb::Moving:
        return QStringLiteral("moving");
    case CaptionVerb::Clicking:
        return QStringLiteral("clicking");
    case CaptionVerb::DoubleClicking:
        return QStringLiteral("double_clicking");
    case CaptionVerb::Dragging:
        return QStringLiteral("dragging");
    case CaptionVerb::Scrolling:
        return QStringLiteral("scrolling");
    case CaptionVerb::Typing:
        return QStringLiteral("typing");
    case CaptionVerb::PressingKeys:
        return QStringLiteral("pressing_keys");
    case CaptionVerb::Focusing:
        return QStringLiteral("focusing");
    case CaptionVerb::Observing:
        return QStringLiteral("observing");
    case CaptionVerb::Interrupted:
        return QStringLiteral("interrupted");
    case CaptionVerb::Refused:
        return QStringLiteral("refused");
    }
    return QStringLiteral("moving");
}

Qt::MouseButton qt_button(PointerButton button)
{
    switch (button) {
    case PointerButton::Left:
        return Qt::LeftButton;
    case PointerButton::Middle:
        return Qt::MiddleButton;
    case PointerButton::Right:
        return Qt::RightButton;
    }
    return Qt::LeftButton;
}

/// The last dotted segment of an application id, which is how people name applications.
QString app_label(const std::string &app_id)
{
    const QString id = QString::fromStdString(app_id);
    const qsizetype dot = id.lastIndexOf(QLatin1Char('.'));
    return dot >= 0 ? id.mid(dot + 1) : id;
}

QByteArray line_of(const QJsonObject &object)
{
    QByteArray bytes = QJsonDocument(object).toJson(QJsonDocument::Compact);
    bytes.append('\n');
    return bytes;
}

} // namespace

StageController::StageController(Options options, QObject *parent)
    : QObject(parent)
    , m_options(std::move(options))
{
    DirectorPolicy policy;
    policy.reduced_motion = m_options.reduced_motion;
    m_director = StageDirector(policy);
    m_playbackPolicy = m_options.playback;

    m_clock.start();
    m_timer.setSingleShot(true);
    m_timer.setTimerType(Qt::PreciseTimer);
    connect(&m_timer, &QTimer::timeout, this, &StageController::tick);

    if (m_options.key.size() != 32) {
        qWarning("archie-stage: no command key; the agent cannot act on this stage");
        return;
    }
    if (m_options.control_socket.isEmpty()) {
        qWarning("archie-stage: no control socket; the agent cannot act on this stage");
        return;
    }
    const QFileInfo socket(m_options.control_socket);
    QDir().mkpath(socket.absolutePath());
    QFile::setPermissions(socket.absolutePath(),
                          QFileDevice::ReadOwner | QFileDevice::WriteOwner | QFileDevice::ExeOwner);
    QLocalServer::removeServer(m_options.control_socket);
    // Only this account may connect, and the key is still required for anything to run.
    m_server.setSocketOptions(QLocalServer::UserAccessOption);
    m_server.setMaxPendingConnections(kMaxClients);
    connect(&m_server, &QLocalServer::newConnection, this, &StageController::acceptConnection);
    if (!m_server.listen(m_options.control_socket)) {
        qWarning("archie-stage: cannot listen on %s: %s", qUtf8Printable(m_options.control_socket),
                 qUtf8Printable(m_server.errorString()));
    }
    emit listeningChanged();
}

StageController::~StageController()
{
    if (m_window) {
        m_window->removeEventFilter(this);
    }
}

std::int64_t StageController::wallclock()
{
    return QDateTime::currentMSecsSinceEpoch();
}

QString StageController::authority() const
{
    return mode_name(m_authority.mode());
}

void StageController::setCompositor(QWaylandCompositor *compositor)
{
    if (m_compositor == compositor) {
        return;
    }
    if (m_compositor) {
        disconnect(m_compositor, nullptr, this, nullptr);
    }
    m_compositor = compositor;
    if (m_compositor) {
        connect(m_compositor, &QWaylandCompositor::defaultSeatChanged, this, &StageController::watchSeat);
        watchSeat();
    }
    emit compositorChanged();
}

void StageController::watchSeat()
{
    QWaylandSeat *current = seat();
    if (current == m_watchedSeat) {
        return;
    }
    if (m_watchedSeat) {
        disconnect(m_watchedSeat, nullptr, this, nullptr);
    }
    m_typer.reset();
    m_watchedSeat = current;
    if (current) {
        connect(current, &QWaylandSeat::keyboardFocusChanged, this,
                [this](QWaylandSurface *newFocus, QWaylandSurface *) { handleKeyboardFocus(newFocus); });
    }
}

void StageController::handleKeyboardFocus(QWaylandSurface *newFocus)
{
    QJsonValue focused = QJsonValue::Null;
    m_focusedItem = nullptr;
    for (auto it = m_items.constBegin(); it != m_items.constEnd(); ++it) {
        if (it.value() && it.value()->surface() == newFocus) {
            focused = static_cast<qint64>(it.key());
            m_table.focus(it.key());
            m_focusedItem = it.value().data();
        }
    }
    emit focusChanged();
    broadcast(line_of(QJsonObject{{QStringLiteral("event"), QStringLiteral("focus")},
                                  {QStringLiteral("surface"), focused}}));
}

KeyTyper *StageController::typer()
{
    if (!m_typer && seat()) {
        m_typer = std::make_unique<KeyTyper>(seat());
    }
    return m_typer.get();
}

void StageController::setWindow(QQuickWindow *window)
{
    if (m_window == window) {
        return;
    }
    if (m_window) {
        m_window->removeEventFilter(this);
    }
    m_window = window;
    if (m_window) {
        m_window->installEventFilter(this);
        // The agent's is the only pointer on screen while it holds the stage.
        m_window->setCursor(Qt::BlankCursor);
    }
    emit windowChanged();
    tick();
}

void StageController::setDesk(QObject *desk)
{
    if (m_desk == desk) {
        return;
    }
    m_desk = desk;
    emit deskChanged();
}

QWaylandSeat *StageController::seat() const
{
    return m_compositor ? m_compositor->defaultSeat() : nullptr;
}

QWaylandQuickItem *StageController::itemFor(SurfaceId id) const
{
    const auto found = m_items.constFind(id);
    return found == m_items.constEnd() ? nullptr : found->data();
}

// ------------------------------------------------------------------------------------
// Surfaces
// ------------------------------------------------------------------------------------

namespace {

IRect desk_geometry(QWaylandQuickItem *item, QObject *desk)
{
    QPointF origin(item->x(), item->y());
    if (auto *deskItem = qobject_cast<QQuickItem *>(desk)) {
        origin = item->mapToItem(deskItem, QPointF(0, 0));
    }
    return IRect{static_cast<int>(std::lround(origin.x())), static_cast<int>(std::lround(origin.y())),
                 static_cast<int>(std::lround(item->width())), static_cast<int>(std::lround(item->height()))};
}

} // namespace

void StageController::registerSurface(QWaylandQuickItem *item, const QString &appId)
{
    if (!item || m_ids.contains(item)) {
        return;
    }
    const SurfaceId id = m_nextSurface++;
    m_items.insert(id, item);
    m_ids.insert(item, id);
    connect(item, &QObject::destroyed, this, [this, item] {
        const auto found = m_ids.constFind(item);
        if (found == m_ids.constEnd()) {
            return;
        }
        const SurfaceId gone = *found;
        m_ids.remove(item);
        m_items.remove(gone);
        m_pids.remove(gone);
        if (m_active && m_active->command.surface == gone) {
            cancelPlayback("surface_unmapped");
        }
        if (m_table.unmap(gone)) {
            QJsonObject event{{QStringLiteral("event"), QStringLiteral("unmapped")},
                              {QStringLiteral("surface"), static_cast<qint64>(gone)}};
            broadcast(line_of(event));
        }
        emit surfacesChanged();
    });
    if (QWaylandSurface *surface = item->surface()) {
        if (QWaylandClient *client = surface->client()) {
            m_pids.insert(id, static_cast<qint64>(client->processId()));
        }
        connect(surface, &QWaylandSurface::damaged, this, [this, id](const QRegion &region) {
            if (region.isEmpty() || !m_table.find(id)) {
                return;
            }
            // The rectangles themselves while there are few: perception asks whether one
            // control repainted, and a single bounding box over a whole window answers yes
            // to everything. Past that the bounding box is honest and much cheaper.
            QJsonArray rects;
            if (region.rectCount() <= kMaxDamageRects) {
                for (const QRect &rect : region) {
                    if (!rect.isEmpty()) {
                        rects.append(QJsonObject{{QStringLiteral("x"), rect.x()},
                                                 {QStringLiteral("y"), rect.y()},
                                                 {QStringLiteral("width"), rect.width()},
                                                 {QStringLiteral("height"), rect.height()}});
                    }
                }
            }
            if (rects.isEmpty()) {
                const QRect bounds = region.boundingRect();
                if (bounds.isEmpty()) {
                    return;
                }
                rects.append(QJsonObject{{QStringLiteral("x"), bounds.x()},
                                         {QStringLiteral("y"), bounds.y()},
                                         {QStringLiteral("width"), bounds.width()},
                                         {QStringLiteral("height"), bounds.height()}});
            }
            QJsonObject event{{QStringLiteral("event"), QStringLiteral("damage")},
                              {QStringLiteral("surface"), static_cast<qint64>(id)},
                              {QStringLiteral("rects"), rects}};
            broadcast(line_of(event));
        });
    }
    emit surfacesChanged();
    updateSurface(item, appId);
}

void StageController::updateSurface(QWaylandQuickItem *item, const QString &appId)
{
    const auto found = m_ids.constFind(item);
    if (found == m_ids.constEnd()) {
        return;
    }
    const SurfaceId id = *found;
    const IRect geometry = desk_geometry(item, m_desk);
    const std::string app = appId.toStdString();
    const Surface *known = m_table.find(id);
    const auto now = monotonic();

    if (geometry.empty()) {
        return;
    }
    if (!known || known->app_id != app) {
        // A surface that renames itself is not the surface a command was prepared for.
        if (known) {
            if (m_active && m_active->command.surface == id) {
                cancelPlayback("surface_changed");
            }
            m_table.unmap(id);
        }
        m_table.map(id, app, geometry);
    } else if (!(known->geometry == geometry)) {
        const auto revision = known->geometry_revision;
        m_table.move(id, geometry);
        if (m_active && m_active->command.surface == id && m_table.find(id)->geometry_revision != revision) {
            cancelPlayback("surface_changed");
        }
        m_director.surface_moved(*m_table.find(id), now);
    } else {
        return;
    }
    broadcast(surfaceEvent(id));
    tick();
}

void StageController::unregisterSurface(QWaylandQuickItem *item)
{
    const auto found = m_ids.constFind(item);
    if (found == m_ids.constEnd()) {
        return;
    }
    const SurfaceId id = *found;
    if (m_active && m_active->command.surface == id) {
        cancelPlayback("surface_unmapped");
    }
    m_ids.remove(item);
    m_items.remove(id);
    m_pids.remove(id);
    if (m_table.unmap(id)) {
        broadcast(line_of(QJsonObject{{QStringLiteral("event"), QStringLiteral("unmapped")},
                                      {QStringLiteral("surface"), static_cast<qint64>(id)}}));
    }
    emit surfacesChanged();
    tick();
}

void StageController::setOutputSize(int width, int height)
{
    if (width <= 0 || height <= 0) {
        return;
    }
    m_outputWidth = width;
    m_outputHeight = height;
    m_director.set_output(width, height, monotonic());
    tick();
}

QByteArray StageController::surfaceEvent(SurfaceId id) const
{
    const Surface *surface = m_table.find(id);
    if (!surface) {
        return {};
    }
    return line_of(QJsonObject{
        {QStringLiteral("event"), QStringLiteral("surface")},
        {QStringLiteral("surface"), static_cast<qint64>(id)},
        {QStringLiteral("app_id"), QString::fromStdString(surface->app_id)},
        {QStringLiteral("x"), surface->geometry.x},
        {QStringLiteral("y"), surface->geometry.y},
        {QStringLiteral("width"), surface->geometry.w},
        {QStringLiteral("height"), surface->geometry.h},
        {QStringLiteral("geometry_revision"), static_cast<qint64>(surface->geometry_revision)},
        // Which process draws it: how something that reads an application's own account of
        // its controls, on another bus, finds the same window.
        {QStringLiteral("pid"), m_pids.value(id, 0)},
    });
}

// ------------------------------------------------------------------------------------
// The control socket
// ------------------------------------------------------------------------------------

void StageController::acceptConnection()
{
    while (QLocalSocket *socket = m_server.nextPendingConnection()) {
        if (m_clients.size() >= kMaxClients) {
            socket->disconnectFromServer();
            socket->deleteLater();
            continue;
        }
        m_clients.append(socket);
        connect(socket, &QLocalSocket::readyRead, this, [this, socket] { readFrom(socket); });
        connect(socket, &QLocalSocket::disconnected, this, [this, socket] {
            m_buffers.remove(socket);
            m_clients.removeAll(socket);
            socket->deleteLater();
        });
        greet(socket);
    }
}

void StageController::greet(QLocalSocket *socket)
{
    QByteArray hello;
    for (const Surface *surface : m_table.mapped_surfaces()) {
        hello += surfaceEvent(surface->id);
    }
    const auto focused = m_table.focused();
    hello += line_of(QJsonObject{{QStringLiteral("event"), QStringLiteral("focus")},
                                 {QStringLiteral("surface"),
                                  focused ? QJsonValue(static_cast<qint64>(*focused)) : QJsonValue::Null}});
    hello += line_of(QJsonObject{{QStringLiteral("event"), QStringLiteral("authority")},
                                 {QStringLiteral("mode"), mode_name(m_authority.mode())},
                                 {QStringLiteral("reason"), reason_code(m_authority.reason())}});
    socket->write(hello);
}

void StageController::broadcast(const QByteArray &line)
{
    if (line.isEmpty()) {
        return;
    }
    for (const auto &client : std::as_const(m_clients)) {
        if (client && client->state() == QLocalSocket::ConnectedState) {
            client->write(line);
        }
    }
}

void StageController::readFrom(QLocalSocket *socket)
{
    QByteArray &buffer = m_buffers[socket];
    buffer += socket->readAll();
    while (true) {
        const qsizetype newline = buffer.indexOf('\n');
        if (newline < 0) {
            if (buffer.size() > archie::stage::kMaxEnvelopeBytes) {
                // No framing can be trusted after an unbounded line.
                buffer.clear();
                socket->disconnectFromServer();
            }
            return;
        }
        const QByteArray line = buffer.left(newline + 1);
        buffer.remove(0, newline + 1);
        handleLine(socket, line);
    }
}

QByteArray StageController::receipt(std::uint64_t sequence, const QString &outcome) const
{
    return line_of(QJsonObject{{QStringLiteral("event"), QStringLiteral("receipt")},
                               {QStringLiteral("seq"), static_cast<qint64>(sequence)},
                               {QStringLiteral("outcome"), outcome},
                               {QStringLiteral("observed_revision"), static_cast<qint64>(m_table.revision())}});
}

void StageController::sendReceipt(std::uint64_t sequence, const QString &outcome)
{
    broadcast(receipt(sequence, outcome));
}

void StageController::handleLine(QLocalSocket *socket, const QByteArray &line)
{
    auto decoded = archie::stage::decode_command(line, m_options.key, wallclock());
    if (!decoded.command) {
        // Nothing about an unverified line can be believed, including its sequence. It is
        // answered under sequence 0, and only to the connection that sent it, so a watcher
        // never mistakes someone else's bad line for news about its own command.
        if (socket->state() == QLocalSocket::ConnectedState) {
            socket->write(receipt(0, decoded.error));
        }
        return;
    }
    const auto now = monotonic();
    const auto before = m_authority.mode();
    m_authority.tick(now);
    noteAuthority(before);

    const StageCommand &command = decoded.command->command;
    if (m_playback) {
        // One action at a time. A queue would run commands against a stage that has
        // moved on since they were prepared.
        sendReceipt(command.sequence, QStringLiteral("stage_busy"));
        return;
    }
    auto refusal = check_command(command, m_table, m_authority, m_lastSequence, wallclock());
    if (!refusal && m_physicalKeysHeld > 0) {
        refusal = CommandRefusal::OperatorHasControl;
    }
    if (!refusal) {
        refusal = undeliverable(*decoded.command);
    }
    if (refusal) {
        m_director.command_refused(command, *refusal, now);
        sendReceipt(command.sequence, QString::fromLatin1(refusal_code(*refusal)));
        tick();
        return;
    }
    m_lastSequence = command.sequence;
    const Surface *surface = m_table.find(command.surface);
    m_active = std::move(*decoded.command);
    m_textRoute.reset();
    m_commandSocket = socket;
    m_capture.reset();
    m_playback = Playback::start(m_active->command, *surface, m_pointer, now, m_playbackPolicy);
    m_director.command_started(m_active->command, *surface, *m_playback, now);
    tick();
}

// ------------------------------------------------------------------------------------
// Input
// ------------------------------------------------------------------------------------

std::optional<CommandRefusal> StageController::undeliverable(const archie::stage::DecodedCommand &decoded)
{
    const StageCommand &command = decoded.command;
    if (command.action != StageAction::Text && command.action != StageAction::Key) {
        return std::nullopt;
    }
    QWaylandQuickItem *item = itemFor(command.surface);
    KeyTyper *keys = typer();
    if (!item || !keys) {
        return CommandRefusal::NotTypeable;
    }
    // Decided before anything is pressed, so a command either types whole or not at all —
    // except for text-input, whose readiness only shows once the surface has focus.
    for (const char32_t unit : decoded.text) {
        if (!keys->mayType(unit, item->surface())) {
            return CommandRefusal::NotTypeable;
        }
    }
    for (const int key : decoded.keys) {
        if (!keys->hasKey(key)) {
            return CommandRefusal::NotTypeable;
        }
    }
    return std::nullopt;
}

void StageController::deliver(const InputStep &step)
{
    QWaylandSeat *s = seat();
    if (!s || !m_active) {
        return;
    }
    QWaylandQuickItem *item = itemFor(m_active->command.surface);
    const Surface *surface = m_table.find(m_active->command.surface);
    if (!item || !surface) {
        return;
    }
    switch (step.kind) {
    case StepKind::Motion: {
        m_pointer = step.point;
        const QPointF local(step.point.x - static_cast<float>(surface->geometry.x),
                           step.point.y - static_cast<float>(surface->geometry.y));
        const bool over = local.x() >= 0 && local.y() >= 0 && local.x() < surface->geometry.w &&
                          local.y() < surface->geometry.h;
        // A pointer on its way to a window is not over it yet, and the application hears
        // nothing until it is. A drag in progress keeps its window, as a held button does.
        if (over || (m_playback && m_playback->button_held())) {
            s->sendMouseMoveEvent(item->view(), local, QPointF(step.point.x, step.point.y));
        } else {
            s->setMouseFocus(nullptr);
        }
        break;
    }
    case StepKind::Press:
        s->sendMousePressEvent(qt_button(step.button));
        break;
    case StepKind::Release:
        s->sendMouseReleaseEvent(qt_button(step.button));
        break;
    case StepKind::Wheel:
        if (step.wheel_y != 0) {
            s->sendMouseWheelEvent(Qt::Vertical, -120 * step.wheel_y);
        }
        if (step.wheel_x != 0) {
            s->sendMouseWheelEvent(Qt::Horizontal, -120 * step.wheel_x);
        }
        break;
    case StepKind::FocusSurface:
        // Keyboard focus and the application's input method, together: a click from a person
        // does both, and an input method that never hears of the focus never turns on.
        item->takeFocus(s);
        break;
    case StepKind::Capture: {
        // The surface item alone, at its own size: the window as its application drew it,
        // with no camera zoom, cursor, caption or neighbouring window in it. A command may
        // ask for one box of it instead, and then only that box is kept.
        m_capture = Capture{m_active->command.sequence, m_active->command.surface, m_commandSocket};
        m_capture->region = snapshot_region(m_active->command, *surface);
        m_capture->surface_width = surface->geometry.w;
        m_capture->surface_height = surface->geometry.h;
        m_capture->asked_at_ms = QDateTime::currentMSecsSinceEpoch();
        m_grab = item->grabToImage();
        if (!m_grab) {
            m_capture->done = true;
            break;
        }
        const std::uint64_t sequence = m_active->command.sequence;
        connect(m_grab.data(), &QQuickItemGrabResult::ready, this, [this, sequence] { finishCapture(sequence); });
        break;
    }
    case StepKind::TextUnit:
        if (step.index < static_cast<std::size_t>(m_active->text.size())) {
            const char32_t unit = m_active->text.at(static_cast<qsizetype>(step.index));
            KeyTyper *keys = typer();
            if (keys && !m_textRoute) {
                // Chosen once the surface has had time to take focus, before the first
                // character, so a text is typed whole or not at all.
                m_textRoute = keys->routeFor(m_active->text);
            }
            if (!keys || !m_textRoute || !keys->type(unit, *m_textRoute)) {
                failPlayback(CommandRefusal::NotTypeable);
            }
        }
        break;
    case StepKind::KeyPress:
    case StepKind::KeyRelease:
        if (step.index < static_cast<std::size_t>(m_active->keys.size())) {
            const int key = m_active->keys.at(static_cast<qsizetype>(step.index));
            const bool pressed = step.kind == StepKind::KeyPress;
            KeyTyper *keys = typer();
            if ((!keys || !keys->key(key, pressed)) && pressed) {
                failPlayback(CommandRefusal::NotTypeable);
            }
        }
        break;
    }
}

QString StageController::snapshotDirectory() const
{
    return QFileInfo(m_options.control_socket).absolutePath() + QStringLiteral("/snapshots");
}

void StageController::finishCapture(std::uint64_t sequence)
{
    if (!m_capture || m_capture->sequence != sequence || !m_grab) {
        return; // abandoned: the command stopped while the frame was rendering
    }
    const QImage grabbed = m_grab->image();
    m_grab.reset();
    m_capture->done = true;
    if (grabbed.isNull()) {
        return;
    }
    const qint64 grabbed_at_ms = QDateTime::currentMSecsSinceEpoch();
    // The window as the compositor rendered it may be larger than the window as the
    // application measures it; the part asked for is in the application's pixels.
    const double sx = m_capture->surface_width > 0
                          ? static_cast<double>(grabbed.width()) / m_capture->surface_width
                          : 1.0;
    const double sy = m_capture->surface_height > 0
                          ? static_cast<double>(grabbed.height()) / m_capture->surface_height
                          : 1.0;
    const QRect part = QRect(qRound(m_capture->region.x * sx), qRound(m_capture->region.y * sy),
                             qRound(m_capture->region.w * sx), qRound(m_capture->region.h * sy))
                           .intersected(grabbed.rect());
    if (part.isEmpty()) {
        return;
    }
    const QImage image = part == grabbed.rect() ? grabbed : grabbed.copy(part);
    QByteArray png;
    {
        QBuffer buffer(&png);
        if (!buffer.open(QIODevice::WriteOnly) || !image.save(&buffer, "PNG")) {
            return;
        }
    }
    const qint64 encoded_at_ms = QDateTime::currentMSecsSinceEpoch();
    // Beside the control socket, in a directory only this account can enter. The pixels go
    // to one file for the connection that asked; that client verifies the digest it is
    // given and deletes the file.
    const QString directory = snapshotDirectory();
    if (!QDir().mkpath(directory) ||
        !QFile::setPermissions(directory, QFileDevice::ReadOwner | QFileDevice::WriteOwner | QFileDevice::ExeOwner)) {
        return;
    }
    const QString path = directory + QStringLiteral("/") + QString::number(sequence) + QStringLiteral(".png");
    QSaveFile file(path);
    if (!file.open(QIODevice::WriteOnly) || file.write(png) != png.size() || !file.commit()) {
        return;
    }
    QFile::setPermissions(path, QFileDevice::ReadOwner | QFileDevice::WriteOwner);
    m_capture->ok = true;
    if (m_capture->socket && m_capture->socket->state() == QLocalSocket::ConnectedState) {
        m_capture->socket->write(line_of(QJsonObject{
            {QStringLiteral("event"), QStringLiteral("snapshot")},
            {QStringLiteral("seq"), static_cast<qint64>(sequence)},
            {QStringLiteral("surface"), static_cast<qint64>(m_capture->surface)},
            {QStringLiteral("x"), m_capture->region.x},
            {QStringLiteral("y"), m_capture->region.y},
            {QStringLiteral("width"), image.width()},
            {QStringLiteral("height"), image.height()},
            {QStringLiteral("sha256"),
             QString::fromLatin1(QCryptographicHash::hash(png, QCryptographicHash::Sha256).toHex())},
            {QStringLiteral("path"), path},
        }));
    }
    // What a capture costs, split where it can actually be changed: rendering the window
    // is the compositor's, encoding and writing is what a smaller region saves.
    qInfo("archie-stage: capture %llu %dx%d at %d,%d render %lld ms encode %lld ms write %lld ms %lld bytes",
          static_cast<unsigned long long>(sequence), image.width(), image.height(),
          m_capture->region.x, m_capture->region.y, grabbed_at_ms - m_capture->asked_at_ms,
          encoded_at_ms - grabbed_at_ms, QDateTime::currentMSecsSinceEpoch() - encoded_at_ms,
          static_cast<long long>(png.size()));
    tick();
}

void StageController::cancelPlayback(const char *outcome)
{
    stopPlayback(QString::fromLatin1(outcome), std::nullopt);
}

void StageController::failPlayback(CommandRefusal refusal)
{
    stopPlayback(QString::fromLatin1(refusal_code(refusal)), refusal);
}

void StageController::stopPlayback(const QString &outcome, std::optional<CommandRefusal> failure)
{
    if (!m_playback || !m_active) {
        return;
    }
    const auto now = monotonic();
    // Whatever is held is let go: buttons and keys the playback pressed, and any level key
    // a character was halfway through.
    const std::vector<InputStep> releases = m_playback->cancel(now);
    for (const InputStep &step : releases) {
        deliver(step);
    }
    if (m_typer) {
        m_typer->releaseAll();
    }
    sendReceipt(m_active->command.sequence, outcome);
    if (failure) {
        m_director.command_failed(*failure, now);
    } else {
        m_director.command_ended(PlaybackState::Cancelled, now);
    }
    m_pointer = m_playback->pointer_at(now);
    m_playback.reset();
    // The text and keys are dropped with the playback that needed them, and a capture
    // still rendering is abandoned: its result is never written.
    m_active.reset();
    m_textRoute.reset();
    m_capture.reset();
    m_grab.reset();
}

void StageController::noteAuthority(AuthorityMode before)
{
    const AuthorityMode mode = m_authority.mode();
    if (mode == before) {
        return;
    }
    const auto now = monotonic();
    // Who holds the stage, and why, belongs in the log: a takeover nobody remembers making
    // is the first thing to look for when the agent seemed to stop on its own.
    qInfo("archie-stage: authority %s (%s)", qUtf8Printable(mode_name(mode)),
          qUtf8Printable(reason_code(m_authority.reason())));
    m_director.authority_changed(mode, now);
    if (mode != AuthorityMode::Autonomous) {
        cancelPlayback("preempted_by_operator");
    }
    if (m_window) {
        if (mode == AuthorityMode::Autonomous) {
            m_window->setCursor(Qt::BlankCursor);
        } else {
            m_window->unsetCursor();
        }
    }
    m_havePhysicalPointer = false;
    broadcast(line_of(QJsonObject{{QStringLiteral("event"), QStringLiteral("authority")},
                                  {QStringLiteral("mode"), mode_name(mode)},
                                  {QStringLiteral("reason"), reason_code(m_authority.reason())}}));
    emit authorityChanged();
}

bool StageController::eventFilter(QObject *watched, QEvent *event)
{
    if (watched != m_window) {
        return QObject::eventFilter(watched, event);
    }
    const auto now = monotonic();
    const AuthorityMode before = m_authority.mode();
    bool input = false;

    switch (event->type()) {
    case QEvent::MouseMove: {
        const QPointF position = static_cast<QMouseEvent *>(event)->position();
        if (m_havePhysicalPointer) {
            m_authority.physical_motion(static_cast<float>(position.x() - m_lastPhysicalPointer.x()),
                                        static_cast<float>(position.y() - m_lastPhysicalPointer.y()), now);
        }
        m_lastPhysicalPointer = position;
        m_havePhysicalPointer = true;
        input = true;
        break;
    }
    case QEvent::MouseButtonPress:
    case QEvent::MouseButtonDblClick:
    case QEvent::Wheel:
    case QEvent::TouchBegin:
        m_authority.physical_button(now);
        input = true;
        break;
    case QEvent::MouseButtonRelease:
    case QEvent::TouchUpdate:
    case QEvent::TouchEnd:
        input = true;
        break;
    case QEvent::KeyRelease:
        if (!static_cast<QKeyEvent *>(event)->isAutoRepeat() && m_physicalKeysHeld > 0) {
            --m_physicalKeysHeld;
        }
        input = true;
        break;
    case QEvent::KeyPress: {
        const auto *key = static_cast<QKeyEvent *>(event);
        if (!key->isAutoRepeat()) {
            ++m_physicalKeysHeld;
        }
        const Qt::KeyboardModifiers modifiers = key->modifiers();
        const bool ctrl_alt = modifiers.testFlag(Qt::ControlModifier) && modifiers.testFlag(Qt::AltModifier);
        if (key->key() == Qt::Key_Pause || (ctrl_alt && key->key() == Qt::Key_Escape)) {
            m_authority.pause_chord(now);
            noteAuthority(before);
            tick();
            return true;
        }
        if (ctrl_alt && (key->key() == Qt::Key_Return || key->key() == Qt::Key_Enter)) {
            m_authority.resume(now);
            noteAuthority(before);
            tick();
            return true;
        }
        // Every key counts, modifiers included. The compositor sends a held modifier's state
        // to the focused application no matter what happens to this event, so a hand on Ctrl
        // would turn the agent's next "c" into a copy. Starting the pause chord takes the
        // stage first; the chord still pauses it.
        m_authority.physical_key(now);
        input = true;
        break;
    }
    case QEvent::FocusOut:
    case QEvent::WindowDeactivate:
        // Keys released while the window was not listening never report their release.
        m_physicalKeysHeld = 0;
        break;
    case QEvent::Leave:
        m_havePhysicalPointer = false;
        break;
    default:
        break;
    }
    if (!input) {
        return QObject::eventFilter(watched, event);
    }
    const bool agent_held_the_stage = before == AuthorityMode::Autonomous;
    if (m_authority.mode() != before) {
        noteAuthority(before);
        tick();
    }
    // While the agent holds the stage, physical input is watched and never delivered. The
    // event that hands the stage over is not delivered either: it was a person reaching for
    // the machine, not an instruction to whatever was under the pointer.
    return agent_held_the_stage;
}

// ------------------------------------------------------------------------------------
// Frames
// ------------------------------------------------------------------------------------

void StageController::tick()
{
    const auto now = monotonic();
    const AuthorityMode before = m_authority.mode();
    m_authority.tick(now);
    noteAuthority(before);

    if (m_playback) {
        const std::vector<InputStep> steps = m_playback->due(now);
        for (const InputStep &step : steps) {
            if (!m_playback) {
                break; // a step that could not be delivered stopped the playback
            }
            deliver(step);
        }
        if (m_playback && m_playback->state() == PlaybackState::Finished) {
            const bool capture = m_active->command.action == StageAction::Snapshot;
            if (capture && !(m_capture && m_capture->ok)) {
                // A capture renders on the next frame, which is usually long done by now.
                // One that is still pending gets a little longer; one that failed, or never
                // finishes, is reported as failed, never as executed.
                const bool pending = m_capture && !m_capture->done && now - m_playback->started_ms() < kCaptureTimeoutMs;
                if (!pending) {
                    failPlayback(CommandRefusal::CaptureFailed);
                }
            } else {
                sendReceipt(m_active->command.sequence, QStringLiteral("executed"));
                m_director.command_ended(PlaybackState::Finished, now);
                m_playback.reset();
                m_active.reset();
                m_textRoute.reset();
                m_capture.reset();
                m_grab.reset();
            }
        }
    }
    m_director.tick(now);
    const StageFrame frame = m_director.frame(now, m_options.tier);
    publish(frame);

    std::optional<std::int64_t> next = frame.next_frame_at_ms;
    const auto sooner = [&next](std::int64_t at) {
        if (!next || at < *next) {
            next = at;
        }
    };
    if (m_playback) {
        sooner(now + kInputIntervalMs);
    }
    if (const auto resume = m_authority.resumes_at_ms()) {
        sooner(*resume);
    }
    schedule(next);
}

void StageController::schedule(std::optional<std::int64_t> at)
{
    if (!at) {
        m_timer.stop();
        return;
    }
    const auto delay = std::max<std::int64_t>(0, *at - monotonic());
    m_timer.start(static_cast<int>(std::min<std::int64_t>(delay, 60'000)));
}

void StageController::publish(const StageFrame &frame)
{
    m_frame = frame;
    const VisibleRegion region = visible_region(frame.view, m_outputWidth, m_outputHeight);
    m_viewX = region.x;
    m_viewY = region.y;
    m_viewZoom = std::max(1.0F, frame.view.zoom);

    const PointF cursor = to_screen(frame.view, m_outputWidth, m_outputHeight, frame.cursor);
    m_cursorScreenX = cursor.x;
    m_cursorScreenY = cursor.y;
    if (frame.drag_from) {
        const PointF from = to_screen(frame.view, m_outputWidth, m_outputHeight, *frame.drag_from);
        m_dragScreenX = from.x;
        m_dragScreenY = from.y;
    }

    m_ripples.clear();
    for (const Ripple &ripple : frame.ripples) {
        const PointF center = to_screen(frame.view, m_outputWidth, m_outputHeight, ripple.center);
        m_ripples.append(QVariantMap{{QStringLiteral("x"), center.x},
                                     {QStringLiteral("y"), center.y},
                                     {QStringLiteral("r"), ripple.radius * m_viewZoom},
                                     {QStringLiteral("opacity"), ripple.opacity}});
    }

    if (frame.caption) {
        m_captionVerb = verb_code(frame.caption->verb);
        m_captionApp = app_label(frame.caption->app_id);
        m_captionCount = frame.caption->verb == CaptionVerb::Typing ? static_cast<int>(frame.caption->text_chars)
                                                                    : frame.caption->key_count;
        m_captionRefusal = frame.caption->refusal ? QString::fromLatin1(refusal_code(*frame.caption->refusal))
                                                  : QString();
        m_captionOpacity = frame.caption->opacity;
    } else {
        m_captionOpacity = 0;
    }
    m_activity.clear();
    for (const Caption &entry : frame.activity) {
        const int count = entry.text_chars > 0 ? static_cast<int>(entry.text_chars) : std::max(0, entry.key_count);
        m_activity.append(QVariantMap{
            {QStringLiteral("verb"), verb_code(entry.verb)},
            {QStringLiteral("app"), app_label(entry.app_id)},
            {QStringLiteral("count"), count},
            {QStringLiteral("refusal"),
             entry.refusal ? QString::fromLatin1(refusal_code(*entry.refusal)) : QString()},
            {QStringLiteral("opacity"), entry.opacity},
        });
    }
    m_banner = frame.banner ? mode_name(*frame.banner) : QString();
    emit frameChanged();
}
