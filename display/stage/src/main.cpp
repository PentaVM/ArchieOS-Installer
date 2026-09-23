// `archie-stage` — the compositor Archie works in.
//
// A Wayland compositor built on Qt Wayland Compositor. Applications connect to its socket
// like any desktop; Archie operates them through sealed commands on its control socket;
// a person watches, and takes over with a hand on the mouse or the keyboard.
//
// Runs on DRM/KMS through EGLFS on a node, or as a window inside another Wayland session
// (WSLg, a development desktop), where the applications it hosts appear inside that
// window. The behaviour is the same in both.

#include <QCommandLineParser>
#include <QFile>
#include <QGuiApplication>
#include <QScreen>
#include <QQmlApplicationEngine>
#include <QQuickWindow>
#include <QStandardPaths>
#include <QTimer>

#include "CommandCodec.h"
#include "StageController.h"

int main(int argc, char *argv[])
{
    QGuiApplication app(argc, argv);
    QGuiApplication::setApplicationName(QStringLiteral("archie-stage"));
    QGuiApplication::setDesktopFileName(QStringLiteral("org.archieos.Stage"));

    QCommandLineParser parser;
    parser.setApplicationDescription(QStringLiteral("The Archie Stage compositor"));
    parser.addHelpOption();
    const QCommandLineOption socketName(QStringLiteral("socket-name"),
                                        QStringLiteral("Wayland socket applications connect to."),
                                        QStringLiteral("name"), QStringLiteral("archie-stage-0"));
    const QCommandLineOption control(QStringLiteral("control"), QStringLiteral("Control socket path."),
                                     QStringLiteral("path"));
    const QCommandLineOption keyFile(QStringLiteral("key-file"),
                                     QStringLiteral("File holding the 32-byte command key as hex."),
                                     QStringLiteral("path"));
    const QCommandLineOption reducedMotion(QStringLiteral("reduced-motion"),
                                           QStringLiteral("Cut instead of easing; no ripples."));
    const QCommandLineOption tier(QStringLiteral("tier"), QStringLiteral("normal, conserve or ultra."),
                                  QStringLiteral("tier"), QStringLiteral("normal"));
    const QCommandLineOption tempo(QStringLiteral("tempo"),
                                   QStringLiteral("person, to be watched, or agent, four actions a second."),
                                   QStringLiteral("tempo"), QStringLiteral("person"));
    const QCommandLineOption strokeSpeed(
        QStringLiteral("stroke-speed"),
        QStringLiteral("How fast a line is drawn, in pixels a second. An application sees the pointer at its "
                       "own frame rate, so a line drawn faster than about ten pixels a frame reaches it as "
                       "a polygon of its corners."),
        QStringLiteral("px/s"));
    const QCommandLineOption windowed(QStringLiteral("windowed"), QStringLiteral("Do not go full screen."));
    const QCommandLineOption screen(
        QStringLiteral("screen"),
        QStringLiteral("Which screen to take, counting from zero. Left out, Archie takes the last "
                       "one when there is more than one, so the person keeps the first."),
        QStringLiteral("n"));
    const QCommandLineOption width(QStringLiteral("width"), QStringLiteral("Window width."), QStringLiteral("px"),
                                   QStringLiteral("1600"));
    const QCommandLineOption height(QStringLiteral("height"), QStringLiteral("Window height."),
                                    QStringLiteral("px"), QStringLiteral("900"));
    const QCommandLineOption grab(QStringLiteral("grab"), QStringLiteral("Save a PNG of the stage."),
                                  QStringLiteral("path"));
    const QCommandLineOption grabAfter(QStringLiteral("grab-after-ms"), QStringLiteral("When to grab."),
                                       QStringLiteral("ms"), QStringLiteral("3000"));
    const QCommandLineOption grabEvery(QStringLiteral("grab-every-ms"),
                                       QStringLiteral("Keep grabbing to numbered files at this interval."),
                                       QStringLiteral("ms"));
    const QCommandLineOption quitAfter(QStringLiteral("quit-after-ms"), QStringLiteral("Exit after this long."),
                                       QStringLiteral("ms"));
    parser.addOptions({socketName, control, keyFile, reducedMotion, tier, tempo, strokeSpeed, windowed, screen, width, height, grab, grabAfter,
                       grabEvery, quitAfter});
    parser.process(app);

    StageController::Options options;
    options.control_socket = parser.value(control);
    if (options.control_socket.isEmpty()) {
        const QString runtime = QStandardPaths::writableLocation(QStandardPaths::RuntimeLocation);
        options.control_socket = runtime + QStringLiteral("/archie-stage/control.sock");
    }
    if (parser.isSet(keyFile)) {
        QFile file(parser.value(keyFile));
        if (file.open(QIODevice::ReadOnly)) {
            if (const auto key = archie::stage::parse_key(file.readAll())) {
                options.key = *key;
            } else {
                qWarning("archie-stage: %s is not a 64-hex-character key", qUtf8Printable(parser.value(keyFile)));
            }
        } else {
            qWarning("archie-stage: cannot read %s", qUtf8Printable(parser.value(keyFile)));
        }
    }
    options.reduced_motion = parser.isSet(reducedMotion);
    if (parser.value(tempo) == QLatin1String("agent")) {
        // What IRIS needs to work an application: the same checks, the same cinematics,
        // on a clock that fits four actions in a second.
        options.playback = archie::visual::PlaybackPolicy::agent_tempo();
    } else if (parser.value(tempo) == QLatin1String("video")) {
        options.playback = archie::visual::PlaybackPolicy::video_tempo();
    } else if (parser.value(tempo) != QLatin1String("person")) {
        qWarning("archie-stage: --tempo is person, agent or video");
    }
    if (parser.isSet(strokeSpeed)) {
        bool read = false;
        const float speed = parser.value(strokeSpeed).toFloat(&read);
        if (read && speed > 0.0F) {
            options.playback.stroke_speed_px_per_s = speed;
        } else {
            qWarning("archie-stage: --stroke-speed wants pixels a second, above nothing");
        }
    }
    if (parser.value(tier) == QLatin1String("conserve")) {
        options.tier = archie::visual::RenderTier::Conserve;
    } else if (parser.value(tier) == QLatin1String("ultra")) {
        options.tier = archie::visual::RenderTier::Ultra;
    }

    // Which screen Archie works on. A stage that is half a screen wide, or cut off at the
    // edge of one, is a stage nobody can watch: when Archie takes a screen it takes all of
    // it. Left to itself it takes the last screen, so a person with two keeps the first.
    const QList<QScreen *> screens = QGuiApplication::screens();
    int taken = screens.size() > 1 ? screens.size() - 1 : 0;
    if (parser.isSet(screen)) {
        bool read = false;
        const int asked = parser.value(screen).toInt(&read);
        if (read && asked >= 0 && asked < screens.size()) {
            taken = asked;
        } else {
            qWarning("archie-stage: --screen is 0 to %lld", static_cast<long long>(screens.size() - 1));
        }
    }
    for (int at = 0; at < screens.size(); ++at) {
        const QRect where = screens.at(at)->geometry();
        qInfo("archie-stage: screen %d is %dx%d at %d,%d%s", at, where.width(), where.height(),
              where.x(), where.y(), at == taken ? " <- taken" : "");
    }
    const QRect stand = screens.isEmpty() ? QRect(0, 0, 1280, 800) : screens.at(taken)->geometry();

    StageController controller(options);
    QQmlApplicationEngine engine;
    engine.setInitialProperties({
        {QStringLiteral("stage"), QVariant::fromValue(&controller)},
        {QStringLiteral("stageSocketName"), parser.value(socketName)},
        {QStringLiteral("fullscreen"), !parser.isSet(windowed)},
        {QStringLiteral("screenIndex"), taken},
        // Windowed, the size asked for; otherwise the whole screen, so the stage is never
        // three quarters of one with its edge off the side.
        {QStringLiteral("initialWidth"),
         parser.isSet(windowed) ? parser.value(width).toInt() : stand.width()},
        {QStringLiteral("initialHeight"),
         parser.isSet(windowed) ? parser.value(height).toInt() : stand.height()},
    });
    QObject::connect(&engine, &QQmlApplicationEngine::objectCreationFailed, &app,
                     [] { QCoreApplication::exit(1); }, Qt::QueuedConnection);
    engine.loadFromModule("Archie.Stage", "Main");

    if (parser.isSet(grab)) {
        const QString path = parser.value(grab);
        const auto save = [&controller](const QString &target) {
            if (QQuickWindow *window = controller.window()) {
                if (!window->grabWindow().save(target)) {
                    qWarning("archie-stage: could not save %s", qUtf8Printable(target));
                }
            }
        };
        if (parser.isSet(grabEvery)) {
            const int every = parser.value(grabEvery).toInt();
            auto *timer = new QTimer(&app);
            QObject::connect(timer, &QTimer::timeout, &app, [save, path, count = 0]() mutable {
                QString numbered = path;
                numbered.replace(QStringLiteral(".png"), QStringLiteral("-%1.png").arg(++count, 3, 10, QLatin1Char('0')));
                save(numbered);
            });
            QTimer::singleShot(parser.value(grabAfter).toInt(), timer, [timer, every] { timer->start(every); });
        } else {
            QTimer::singleShot(parser.value(grabAfter).toInt(), &app, [save, path] { save(path); });
        }
    }
    if (parser.isSet(quitAfter)) {
        QTimer::singleShot(parser.value(quitAfter).toInt(), &app, &QCoreApplication::quit);
    }
    return app.exec();
}
