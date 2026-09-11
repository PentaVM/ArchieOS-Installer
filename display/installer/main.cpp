#include "Wizard.h"
#include <QGuiApplication>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQuickStyle>
#include <QQuickWindow>
#include <QTimer>
#include <QFile>
#include <QImage>
#include <QCommandLineParser>
int main(int argc,char **argv) {
    QGuiApplication app(argc,argv);app.setApplicationName("ArchieOS Installer");app.setOrganizationName("ArchieOS");
    QQuickStyle::setStyle("Basic");
    QCommandLineParser args;args.addHelpOption();
    args.addOption({"render-check","Save an actual Qt frame to a new PNG and exit","path"});args.addOption({"render-page","Page for render check (0..7)","number","0"});args.process(app);
    Wizard wizard;QQmlApplicationEngine engine;engine.rootContext()->setContextProperty("wizard",&wizard);
    engine.loadFromModule("Archie.Installer","Main");if(engine.rootObjects().isEmpty())return 1;
    if(args.isSet("render-check")) engine.rootObjects().front()->setProperty("page",args.value("render-page").toInt());
    if(args.isSet("render-check")) QTimer::singleShot(1800,&app,[&]{
        auto window=qobject_cast<QQuickWindow*>(engine.rootObjects().front());QFile file(args.value("render-check"));
        const bool ok=window && file.open(QIODevice::WriteOnly|QIODevice::NewOnly) && window->grabWindow().save(&file,"PNG");app.exit(ok?0:1);
    });
    return app.exec();
}
