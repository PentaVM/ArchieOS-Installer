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
    const QCommandLineOption windowed(QStringLiteral("windowed"), QStringLiteral("Do not go full screen."));
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
    parser.addOptions({socketName, control, keyFile, reducedMotion, tier, windowed, width, height, grab, grabAfter,
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
    if (parser.value(tier) == QLatin1String("conserve")) {
        options.tier = archie::visual::RenderTier::Conserve;
    } else if (parser.value(tier) == QLatin1String("ultra")) {
        options.tier = archie::visual::RenderTier::Ultra;
    }

    StageController controller(options);
    QQmlApplicationEngine engine;
    engine.setInitialProperties({
        {QStringLiteral("stage"), QVariant::fromValue(&controller)},
        {QStringLiteral("stageSocketName"), parser.value(socketName)},
        {QStringLiteral("fullscreen"), !parser.isSet(windowed)},
        {QStringLiteral("initialWidth"), parser.value(width).toInt()},
        {QStringLiteral("initialHeight"), parser.value(height).toInt()},
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
