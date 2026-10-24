#include "XaviDesignBridge.h"
#include "archie_lumen_design.h"
#include <QCache>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QImageReader>
#include <algorithm>
#include <cmath>
#include <memory>

namespace {
const QJsonObject &catalog() {
    static const auto value = QJsonDocument::fromJson(QByteArray(design_presets_json())).object();
    return value;
}
const QStringList styles{QStringLiteral("astra_original"), QStringLiteral("prism_cut"),
    QStringLiteral("obsidian"), QStringLiteral("brushed_titanium"), QStringLiteral("hologram"),
    QStringLiteral("fractured_crystal"), QStringLiteral("microcircuit"),
    QStringLiteral("photon_skin"), QStringLiteral("matte_ceramic")};
const QStringList links{QStringLiteral("astra_original"), QStringLiteral("hairline"),
    QStringLiteral("hairline_dual"), QStringLiteral("fiber_twist"), QStringLiteral("technical"),
    QStringLiteral("dash_phase"), QStringLiteral("refracted_thread"),
    QStringLiteral("circuit_dash"), QStringLiteral("plasma_ribbon")};
const QStringList pulses{QStringLiteral("none"), QStringLiteral("soft_packet"),
    QStringLiteral("traveling_glint"), QStringLiteral("subtle_tick"), QStringLiteral("scan_packet"),
    QStringLiteral("facet_spark"), QStringLiteral("data_packet"), QStringLiteral("plasma_bead")};
}

QVariantMap xaviDefaults() { return catalog().value(QStringLiteral("defaults")).toObject().toVariantMap(); }
std::optional<QVariantMap> xaviProfile(const QString &name) {
    for (const auto &p : catalog().value(QStringLiteral("profiles")).toArray()) {
        const auto o = p.toObject();
        if (o.value(QStringLiteral("id")).toString() == name)
            return o.value(QStringLiteral("overrides")).toObject().toVariantMap();
    }
    return std::nullopt;
}
QVariantMap xaviLegacy() { return catalog().value(QStringLiteral("legacy")).toObject().toVariantMap(); }
QStringList xaviPresetNames() {
    QStringList out;
    for (const auto &p : catalog().value(QStringLiteral("presets")).toArray())
        out.append(p.toObject().value(QStringLiteral("id")).toString());
    return out;
}
std::optional<QVariantMap> xaviPreset(const QString &name) {
    for (const auto &p : catalog().value(QStringLiteral("presets")).toArray()) {
        const auto o = p.toObject();
        if (o.value(QStringLiteral("id")).toString() == name)
            return o.value(QStringLiteral("settings")).toObject().toVariantMap();
    }
    return std::nullopt;
}
QStringList xaviChoices(const QString &key) {
    if (key == QLatin1String("visualNodeStyle") || key == QLatin1String("visualFolderStyle")) return styles;
    if (key == QLatin1String("visualLinkStyle")) return links;
    if (key == QLatin1String("visualPulseStyle")) return pulses + QStringList{QStringLiteral("astra_original")};
    if (key == QLatin1String("visualBloomMode")) return {QStringLiteral("astra_original"), QStringLiteral("global"), QStringLiteral("selection"), QStringLiteral("polar"), QStringLiteral("off")};
    if (key == QLatin1String("visualBloomMask")) return {QStringLiteral("bright"), QStringLiteral("poles_and_focus"), QStringLiteral("selected_and_neighbors"), QStringLiteral("none")};
    if (key == QLatin1String("visualTextureTier")) return {QStringLiteral("adaptive"), QStringLiteral("procedural")};
    if (key == QLatin1String("shellStyle")) return {QStringLiteral("fragments"), QStringLiteral("veil"), QStringLiteral("filaments"), QStringLiteral("aurora")};
    return {};
}
int xaviStyle(const QString &name) { return std::max(0, int(styles.indexOf(name))); }
int xaviLink(const QString &name) { return std::max(0, int(links.indexOf(name))); }
int xaviPulse(const QString &name) { return std::max(0, int(pulses.indexOf(name))); }

