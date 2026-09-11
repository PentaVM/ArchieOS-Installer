#include "Wizard.h"
#include <QJsonDocument>
#include <QJsonObject>
#include <QJsonArray>
#include <QFile>
#include <QSaveFile>
#include <QDir>
#include <QStandardPaths>
#include <QStorageInfo>
#include <QNetworkInterface>
#include <QNetworkReply>
#include <QProcess>
#include <QMediaDevices>
#include <QAudioDevice>
#include <QVideoFrame>
#include <QDateTime>
#include <QCoreApplication>
#include <QDesktopServices>
#include <QGuiApplication>
#include <QClipboard>
#include <QTimer>
#include <QUuid>
#include <QMessageAuthenticationCode>
#include <QRegularExpression>
#include <cmath>

static QString cacheDirectory() {
    const auto dir=QStandardPaths::writableLocation(QStandardPaths::AppLocalDataLocation);
    QDir().mkpath(dir); return dir;
}
Wizard::Wizard(QObject *parent):QObject(parent) {
    QFile cached(cacheDirectory()+"/regions.json");
    if(cached.open(QIODevice::ReadOnly) && cached.size()<=8000000) readCatalog(cached.readAll());
    inspectMachine();
}
bool Wizard::live() const {
#ifdef Q_OS_LINUX
    return QFile::exists("/run/archiso");
#else
    return false;
#endif
}
void Wizard::message(const QString &text) { m_status=text; emit changed(); }
void Wizard::inspectMachine() {
    m_volumes.clear();m_networks.clear();
    for(const auto &v:QStorageInfo::mountedVolumes()) {
        if(!v.isValid() || !v.isReady()) continue;
        m_volumes.push_back(QVariantMap{{"path",v.rootPath()},{"name",v.displayName()},
            {"free",double(v.bytesAvailable())/1e9},{"total",double(v.bytesTotal())/1e9},{"readOnly",v.isReadOnly()}});
    }
    for(const auto &n:QNetworkInterface::allInterfaces()) {
        if(n.flags().testFlag(QNetworkInterface::IsLoopBack)) continue;
        QStringList addresses;
        for(const auto &a:n.addressEntries()) if(a.ip().protocol()==QAbstractSocket::IPv4Protocol) addresses<<a.ip().toString();
        m_networks.push_back(QVariantMap{{"name",n.humanReadableName()},{"up",n.flags().testFlag(QNetworkInterface::IsUp)},
            {"address",addresses.join(", ")}});
    }
    message(live()?"Entorno live detectado. Revisa el disco antes de instalar.":"Preparación en este equipo. La instalación se ejecuta desde el USB de ArchieOS.");
}
void Wizard::readCatalog(const QByteArray &bytes) {
    const auto doc=QJsonDocument::fromJson(bytes);
    if(!doc.isObject()) return;
    QVariantList result;
    for(const auto &entry:doc.object().value("features").toArray()) {
        const auto p=entry.toObject().value("properties").toObject();
        const QString url=p.value("urls").toObject().value("pbf").toString();
        if(!url.startsWith("https://download.geofabrik.de/") || !url.endsWith(".osm.pbf")) continue;
        result.push_back(QVariantMap{{"name",p.value("name").toString()},{"id",p.value("id").toString()},{"url",url}});
    }
    if(!result.isEmpty()) {m_regions=result;emit changed();}
}
void Wizard::refreshCatalog() {
    QNetworkRequest request(QUrl("https://download.geofabrik.de/index-v1-nogeom.json"));
    request.setTransferTimeout(30000);
    auto reply=m_network.get(request);
    connect(reply,&QNetworkReply::readyRead,this,[reply]{if(reply->bytesAvailable()>8000000) reply->abort();});
    message("Actualizando regiones desde Geofabrik…");
    connect(reply,&QNetworkReply::finished,this,[this,reply]{
        const auto bytes=reply->readAll();
        if(reply->error()!=QNetworkReply::NoError || bytes.size()>8000000) message("Catálogo no disponible. Conservamos la copia local; puedes continuar sin descargar.");
        else {readCatalog(bytes);QSaveFile file(cacheDirectory()+"/regions.json");
            if(!m_regions.isEmpty() && file.open(QIODevice::WriteOnly)){file.write(bytes);file.commit();}
            message(QString("Catálogo local: %1 regiones. La descarga se verifica durante la instalación.").arg(m_regions.size()));}
        reply->deleteLater();
    });
}
void Wizard::useNodeLocation() {
    QNetworkRequest request(QUrl("http://127.0.0.1:7666/api/snapshot"));request.setTransferTimeout(3000);
    auto reply=m_network.get(request);
    connect(reply,&QNetworkReply::readyRead,this,[reply]{if(reply->bytesAvailable()>2097152) reply->abort();});
    connect(reply,&QNetworkReply::finished,this,[this,reply]{
        const auto doc=QJsonDocument::fromJson(reply->readAll()).object();
        const auto location=doc.value("status").toObject().value("location").toObject();
        if(reply->error()==QNetworkReply::NoError && doc.value("daemon_reachable").toBool() && doc.value("staleness_ms").toDouble(999999)<=10000 && location.value("latitude").isDouble() && location.value("longitude").isDouble()) {
            m_location=location.toVariantMap();emit locationReady();message("Ubicación declarada por el nodo local. Confírmala antes de usarla.");
        } else message("Este nodo no publica una ubicación. Puedes introducir coordenadas o dejarla desconocida.");
        reply->deleteLater();
    });
}
QVariantMap Wizard::loadDraft() {
    QFile file(cacheDirectory()+"/wizard-profile.json");
    if(!file.exists()) return {};
    if(!file.open(QIODevice::ReadOnly) || file.size()>65536) {message("No se pudo leer la preparación guardada.");return {};}
    const auto object=QJsonDocument::fromJson(file.readAll()).object();
    if(object.value("version").toInt()!=1 || !object.value("owner").isObject()) {message("La preparación guardada no es compatible.");return {};}
    m_draft=file.fileName();message("Preparación recuperada. Revisa la ubicación y autoriza los sensores de nuevo si quieres probarlos.");
    return object.toVariantMap();
}
bool Wizard::saveDraft(const QVariantMap &profile) {
    const auto owner=profile.value("owner").toMap();
    for(const auto &key:{"name","nickname","address_as","ai_alias"}) {
        const auto value=owner.value(key).toString().trimmed();
        if(value.isEmpty() || value.size()>80) {message("Completa los nombres del owner y de la IA (máximo 80 caracteres).");return false;}
    }
    const QRegularExpression account("^[a-z_][a-z0-9_-]{0,31}$");
    const QRegularExpression host("^[a-z0-9][a-z0-9-]{0,62}$");
    if(!account.match(profile.value("username").toString()).hasMatch() || !host.match(profile.value("hostname").toString()).hasMatch()) {
        message("Revisa el usuario y el nombre del nodo: utiliza minúsculas, números y guiones.");return false;
    }
    if(!profile.value("location").isNull()) {
        const auto location=profile.value("location").toMap();
        const auto latitude=location.value("latitude").toDouble(), longitude=location.value("longitude").toDouble();
        if(location.value("latitude").isNull() || location.value("longitude").isNull() || !std::isfinite(latitude) || !std::isfinite(longitude) || std::abs(latitude)>90 || std::abs(longitude)>180) {
            message("Introduce coordenadas válidas o continúa con ubicación desconocida.");return false;
        }
    }
    if(!profile.value("map").isNull() && profile.value("map").toMap().value("url").toString().isEmpty()) {
        message("Selecciona una región del catálogo o desactiva la descarga de mapas.");return false;
    }
    m_draft=cacheDirectory()+"/wizard-profile.json";
    QSaveFile file(m_draft);
    const auto bytes=QJsonDocument::fromVariant(profile).toJson();
    if(bytes.size()>65536 || !file.open(QIODevice::WriteOnly) || !file.setPermissions(QFile::ReadOwner|QFile::WriteOwner) || file.write(bytes)!=bytes.size() || !file.commit()) {
        message("No se pudo guardar el perfil.");return false;
    }
    message("Perfil guardado localmente. No contiene contraseñas ni grabaciones; el instalador lo validará antes de escribir.");return true;
}
void Wizard::openPartitions() {
    if(!live()) {message("El editor de particiones se abre desde el USB live; este equipo permanece intacto.");return;}
    const auto tool=QStandardPaths::findExecutable("gparted");
    if(tool.isEmpty() || !QProcess::startDetached(tool,{})) message("No se pudo abrir GParted. No se modificó ningún disco.");
    else message("Editor abierto: crear o eliminar particiones requiere aplicar su plan explícitamente. Después, vuelve a escanear.");
}
void Wizard::openNetwork() {
#ifdef Q_OS_LINUX
    const auto tool=QStandardPaths::findExecutable("foot");
    const auto wifi=QStandardPaths::findExecutable("iwctl");
    if(!tool.isEmpty() && !wifi.isEmpty() && QProcess::startDetached(tool,{"--",wifi})) return;
#endif
    message("Configura la red en los ajustes del sistema y vuelve a escanear. Las contraseñas no se guardan en el perfil.");
}
void Wizard::launchInstall() {
    if(!live() || m_draft.isEmpty()) {message("Guarda el perfil y arranca el USB live para instalar. No se escribió ningún disco.");return;}
    auto terminal=QStandardPaths::findExecutable("foot");
    auto installer=QStandardPaths::findExecutable("archie-install");
    if(terminal.isEmpty() || installer.isEmpty() || !QProcess::startDetached(terminal,{"--",installer,"--profile",m_draft}))
        message("No se pudo abrir el instalador. Conservamos el perfil.");
    else message("Revisa el plan en el instalador. Solo comienza después de confirmar el disco exacto.");
}
void Wizard::attachVideo(QObject *object) {
    auto sink=qobject_cast<QVideoSink*>(object);if(!sink) return;
    connect(sink,&QVideoSink::videoFrameChanged,this,[this](const QVideoFrame &frame){
        const auto image=frame.toImage().scaled(64,48,Qt::IgnoreAspectRatio,Qt::FastTransformation);
        if(image.isNull()) return;
        double sum=0;for(int y=0;y<image.height();++y) for(int x=0;x<image.width();++x) sum+=qGray(image.pixel(x,y));
        m_light=sum/(255.0*image.width()*image.height());emit qualityChanged();
    });
}
void Wizard::testMicrophone(bool enabled) {
    if(m_audio) {m_audio->stop();m_audio->deleteLater();m_audio=nullptr;}
    m_voice=0;emit qualityChanged();if(!enabled)return;
    const auto device=QMediaDevices::defaultAudioInput();
    if(device.isNull()) {message("No hay micrófono disponible.");return;}
    auto format=device.preferredFormat();m_audio=new QAudioSource(device,format,this);
    auto input=m_audio->start();if(!input){message("No se pudo abrir el micrófono.");return;}
    connect(input,&QIODevice::readyRead,this,[this,input,format]{
        const auto data=input->readAll();double peak=0;
        for(qsizetype i=0;i+format.bytesPerSample()<=data.size();i+=format.bytesPerSample()) {
            float sample=format.normalizedSampleValue(data.constData()+i);peak=std::max(peak,double(std::abs(sample)));
        }
        m_voice=peak;emit qualityChanged();
    });
    message("Prueba de nivel local activa. No se graba ni se identifica al hablante.");
}

