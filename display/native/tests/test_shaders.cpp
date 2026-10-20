#include "LumenField.h"
#include "LumenGeometry.h"
#include "XaviDesignBridge.h"

#include <QGuiApplication>
#include <QDateTime>
#include <QDir>
#include <QFile>
#include <QImage>
#include <QMutex>
#include <QOpenGLContext>
#include <QOpenGLFunctions>
#include <QQmlContext>
#include <QQmlEngine>
#include <QQuickView>
#include <QSurfaceFormat>
#include <QTcpServer>
#include <QTcpSocket>
#include <QtTest>

namespace {
QMutex messagesMutex;
QStringList renderingErrors;
QtMessageHandler previousHandler = nullptr;

void recordMessage(QtMsgType type, const QMessageLogContext &context, const QString &message)
{
    const auto lower = message.toLower();
    if ((type == QtWarningMsg || type == QtCriticalMsg || type == QtFatalMsg)
        && (lower.contains(QStringLiteral("shader")) || lower.contains(QStringLiteral("failed"))
            || lower.contains(QStringLiteral("error")) || lower.contains(QStringLiteral("cannot")))) {
        QMutexLocker lock(&messagesMutex);
        renderingErrors.append(message);
    }
    if (previousHandler) previousHandler(type, context, message);
}
} // namespace

class ShaderContracts final : public QObject {
    Q_OBJECT
private slots:
    void allDeliveredMaterialsCompileAndDrawWithLlvmPipe()
    {
        QTcpServer source;
        QVERIFY(source.listen(QHostAddress::LocalHost, 0));
        connect(&source, &QTcpServer::newConnection, this, [&source] {
            while (auto *socket = source.nextPendingConnection()) {
                connect(socket, &QTcpSocket::readyRead, socket, [socket] {
                    socket->readAll();
                    const QByteArray body = R"({"radius":50,"profile":"balanced","root_id":"root","focus_id":"root","nodes":[{"id":"root","name":"Synthetic module","kind":"root","position":[-16,0,0]},{"id":"entity","name":"Synthetic entity","kind":"entity","parent":"root","position":[16,0,0]}],"relations":[{"source":"root","target":"entity","kind":"observed","strength":1}]})";
                    socket->write("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: "
                        + QByteArray::number(body.size()) + "\r\n\r\n" + body);
                    socket->disconnectFromHost();
                });
                connect(socket, &QTcpSocket::disconnected, socket, &QObject::deleteLater);
            }
        });
        LumenField field(nullptr, QUrl(QStringLiteral("http://127.0.0.1:%1/api/orb/lumen").arg(source.serverPort())));
        field.setRenderBudget({{QStringLiteral("frameIntervalMs"), 16}, {QStringLiteral("postProcessing"), true},
            {QStringLiteral("ambientMotion"), true}, {QStringLiteral("maxVisibleNodes"), 12}, {QStringLiteral("maxParticles"), 0}});
        field.setReducedMotion(true);
        field.setActive(true);
        QTRY_VERIFY_WITH_TIMEOUT(field.loaded(), 3000);

        QByteArray renderer;
        QMutex rendererMutex;
        QQuickView view;
        view.setResizeMode(QQuickView::SizeRootObjectToView);
        view.resize(800, 600);
        connect(&view, &QQuickWindow::beforeRendering, &view, [&] {
            if (auto *context = QOpenGLContext::currentContext()) {
                const auto *name = context->functions()->glGetString(GL_RENDERER);
                if (name) {
                    QMutexLocker lock(&rendererMutex);
                    renderer = reinterpret_cast<const char *>(name);
                }
            }
        }, Qt::DirectConnection);
        connect(&view, &QQuickWindow::sceneGraphError, this, [](QQuickWindow::SceneGraphError, const QString &error) {
            QMutexLocker lock(&messagesMutex);
            renderingErrors.append(error);
        });
        view.rootContext()->setContextProperty(QStringLiteral("shaderTestField"), &field);
        previousHandler = qInstallMessageHandler(recordMessage);
        view.setSource(QUrl(QStringLiteral("qrc:/archie/tests/ShaderSmoke.qml")));
        QCOMPARE(view.status(), QQuickView::Ready);
        view.show();
        QTest::qWait(100);
        view.grabWindow();
        {
            QMutexLocker lock(&rendererMutex);
            qInfo("CPU shader renderer: %s", renderer.constData());
            QVERIFY2(renderer.toLower().contains("llvmpipe") || renderer.toLower().contains("softpipe"), renderer.constData());
        }
        const QDir captureDirectory(qEnvironmentVariable("ARCHIE_SHADER_CAPTURE_DIR"));
        const bool capture = qEnvironmentVariableIsSet("ARCHIE_SHADER_CAPTURE_DIR");
        const auto capturePrefix = QDateTime::currentDateTimeUtc().toString(QStringLiteral("yyyyMMddTHHmmsszzz-"));
        if (capture) QVERIFY(captureDirectory.mkpath(QStringLiteral(".")));
        QStringList cases{QStringLiteral("field")};
        cases.append(xaviPresetNames());
        for (const auto &style : xaviChoices(QStringLiteral("shellStyle")))
            cases.append(QStringLiteral("shell:") + style);
        for (const auto &name : cases) {
            if (name.startsWith(QLatin1String("shell:"))) {
                QVERIFY(field.loadProfile(QStringLiteral("field")).isEmpty());
                field.setSetting(QStringLiteral("shellStyle"), name.mid(6));
            } else {
                const auto error = field.loadProfile(name);
                QVERIFY2(error.isEmpty(), qPrintable(error));
            }
            QTest::qWait(120);
            const QImage frame = view.grabWindow();
            QVERIFY2(!frame.isNull(), qPrintable(name));
            QCOMPARE(frame.size(), QSize(800, 600));
            if (capture) {
                QString label = name;
                label.replace(QLatin1Char(':'), QLatin1Char('-'));
                QFile output(captureDirectory.filePath(capturePrefix + label + QStringLiteral(".png")));
                QVERIFY2(output.open(QIODevice::WriteOnly | QIODevice::NewOnly), qPrintable(output.errorString()));
                QVERIFY(frame.save(&output, "PNG"));
            }
            const auto background = frame.pixel(0, 0);
            int drawnSamples = 0;
            for (int y = 0; y < frame.height(); y += 4)
                for (int x = 0; x < frame.width(); x += 4)
                    if (frame.pixel(x, y) != background) ++drawnSamples;
            QVERIFY2(drawnSamples > 16, qPrintable(name + QStringLiteral(" produced no visible fixture geometry")));
            QMutexLocker lock(&messagesMutex);
            QVERIFY2(renderingErrors.isEmpty(), qPrintable(renderingErrors.join(QLatin1Char('\n'))));
        }
        {
            QMutexLocker lock(&rendererMutex);
            QVERIFY2(renderer.toLower().contains("llvmpipe") || renderer.toLower().contains("softpipe"), renderer.constData());
            qInfo("CPU shader renderer: %s", renderer.constData());
        }
        field.setActive(false);
        view.hide();
        qInstallMessageHandler(previousHandler);
        previousHandler = nullptr;
    }
};