unsigned xaviStep(std::vector<archie::visual::lumen::Body> &bodies,
                  const std::optional<archie::visual::Vec3> &pointer, float dt,
                  const archie::visual::lumen::Motion &motion) {
    // Reuse storage rather than allocating at frame rate. No reinterpret_cast across
    // Rust/C++ layouts: the legacy Body has a bool and a different member order.
    thread_local std::vector<DesignBody> work;
    work.resize(bodies.size());
    for (std::size_t i = 0; i < bodies.size(); ++i) {
        const auto &b = bodies[i];
        work[i] = {{b.position.x,b.position.y,b.position.z}, {b.velocity.x,b.velocity.y,b.velocity.z},
                   {b.anchor.x,b.anchor.y,b.anchor.z}, b.fixed ? 1U : 0U};
    }
    DesignVec3 p{};
    if (pointer) p = {pointer->x,pointer->y,pointer->z};
    const DesignMotion m{motion.radius,motion.spring,motion.damping,motion.reach,motion.reaction};
    const auto flags = design_step(work.data(), work.size(), pointer ? &p : nullptr, dt, &m);
    for (std::size_t i = 0; i < bodies.size(); ++i) {
        const auto &b = work[i];
        bodies[i].position = {b.position.x,b.position.y,b.position.z};
        bodies[i].velocity = {b.velocity.x,b.velocity.y,b.velocity.z};
    }
    return flags;
}

XaviGeometry::XaviGeometry(QQuick3DObject *parent) : LumenGeometry(parent) {}
bool xaviShell(int kind,const std::vector<XaviShellNode> &nodes,float radius,float density,float scale,
    archie::visual::Vec3 warm,archie::visual::Vec3 cool,archie::visual::Vec3 tint,
    std::vector<float> &vertices,std::vector<std::uint32_t> &indices,float &shellRadius) {
    std::vector<DesignShellNode> input;
    for(const auto &n:nodes)input.push_back({{n.position.x,n.position.y,n.position.z},{n.color.x,n.color.y,n.color.z},n.branch?1U:0U});
    const DesignShellInput p{std::uint32_t(kind),input.data(),input.size(),radius,density,scale,
        {warm.x,warm.y,warm.z},{cool.x,cool.y,cool.z},{tint.x,tint.y,tint.z}};
    std::size_t nv=0,ni=0;
    if(!design_shell(&p,nullptr,0,nullptr,0,&nv,&ni,&shellRadius)||nv>100000||ni>200000)return false;
    vertices.resize(nv*10);indices.resize(ni);
    // Empty geometry is a legitimate empty observed field.
    if(nv==0)return true;
    return design_shell(&p,vertices.data(),vertices.size(),indices.data(),indices.size(),&nv,&ni,&shellRadius);
}
void XaviGeometry::setStyle(int v) { v = std::clamp(v,0,8); if (v != m_style) { m_style=v; rebuild(); emit changed(); } }
void XaviGeometry::setFolder(bool v) { if (v != m_folder) { m_folder=v; rebuild(); emit changed(); } }
void XaviGeometry::rebuild() {
    // Filled through the bounded Rust mesh ABI; style zero belongs to the existing
    // category-specific original sphere and never replaces it.
    std::size_t nv=0, ni=0;
    if (!design_mesh(m_style,m_folder,nullptr,0,nullptr,0,&nv,&ni)) { clear(); update(); return; }
    if (nv > 100000 || ni > 100000) return;
    std::vector<float> v(nv * 8);
    std::vector<std::uint32_t> i(ni);
    if (design_mesh(m_style,m_folder,v.data(),v.size(),i.data(),i.size(),&nv,&ni)) setTextured(v,i);
}

