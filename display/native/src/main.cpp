#include "AppCatalog.h"
#include "ArchieBridge.h"
#include "LumenField.h"
#include "DisplayControl.h"
#include "ShellRuntime.h"

#include <QGuiApplication>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQuickStyle>
#include <QQuickWindow>
#include <QTimer>
#include <QFile>
#include <QImage>
#include <QCommandLineParser>
#include <QScreen>
#include <QEvent>
#include <QtQuick3D/qquick3d.h>

#include <algorithm>
#include <cstdlib>

// Presentation is a property of the actual host surface. A window can still
// exist while minimized or unexposed, and then must not buy a render budget.
class PresentationTracker final : public QObject {
public:
    PresentationTracker(QQuickWindow *window, ArchieBridge *bridge)
        : QObject(window), m_window(window), m_bridge(bridge) {
        window->installEventFilter(this);
        connect(window, &QWindow::visibilityChanged, this, [this] { update(); });
        update();
    }
    bool eventFilter(QObject *object, QEvent *event) override {
        if (object == m_window && (event->type() == QEvent::Expose || event->type() == QEvent::Show
            || event->type() == QEvent::Hide || event->type() == QEvent::WindowStateChange)) {
            QTimer::singleShot(0, this, [this] { update(); });
        }
        return QObject::eventFilter(object, event);
    }
private:
    void update() {
        m_bridge->setPresented(m_window->isVisible() && m_window->isExposed()
                              && m_window->visibility() != QWindow::Minimized);
    }
    QQuickWindow *m_window;
    ArchieBridge *m_bridge;
};

