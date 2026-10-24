#pragma once

// Qt owns presentation and buffers. The delivered Xavi application decisions live in
// archie-lumen-design (Rust); this adapter has no filesystem scanner or authority.
#include "LumenGeometry.h"
#include "archie/visual/lumen.hpp"
#include <QQuick3DTextureData>
#include <QVariantMap>
#include <QVector4D>
#include <optional>

QVariantMap xaviDefaults();
// The design's composition profiles (field, crown, atlas, dense) as overrides of the
// defaults, exactly as main.js PROFILES has them; nullopt for any other name.
std::optional<QVariantMap> xaviProfile(const QString &name);
// The "Antes" look (main.js LEGACY_SETTINGS) as overrides of the defaults.
QVariantMap xaviLegacy();
QStringList xaviPresetNames();
std::optional<QVariantMap> xaviPreset(const QString &name);
QStringList xaviChoices(const QString &key);
int xaviStyle(const QString &name);
int xaviLink(const QString &name);
int xaviPulse(const QString &name);
// Bit 0: positions changed; bit 1: simulation remains active.
unsigned xaviStep(std::vector<archie::visual::lumen::Body> &bodies,
                  const std::optional<archie::visual::Vec3> &pointer, float dt,
                  const archie::visual::lumen::Motion &motion);
struct XaviShellNode { archie::visual::Vec3 position,color; bool branch; };
bool xaviShell(int kind,const std::vector<XaviShellNode> &nodes,float radius,float density,float scale,
    archie::visual::Vec3 warm,archie::visual::Vec3 cool,archie::visual::Vec3 tint,
    std::vector<float> &vertices,std::vector<std::uint32_t> &indices,float &shellRadius);

class XaviGeometry : public LumenGeometry {
    Q_OBJECT
    QML_ELEMENT
    Q_PROPERTY(int style READ style WRITE setStyle NOTIFY changed)
    Q_PROPERTY(bool folder READ folder WRITE setFolder NOTIFY changed)
public:
    explicit XaviGeometry(QQuick3DObject *parent = nullptr);
    int style() const { return m_style; }
    bool folder() const { return m_folder; }
    void setStyle(int style);
    void setFolder(bool folder);
signals:
    void changed();
private:
    void rebuild();
    int m_style = 0;
    bool m_folder = false;
};

// Linear RGBA albedo or packed normal/roughness, generated only when settings change.
class XaviSurfaceData : public QQuick3DTextureData {
    Q_OBJECT
    QML_ELEMENT
    Q_PROPERTY(int style READ style WRITE setStyle NOTIFY changed)
    Q_PROPERTY(int resolution READ resolution WRITE setResolution NOTIFY changed)
    Q_PROPERTY(float frequency READ frequency WRITE setFrequency NOTIFY changed)
    Q_PROPERTY(bool surface READ surface WRITE setSurface NOTIFY changed)
public:
    explicit XaviSurfaceData(QQuick3DObject *parent = nullptr);
    int style() const { return m_style; }
    int resolution() const { return m_resolution; }
    float frequency() const { return m_frequency; }
    bool surface() const { return m_surface; }
    void setStyle(int value);
    void setResolution(int value);
    void setFrequency(float value);
    void setSurface(bool value);
    Q_INVOKABLE QVector4D hybridParameters(int style, float depth, float emissive, float contrast) const;
signals:
    void changed();
private:
    void rebuild();
    int m_style = 0;
    int m_resolution = 512;
    float m_frequency = 1;
    bool m_surface = false;
};

class XaviHybridData : public QQuick3DTextureData {
    Q_OBJECT
    QML_ELEMENT
    Q_PROPERTY(int resolution READ resolution WRITE setResolution NOTIFY changed)
    Q_PROPERTY(bool surface READ surface WRITE setSurface NOTIFY changed)
    Q_PROPERTY(bool enabled READ enabled WRITE setEnabled NOTIFY changed)
public:
    explicit XaviHybridData(QQuick3DObject *parent = nullptr);
    int resolution() const { return m_resolution; }
    bool surface() const { return m_surface; }
    bool enabled() const { return m_enabled; }
    void setResolution(int value);
    void setSurface(bool value);
    void setEnabled(bool value);
    Q_INVOKABLE int chooseResolution(int maximum, int quality, float projected) const;
signals:
    void changed();
private:
    void rebuild();
    int m_resolution=512;
    bool m_surface=false;
    bool m_enabled=false;
};
