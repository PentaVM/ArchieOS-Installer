#pragma once
#include <QObject>
#include <QVariantList>
#include <QVariantMap>
#include <QUrl>
#include <QNetworkAccessManager>
#include <QAudioSource>
#include <QVideoSink>
class Wizard : public QObject {
    Q_OBJECT
    Q_PROPERTY(QVariantList regions READ regions NOTIFY changed)
    Q_PROPERTY(QVariantList catalogCategories READ catalogCategories NOTIFY changed)
    Q_PROPERTY(QVariantList volumes READ volumes NOTIFY changed)
    Q_PROPERTY(QVariantList networks READ networks NOTIFY changed)
    Q_PROPERTY(QVariantMap location READ location NOTIFY changed)
    Q_PROPERTY(QString status READ status NOTIFY changed)
    Q_PROPERTY(QString draftPath READ draftPath NOTIFY changed)
    Q_PROPERTY(double light READ light NOTIFY qualityChanged)
    Q_PROPERTY(double voice READ voice NOTIFY qualityChanged)
    Q_PROPERTY(bool live READ live CONSTANT)
public:
    explicit Wizard(QObject *parent=nullptr);
    QVariantList regions() const { return m_regions; }
    QVariantList catalogCategories() const { return m_catalogCategories; }
    QVariantList volumes() const { return m_volumes; }
    QVariantList networks() const { return m_networks; }
    QVariantMap location() const { return m_location; }
    QString status() const { return m_status; }
    QString draftPath() const { return m_draft; }
    double light() const { return m_light; }
    double voice() const { return m_voice; }
    bool live() const;
    Q_INVOKABLE void refreshCatalog();
    Q_INVOKABLE void inspectMachine();
    Q_INVOKABLE void useNodeLocation();
    Q_INVOKABLE bool saveDraft(const QVariantMap &profile);
    Q_INVOKABLE QVariantMap loadDraft();
    Q_INVOKABLE void openPartitions();
    Q_INVOKABLE void openNetwork();
    Q_INVOKABLE void launchInstall();
    Q_INVOKABLE void openPhone();
    Q_INVOKABLE void copyActivation();
    Q_INVOKABLE void requestApproval(const QString &resource);
    Q_INVOKABLE void cancelApprovals() { ++m_approvalEpoch; }
    Q_INVOKABLE void attachVideo(QObject *sink);
    Q_INVOKABLE void testMicrophone(bool enabled);
signals:
    void changed();
    void qualityChanged();
    void locationReady();
    void permissionGranted(const QString &resource);
private:
    void message(const QString &text);
    bool readCatalog(const QByteArray &bytes);
    bool readCategoryCatalog(const QString &kind, const QByteArray &bytes, bool cache);
    void fetchGeofabrikIndex(const QUrl &url);
    void loadCachedCategoryCatalogs();
    QNetworkAccessManager m_network;
    QVariantList m_regions, m_catalogCategories, m_volumes, m_networks;
    QVariantMap m_location;
    QString m_status, m_draft;
    int m_approvalEpoch=0;
    double m_light=-1, m_voice=0;
    QAudioSource *m_audio=nullptr;
};
