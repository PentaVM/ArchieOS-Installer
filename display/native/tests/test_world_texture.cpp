#include "MeshGeometry.h"
#include "WorldImageProvider.h"
#include <QGuiApplication>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQuickWindow>
#include <QTest>
#include <QVector3D>
#include <cstdlib>

void check(bool ok,const char *why){if(!ok){qCritical("%s",why);std::exit(1);}}
int main(int argc,char **argv) {
    QGuiApplication app(argc,argv);
    qmlRegisterType<MeshGeometry>("WorldTest",1,0,"MeshGeometry");
    QQmlApplicationEngine engine;
    auto *provider=new WorldImageProvider;
    QImage image(64,64,QImage::Format_ARGB32);
    for(int y=0;y<64;++y) for(int x=0;x<64;++x) image.setPixelColor(x,y,x<16?QColor(0,0,0,0):x<32?QColor(128,128,128):y<32?Qt::red:Qt::blue);
    provider->replace({{QStringLiteral("orientation"),image}});
    engine.addImageProvider(QStringLiteral("world"),provider);
    // Geography uses image v=0 at north. Verify that convention through the
    // production material, actual custom geometry, image provider and Qt renderer.
    const float vertices[]={
        -50,50,0, 0,0,1, 0,0, -50,-50,0, 0,0,1, 0,1, 50,50,0, 0,0,1, 1,0,
        50,50,0, 0,0,1, 1,0, -50,-50,0, 0,0,1, 0,1, 50,-50,0, 0,0,1, 1,1};
    const quint32 indices[]={0,1,2,3,4,5};
    engine.rootContext()->setContextProperty(QStringLiteral("tileVertices"),QByteArray(reinterpret_cast<const char*>(vertices),sizeof(vertices)));
    engine.rootContext()->setContextProperty(QStringLiteral("tileIndices"),QByteArray(reinterpret_cast<const char*>(indices),sizeof(indices)));
    engine.loadData(R"(
        import QtQuick
        import QtQuick3D
        import WorldTest
        import "qrc:/world-texture"
        Window {
            width:256;height:256;visible:true
            View3D {
                anchors.fill:parent
                camera:tileCamera
                environment:SceneEnvironment {backgroundMode:SceneEnvironment.Color;clearColor:"black";tonemapMode:SceneEnvironment.TonemapModeLinear}
                OrthographicCamera {id:tileCamera;z:200;horizontalMagnification:1;verticalMagnification:1}
                Model {
                    geometry:MeshGeometry {vertices:tileVertices;triangleIndices:tileIndices;hasUv:true}
                    materials:WorldImageryMaterial {textureSource:"image://world/orientation";terrainColor:"#00ff00"}
                }
            }
        }
    )");
    check(!engine.rootObjects().isEmpty(),"production imagery material loads");
    auto *window=qobject_cast<QQuickWindow*>(engine.rootObjects().first());
    check(window&&QTest::qWaitForWindowExposed(window),"texture render surface is exposed");
    QTest::qWait(500);
    const auto frame=window->grabWindow();
    check(!frame.isNull(),"actual Qt texture frame is available");
    const auto north=frame.pixelColor(frame.width()/2,frame.height()/2-25);
    const auto south=frame.pixelColor(frame.width()/2,frame.height()/2+25);
    qInfo()<<"north/south texture pixels"<<north<<south;
    check(north.red()>200&&north.blue()<30&&south.blue()>200&&south.red()<30,"image north row maps to geographical north and south row maps to south");
    const auto missing=frame.pixelColor(frame.width()/2-37,frame.height()/2);
    check(missing.green()>200&&missing.red()<30&&missing.blue()<30,"transparent image gaps show neutral terrain material on the same surface");
    const auto gray=frame.pixelColor(frame.width()/2-12,frame.height()/2);
    qInfo()<<"neutral terrain/gray pixels"<<missing<<gray;
    check(std::abs(gray.red()-128)<10&&std::abs(gray.green()-128)<10&&std::abs(gray.blue()-128)<10,"imagery midtones retain source color without cinematic tone mapping");
    qInfo("World image-provider texture render contract passed");
}
