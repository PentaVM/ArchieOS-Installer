#include <QGuiApplication>
#include <QQuickItem>
#include <QQmlEngine>
#include <QQmlComponent>
#include <QVariantMap>
#include <QDebug>
#include <cstdlib>
class ProjectionReader : public QObject {
    Q_OBJECT
    Q_PROPERTY(QVariantMap exploration MEMBER exploration NOTIFY explorationChanged)
    Q_PROPERTY(QVariantMap snapshot MEMBER snapshot NOTIFY snapshotChanged)
    Q_PROPERTY(QVariantMap kernel MEMBER kernel NOTIFY kernelChanged)
public:
    QVariantMap exploration,snapshot,kernel;int plans=0;bool animated=true;double lastElevation=0;
    Q_INVOKABLE bool frameExploration(double,double,double,double elevation,bool motion){++plans;animated=motion;lastElevation=elevation;return true;}
signals:
    void explorationChanged();void snapshotChanged();void kernelChanged();
};
void check(bool condition,const char *message){if(!condition){qCritical("%s",message);std::exit(1);}}
QQuickItem *findNotice(QQuickItem *item,const QString &name) {
    if(!item) return nullptr;
    if(item->objectName()==name) return item;
    for(auto *child:item->childItems()) if(auto *found=findNotice(child,name)) return found;
    return nullptr;
}
QVariantMap job(QString id,QVariant elevation=QVariant()) {return {{QStringLiteral("id"),id},{QStringLiteral("domain"),QStringLiteral("personal")},{QStringLiteral("state"),QStringLiteral("running")},{QStringLiteral("camera"),QVariantMap{{QStringLiteral("latitude"),0},{QStringLiteral("longitude"),0},{QStringLiteral("radius_m"),1000},{QStringLiteral("elevation_m"),elevation}}}};}
int main(int argc,char **argv) {
    QGuiApplication app(argc,argv);QQmlEngine engine;ProjectionReader reader;
    reader.exploration={{QStringLiteral("source"),QStringLiteral("live")},{QStringLiteral("jobs"),QVariantList{job(QStringLiteral("first"))}}};
    QQmlComponent component(&engine,QUrl(QStringLiteral("qrc:/world-test/WorldHud.qml")));
    auto *hud=component.createWithInitialProperties({{QStringLiteral("controller"),QVariant::fromValue(&reader)},{QStringLiteral("worldActive"),true}});
    if(!hud)qCritical()<<component.errors();
    check(hud,"production WorldHud loads with actual shared shell resources");
    check(reader.plans==1,"initial new job frames once");
    emit reader.explorationChanged();check(reader.plans==1,"poll does not reframe same job");
    reader.exploration.insert(QStringLiteral("jobs"),QVariantList{job(QStringLiteral("first"),4200)});
    emit reader.explorationChanged();check(reader.plans==2 && reader.lastElevation==4200,"first measured terrain elevation corrects automatic same-job framing");
    reader.exploration.insert(QStringLiteral("jobs"),QVariantList{job(QStringLiteral("first"),4300)});
    emit reader.explorationChanged();check(reader.plans==2,"later elevation progress does not restart consumed correction");
    QMetaObject::invokeMethod(hud,"cedeCamera");
    emit reader.explorationChanged();check(reader.plans==2,"manual camera survives poll");
    hud->setProperty("worldActive",false);hud->setProperty("worldActive",true);
    check(reader.plans==2,"focus changes cannot reclaim consumed camera plan");
    reader.exploration.insert(QStringLiteral("jobs"),QVariantList{job(QStringLiteral("first")),job(QStringLiteral("next"))});
    emit reader.explorationChanged();check(reader.plans==3,"new job can frame next AOI");
    QMetaObject::invokeMethod(hud,"cedeCamera");
    reader.exploration.insert(QStringLiteral("jobs"),QVariantList{job(QStringLiteral("first")),job(QStringLiteral("next"),5000)});
    emit reader.explorationChanged();check(reader.plans==3,"manual takeover blocks first terrain-ready reframe");
    reader.exploration.insert(QStringLiteral("jobs"),QVariantList{job(QStringLiteral("below-sea"),-430)});
    emit reader.explorationChanged();check(reader.plans==4 && reader.lastElevation==-430,"known negative elevation is preserved in camera target");
    reader.exploration.insert(QStringLiteral("source"),QStringLiteral("unavailable"));
    emit reader.explorationChanged();check(reader.plans==4,"unavailable snapshot cannot frame");
    const auto darkFill=hud->property("fill");
    QQmlComponent theme(&engine);
    theme.setData("import QtQuick\nimport \"qrc:/archie/shell/prototype\" as Shell\nQtObject { Component.onCompleted: { Shell.Theme.colorMode=\"light\";Shell.Theme.reducedMotion=true } }",QUrl());
    auto *themeSwitch=theme.create();check(themeSwitch,"shared Theme can be switched at runtime");
    check(hud->property("fill")!=darkFill,"World HUD follows actual shared light theme");
    reader.exploration.insert(QStringLiteral("source"),QStringLiteral("live"));
    reader.exploration.insert(QStringLiteral("jobs"),QVariantList{job(QStringLiteral("reduced-motion"))});
    emit reader.explorationChanged();check(reader.plans==5,"new plan remains available in reduced motion");
    check(!reader.animated,"reduced motion disables spatial camera interpolation");
    auto bounded=job(QStringLiteral("bounded-view"));
    bounded.insert(QStringLiteral("projection_truncated"),true);
    bounded.insert(QStringLiteral("preview"),QVariantMap{{QStringLiteral("omitted_buildings"),7}});
    reader.exploration.insert(QStringLiteral("jobs"),QVariantList{bounded});
    emit reader.explorationChanged();QCoreApplication::processEvents();
    auto *truncated=findNotice(qobject_cast<QQuickItem*>(hud),QStringLiteral("projectionTruncationNotice"));
    auto *omitted=findNotice(qobject_cast<QQuickItem*>(hud),QStringLiteral("omittedBuildingsNotice"));
    check(truncated && truncated->property("visible").toBool(),"bounded projection explicitly identifies truncated plan/artifacts");
    check(omitted && omitted->property("visible").toBool() && omitted->property("text").toString().contains(QStringLiteral("7")),"omitted building count is visible");
    delete themeSwitch;delete hud;qInfo("World HUD QML contracts passed");
}
#include "test_world_hud.moc"