int main(int argc, char **argv)
{
    qputenv("QT_QPA_PLATFORM", "offscreen");
    qputenv("LIBGL_ALWAYS_SOFTWARE", "1");
    qputenv("GALLIUM_DRIVER", "llvmpipe");
    qputenv("QSG_RHI_BACKEND", "opengl");
    qputenv("QT_QUICK_BACKEND", "rhi");
    qunsetenv("QMLSCENE_DEVICE");
    QQuickWindow::setSceneGraphBackend(QStringLiteral("rhi"));
    QQuickWindow::setGraphicsApi(QSGRendererInterface::OpenGL);
    QSurfaceFormat format;
    format.setVersion(3, 3);
    format.setProfile(QSurfaceFormat::CoreProfile);
    format.setDepthBufferSize(24);
    format.setStencilBufferSize(8);
    QSurfaceFormat::setDefaultFormat(format);
    QGuiApplication app(argc, argv);
    qmlRegisterType<LumenGeometry>("Archie.Observatory", 1, 0, "LumenGeometry");
    qmlRegisterType<LumenInstancing>("Archie.Observatory", 1, 0, "LumenInstancing");
    qmlRegisterType<XaviGeometry>("Archie.Observatory", 1, 0, "XaviGeometry");
    qmlRegisterType<XaviSurfaceData>("Archie.Observatory", 1, 0, "XaviSurfaceData");
    qmlRegisterType<XaviHybridData>("Archie.Observatory", 1, 0, "XaviHybridData");
    ShaderContracts test;
    return QTest::qExec(&test, argc, argv);
}

#include "test_shaders.moc"