void Wizard::openPhone() {
    QDesktopServices::openUrl(QUrl("http://localhost:7668"));
    message("Vinculación en el navegador seguro. El servicio owner-companion debe estar iniciado para este nodo.");
}

static QNetworkReply *nativePost(QNetworkAccessManager &manager,const QString &path,const QJsonObject &body,const QByteArray &key) {
    const auto bytes=QJsonDocument(body).toJson();const auto stamp=QByteArray::number(QDateTime::currentMSecsSinceEpoch());
    const auto nonce=QUuid::createUuid().toString(QUuid::WithoutBraces).toUtf8();
    QNetworkRequest request(QUrl("http://localhost:7668"+path));request.setTransferTimeout(5000);
    request.setHeader(QNetworkRequest::ContentTypeHeader,"application/json");request.setRawHeader("X-Archie-Time",stamp);
    request.setRawHeader("X-Archie-Nonce",nonce);
    request.setRawHeader("X-Archie-Signature",QMessageAuthenticationCode::hash(stamp+"\n"+nonce+"\n"+path.toUtf8()+"\n"+bytes,key,QCryptographicHash::Sha256).toHex());
    return manager.post(request,bytes);
}
static bool verifiedReply(QNetworkReply *reply,const QByteArray &bytes,const QByteArray &key) {
    const auto nonce=reply->request().rawHeader("X-Archie-Nonce");
    const auto status=QByteArray::number(reply->attribute(QNetworkRequest::HttpStatusCodeAttribute).toInt());
    const auto expected=QMessageAuthenticationCode::hash(nonce+"\n"+status+"\n"+bytes,key,QCryptographicHash::Sha256).toHex();
    const auto signature=reply->rawHeader("X-Archie-Response-Signature");
    if(signature.size()!=expected.size())return false;
    unsigned char difference=0;for(qsizetype i=0;i<expected.size();++i)difference|=static_cast<unsigned char>(signature[i]^expected[i]);
    return difference==0;
}
void Wizard::requestApproval(const QString &resource) {
    const auto epoch=++m_approvalEpoch;
    if(resource!="camera:preview" && resource!="microphone:test") {message("Permiso no reconocido.");return;}
    QFile activation(qEnvironmentVariable("ARCHIE_OWNER_ACTIVATION_FILE"));
    if(!activation.open(QIODevice::ReadOnly) || activation.size()>256) {message("Completa la activación del nodo y la vinculación del owner antes de pedir este permiso.");return;}
    const auto token=activation.readAll().trimmed();if(token.size()<32){message("Código de activación no válido.");return;}
    const auto action=QJsonObject{{"kind","approve-permission"},{"resource",resource},{"description",resource=="camera:preview"?"Encender cámara local del wizard":"Probar micrófono local del wizard"}};
    auto reply=nativePost(m_network,"/native/request",QJsonObject{{"action",action}},token);
    connect(reply,&QNetworkReply::finished,this,[this,reply,token,resource,epoch]{
        const auto bytes=reply->readAll();
        const auto id=QJsonDocument::fromJson(bytes).object().value("requestId").toString();
        const bool ok=reply->error()==QNetworkReply::NoError && id.size()==43 && verifiedReply(reply,bytes,token);reply->deleteLater();
        if(epoch!=m_approvalEpoch)return;
        if(!ok){message("No se pudo pedir aprobación al servicio del owner.");return;}
        QDesktopServices::openUrl(QUrl("http://localhost:7668/#request="+id));
        message("Aprueba el permiso con tu passkey en el navegador. Los sensores siguen apagados.");
        auto timer=new QTimer(this);timer->setInterval(1500);const auto expires=QDateTime::currentMSecsSinceEpoch()+120000;
        connect(timer,&QTimer::timeout,this,[this,timer,id,token,resource,expires,epoch]{
            if(epoch!=m_approvalEpoch){timer->stop();timer->deleteLater();return;}
            if(QDateTime::currentMSecsSinceEpoch()>=expires){timer->stop();timer->deleteLater();message("La aprobación venció. El sensor sigue apagado.");return;}
            timer->stop();
            auto result=nativePost(m_network,"/native/result",QJsonObject{{"requestId",id}},token);
            connect(result,&QNetworkReply::finished,this,[this,result,timer,resource,token,epoch]{
                const auto bytes=result->readAll();const bool signatureOk=verifiedReply(result,bytes,token);
                const auto body=QJsonDocument::fromJson(bytes).object();const auto error=result->error();result->deleteLater();
                if(epoch!=m_approvalEpoch){timer->deleteLater();return;}
                if(error!=QNetworkReply::NoError || !signatureOk){timer->deleteLater();message("No se pudo verificar la aprobación; no se activó el sensor.");return;}
                if(body.value("approved").toBool() && body.value("action").toObject().value("resource").toString()==resource){
                    timer->deleteLater();message("Permiso verificado y consumido una vez.");emit permissionGranted(resource);
                }else timer->start();
            });
        });timer->start();
    });
}

void Wizard::copyActivation() {
    QFile file(qEnvironmentVariable("ARCHIE_OWNER_ACTIVATION_FILE"));
    if(!file.open(QIODevice::ReadOnly)||file.size()>256){message("El nodo todavía no tiene un código privado de activación disponible.");return;}
    const auto code=QString::fromUtf8(file.readAll().trimmed());
    if(code.size()<32){message("El código privado no es válido.");return;}
    QGuiApplication::clipboard()->setText(code);message("Código copiado. Pégalo en la vinculación local del owner; se limpia del portapapeles en un minuto.");
    QTimer::singleShot(60000,this,[code]{if(QGuiApplication::clipboard()->text()==code)QGuiApplication::clipboard()->clear();});
}