XaviSurfaceData::XaviSurfaceData(QQuick3DObject *parent) : QQuick3DTextureData(parent) { rebuild(); }
void XaviSurfaceData::setStyle(int v) { v=std::clamp(v,0,8); if(v!=m_style){m_style=v;rebuild();emit changed();} }
void XaviSurfaceData::setResolution(int v) { v=std::clamp(v,32,1024); if(v!=m_resolution){m_resolution=v;rebuild();emit changed();} }
void XaviSurfaceData::setFrequency(float v) { if(!std::isfinite(v))return;v=std::clamp(v,0.1F,8.F);if(v!=m_frequency){m_frequency=v;rebuild();emit changed();} }
void XaviSurfaceData::setSurface(bool v) { if(v!=m_surface){m_surface=v;rebuild();emit changed();} }
void XaviSurfaceData::rebuild() {
    if(m_style==0) {
        setSize(QSize(1,1));setFormat(QQuick3DTextureData::RGBA8);
        setHasTransparency(false);setTextureData(QByteArray(4,char(255)));update();return;
    }
    struct Pack { QByteArray albedo,surface; };
    // A GUI-thread cache shared by both channels and same-style node/folder meshes.
    // At most 32 MiB retained; no disk writes or source texture download at runtime.
    static QCache<QString,Pack> cache(32768);
    const auto key=QStringLiteral("%1:%2:%3").arg(m_style).arg(m_resolution).arg(std::round(m_frequency*2));
    auto *pack=cache.object(key);
    if(!pack) {
        auto fresh=std::make_unique<Pack>();
        const auto bytes=m_resolution*m_resolution*4;
        fresh->albedo.resize(bytes); fresh->surface.resize(bytes);
        if(!design_surface(m_style,m_resolution,m_frequency,
            reinterpret_cast<std::uint8_t *>(fresh->albedo.data()),
            reinterpret_cast<std::uint8_t *>(fresh->surface.data()),bytes)) return;
        pack=fresh.release();cache.insert(key,pack,bytes*2/1024);
    }
    setSize(QSize(m_resolution,m_resolution)); setFormat(QQuick3DTextureData::RGBA8);
    setHasTransparency(false);setTextureData(m_surface ? pack->surface : pack->albedo);update();
}

QVector4D XaviSurfaceData::hybridParameters(int style,float depth,float emissive,float contrast) const {
    float values[4]{};
    design_hybrid(style,depth,emissive,contrast,values,4);
    return {values[0],values[1],values[2],values[3]};
}
XaviHybridData::XaviHybridData(QQuick3DObject *parent):QQuick3DTextureData(parent){rebuild();}
void XaviHybridData::setResolution(int v){v=std::clamp(v,0,4096);if(v!=m_resolution){m_resolution=v;rebuild();emit changed();}}
void XaviHybridData::setSurface(bool v){if(v!=m_surface){m_surface=v;rebuild();emit changed();}}
void XaviHybridData::setEnabled(bool v){if(v!=m_enabled){m_enabled=v;rebuild();emit changed();}}
int XaviHybridData::chooseResolution(int maximum,int quality,float projected) const {
    return int(design_texture_size(std::max(0,maximum),4096,std::clamp(quality,0,2),projected,m_resolution));
}
void XaviHybridData::rebuild(){
    setFormat(QQuick3DTextureData::RGBA8);setHasTransparency(false);
    if(!m_enabled || !m_resolution){
        setSize(QSize(1,1));setTextureData(QByteArray(4,char(255)));update();return;
    }
    // Decode down to the Rust policy's selected LOD. RGBA is packed LINEAR data,
    // so neither sRGB conversion nor premultiplied alpha may touch these channels.
    static QCache<QString,QImage> cache(196608);
    const auto key=QStringLiteral("%1:%2").arg(m_surface).arg(m_resolution);
    auto *image=cache.object(key);
    if(!image){
        QImageReader reader(m_surface?QStringLiteral(":/archie/assets/xavi/prism-surface-4k.png"):
            QStringLiteral(":/archie/assets/xavi/prism-structure-4k.png"));
        reader.setScaledSize(QSize(m_resolution,m_resolution));
        auto fresh=reader.read().convertToFormat(QImage::Format_RGBA8888).mirrored(false,true);
        if(fresh.isNull())return;
        image=new QImage(std::move(fresh));cache.insert(key,image,int(image->sizeInBytes()/1024));
    }
    setSize(image->size());setTextureData(QByteArray(reinterpret_cast<const char *>(image->constBits()),image->sizeInBytes()));update();
}
