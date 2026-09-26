#include "AppCatalog.h"
#include "ArchieBridge.h"
#include "LumenField.h"

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

#include <cstdlib>

int main(int argc, char *argv[])
{
    QGuiApplication::setApplicationName(QStringLiteral("Archie Observatory"));
    QGuiApplication::setOrganizationName(QStringLiteral("PentaVM"));
    QQuickStyle::setStyle(QStringLiteral("Basic"));

    QGuiApplication app(argc, argv);
    QCommandLineParser arguments;
    arguments.addHelpOption();
    arguments.addOption(QCommandLineOption(QStringLiteral("render-check"),QStringLiteral("Write one real Qt frame to a new PNG and exit."),QStringLiteral("path")));
    arguments.addOption(QCommandLineOption(QStringLiteral("screen"), QStringLiteral("Full-screen display number (one-based)."), QStringLiteral("number"), QStringLiteral("1")));
    arguments.addOption(QCommandLineOption(QStringLiteral("scene"), QStringLiteral("Initial observatory scene: world, forge, or orb."), QStringLiteral("name"), QStringLiteral("world")));
    arguments.addOption(QCommandLineOption(QStringLiteral("lens"), QStringLiteral("Initial orb lens: architecture, folders, or entities."), QStringLiteral("name"), QStringLiteral("architecture")));
    arguments.process(app);
    ArchieBridge bridge;
    LumenField lumen;
    AppCatalog appCatalog;
    QQmlApplicationEngine engine;
    engine.rootContext()->setContextProperty(QStringLiteral("archieBridge"), &bridge);
    engine.rootContext()->setContextProperty(QStringLiteral("lumen"), &lumen);
    engine.rootContext()->setContextProperty(QStringLiteral("appCatalog"), &appCatalog);
    lumen.setLens(arguments.value(QStringLiteral("lens")));
    engine.loadFromModule(QStringLiteral("Archie.Observatory"), QStringLiteral("Main"));
    if (engine.rootObjects().isEmpty())
        return EXIT_FAILURE;
    auto *window = qobject_cast<QQuickWindow *>(engine.rootObjects().front());
    const QString scene = arguments.value(QStringLiteral("scene")).toLower();
    if (scene == QStringLiteral("orb")) {
        engine.rootObjects().front()->setProperty("scene", QStringLiteral("orb"));
        bridge.requestMode(QStringLiteral("cerebra"));
    } else if (scene == QStringLiteral("forge") || scene == QStringLiteral("core")) {
        // CORE became FORGE; the old name still opens it.
        engine.rootObjects().front()->setProperty("scene", QStringLiteral("forge"));
        bridge.requestMode(QStringLiteral("cerebra"));
    } else if (scene != QStringLiteral("world")) {
        qWarning("archie-display: unknown initial scene %s; using WORLD", qPrintable(scene));
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
    if (arguments.isSet(QStringLiteral("render-check"))) {
        if (!window) return EXIT_FAILURE;
        window->setVisibility(QWindow::Windowed);
        window->resize(1600,900);
        const QString path=arguments.value(QStringLiteral("render-check"));
        QTimer::singleShot(10000,&app,[window,path,&app] {
            const QImage frame=window->grabWindow();
            QFile file(path);
            const bool saved=!frame.isNull() && file.open(QIODevice::WriteOnly | QIODevice::NewOnly) && frame.save(&file,"PNG");
            app.exit(saved ? EXIT_SUCCESS : EXIT_FAILURE);
        });
    }
    return app.exec();
}