int main(int argc, char *argv[])
{
    QGuiApplication::setApplicationName(QStringLiteral("Archie Observatory"));
    QGuiApplication::setOrganizationName(QStringLiteral("PentaVM"));
    QQuickStyle::setStyle(QStringLiteral("Basic"));

    QGuiApplication app(argc, argv);
    QSurfaceFormat::setDefaultFormat(QQuick3D::idealSurfaceFormat());
    LumenField lumen;
    AppCatalog appCatalog;
    DisplayControl displayControl(&lumen);
    QCommandLineParser arguments;
    arguments.addHelpOption();
    arguments.addOption(QCommandLineOption(QStringLiteral("shell"), QStringLiteral("Open the native Archie desktop shell.")));
    arguments.addOption(QCommandLineOption(QStringLiteral("theme"), QStringLiteral("Shell theme: dark or light (beta)."), QStringLiteral("mode"), QStringLiteral("dark")));
    arguments.addOption(QCommandLineOption(QStringLiteral("shell-bridge"), QStringLiteral("Trusted read-only shell bridge executable."),
        QStringLiteral("path"), QStringLiteral("archie-shell-bridge")));
    arguments.addOption(QCommandLineOption(QStringLiteral("lan-file"), QStringLiteral("Read-only LAN inventory projection."),
        QStringLiteral("path"), QStringLiteral("/var/lib/archie/sentry/lan.json")));
    arguments.addOption(QCommandLineOption(QStringLiteral("workspaces-root"), QStringLiteral("Read-only workspaces projection root."),
        QStringLiteral("path"), QStringLiteral("/archie/workspaces")));
    arguments.addOption(QCommandLineOption(QStringLiteral("render-check"),QStringLiteral("Write one real Qt frame to a new PNG and exit."),QStringLiteral("path")));
    arguments.addOption(QCommandLineOption(QStringLiteral("screen"), QStringLiteral("Full-screen display number (one-based)."), QStringLiteral("number"), QStringLiteral("1")));
    arguments.addOption(QCommandLineOption(QStringLiteral("scene"), QStringLiteral("Initial observatory scene: ") + displayControl.scenes().join(QStringLiteral(", ")) + QLatin1Char('.'), QStringLiteral("name"), displayControl.scenes().front()));
    arguments.addOption(QCommandLineOption(QStringLiteral("lens"), QStringLiteral("Initial orb lens: architecture, folders, or entities."), QStringLiteral("name"), QStringLiteral("architecture")));
    arguments.addOption(QCommandLineOption(QStringLiteral("preset"), QStringLiteral("Initial look preset, e.g. holograma (Xavi's desktop sphere)."), QStringLiteral("name")));
    arguments.addOption(QCommandLineOption(QStringLiteral("content-root"), QStringLiteral("Folder the content sphere lists (read-only)."), QStringLiteral("path")));
    arguments.process(app);
    const bool shellMode = arguments.isSet(QStringLiteral("shell"));
    const QString theme=arguments.value(QStringLiteral("theme"));
    if (theme != QLatin1String("dark") && theme != QLatin1String("light")) {
        qCritical("archie-display: theme must be dark or light");
        return EXIT_FAILURE;
    }
    ArchieBridge bridge(nullptr, shellMode);
    bridge.setPresented(false);
    displayControl.setShellMode(shellMode);
    ShellRuntime shellRuntime(arguments.value(QStringLiteral("shell-bridge")),
        arguments.value(QStringLiteral("lan-file")), arguments.value(QStringLiteral("workspaces-root")));
    QQmlApplicationEngine engine;
    engine.rootContext()->setContextProperty(QStringLiteral("archieBridge"), &bridge);
    engine.rootContext()->setContextProperty(QStringLiteral("lumen"), &lumen);
    engine.rootContext()->setContextProperty(QStringLiteral("appCatalog"), &appCatalog);
    engine.rootContext()->setContextProperty(QStringLiteral("displayControl"), &displayControl);
    engine.rootContext()->setContextProperty(QStringLiteral("shellRuntime"), &shellRuntime);
    engine.rootContext()->setContextProperty(QStringLiteral("shellColorMode"), theme);
    lumen.setLens(arguments.value(QStringLiteral("lens")));
    if (arguments.isSet(QStringLiteral("content-root")))
        lumen.setContentRoot(arguments.value(QStringLiteral("content-root")));
    if (arguments.isSet(QStringLiteral("preset")) && !lumen.applyProfile(arguments.value(QStringLiteral("preset"))))
        qWarning("archie-display: unknown preset %s", qPrintable(arguments.value(QStringLiteral("preset"))));
    if (shellMode) engine.load(QUrl(QStringLiteral("qrc:/archie/shell/prototype/ShellWindow.qml")));
    else engine.loadFromModule(QStringLiteral("Archie.Observatory"), QStringLiteral("Main"));
    if (engine.rootObjects().isEmpty())
        return EXIT_FAILURE;
    auto *window = qobject_cast<QQuickWindow *>(engine.rootObjects().front());
    if (window) new PresentationTracker(window, &bridge);
    if (!shellMode) {
        QString scene = displayControl.sceneNamed(arguments.value(QStringLiteral("scene")));
        if (scene.isEmpty()) {
            qWarning("archie-display: unknown initial scene %s; using %s",
                     qPrintable(arguments.value(QStringLiteral("scene"))), qPrintable(displayControl.scenes().front()));
            scene = displayControl.scenes().front();
        }
        engine.rootObjects().front()->setProperty("scene", scene);
        bridge.requestMode(displayControl.modeOf(scene));
    }
    bool screenNumberOk = false;
    const int screenNumber = arguments.value(QStringLiteral("screen")).toInt(&screenNumberOk);
    const auto screens = QGuiApplication::screens();
    if (window && screenNumberOk && screenNumber > 0 && screenNumber <= screens.size()) {
        window->setScreen(screens.at(screenNumber - 1));
        window->setVisibility(QWindow::FullScreen);
    } else if (screenNumberOk && screenNumber > 0) {
        qWarning("archie-display: requested screen %d is unavailable; this WSL session exposes %lld screen(s)",
                 screenNumber, static_cast<long long>(screens.size()));
    }
    if (!displayControl.start())
        qWarning("archie-display: local visual controls unavailable");
    if (shellMode) {
        shellRuntime.setRunning(true);
        QObject::connect(&app, &QCoreApplication::aboutToQuit, &shellRuntime, [&shellRuntime] { shellRuntime.setRunning(false); });
    }
    if (arguments.isSet(QStringLiteral("render-check"))) {
        if (!window) return EXIT_FAILURE;
        window->setVisibility(QWindow::Windowed);
        window->resize(1600,900);
        const QString path=arguments.value(QStringLiteral("render-check"));
        // A software renderer may need longer than 10 s for a field to settle.
        bool delayValid=false;
        const int delay=qEnvironmentVariableIntValue("ARCHIE_RENDER_CHECK_MS",&delayValid);
        QTimer::singleShot(delayValid ? std::clamp(delay,1000,120000) : 10000,&app,[window,path,shellMode,&shellRuntime,&bridge,&app,&lumen] {
            qInfo("archie-display render-check: orb fps=%d", lumen.fps());
            const QImage frame=window->grabWindow();
            QFile file(path);
            const bool saved=!frame.isNull() && file.open(QIODevice::WriteOnly | QIODevice::NewOnly) && frame.save(&file,"PNG");
            if (shellMode) qInfo("archie-display render-check: bridge=%s child-running=%s", qPrintable(window->property("bridgeLink").toString()), shellRuntime.running() ? "true" : "false");
            if (shellMode) qInfo("archie-display render-check: orb=%s bridge-presented=%s budget-interval=%d", qPrintable(window->property("orbDiagnostics").toString()), bridge.presented() ? "true" : "false", bridge.frame().value(QStringLiteral("frameIntervalMs")).toInt());
            app.exit(saved ? EXIT_SUCCESS : EXIT_FAILURE);
        });
    }
    return app.exec();
}
