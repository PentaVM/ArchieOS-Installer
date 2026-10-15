#include "ArchieBridge.h"

#include "Payload.h"

#include "archie/osm/building.hpp"
#include "archie/visual/mesh.hpp"

#include <QColor>
#include <QCryptographicHash>
#include <QDateTime>
#include <QFile>
#include <QFutureWatcher>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QNetworkReply>
#include <QQuaternion>
#include <QUrl>
#include <QVector3D>
#include <QtConcurrentRun>
#include <QtMath>

#include <algorithm>
#include <cmath>
#include <cstdlib>
#include <cstring>
#include <tuple>
#include <vector>

namespace {
constexpr auto snapshotUrl = "http://127.0.0.1:7666/api/snapshot";
constexpr auto worldUrl = "http://127.0.0.1:7666/api/world";
constexpr auto kernelUrl = "http://127.0.0.1:7666/api/kernel";
constexpr auto orbUrl = "http://127.0.0.1:7666/api/orb";
constexpr auto geoFabricUrl = "http://127.0.0.1:7666/api/geofabric";
constexpr int maximumProjectionBytes = 2 * 1024 * 1024;
constexpr int maximumOrbProjectionBytes = 8 * 1024 * 1024;

/// The model's own world, unaltered.
///
/// An earlier version of this shrank the layout radii to fit the scene the QML camera was
/// framed for, and left `Observatory::home_for` — which places the camera 44 units back
/// to hold every shell at once — untouched. The two are one coherent set of numbers, and
/// overriding half of them put the camera in the right place for a scene ten times the
/// size of the one being drawn.
///
/// So the scale is not adjusted here. The QML adopts the model's coordinate system
/// instead, which is the direction that cannot drift: there is one world, and the
/// renderer is in it.
archie::visual::ObservatoryPolicy scenePolicy()
{
    return archie::visual::ObservatoryPolicy{};
}

/// How old a payload already was when it reached us, from the daemon that made it.
///
/// Absent or negative reads as zero rather than as an error: a projection with no
/// staleness field is one this build has not been taught about, and treating that as
/// arbitrarily old would grey out a display for a reason nobody could find. The model
/// ages it from arrival either way, so a daemon that goes silent is still noticed.
std::int64_t ageOf(const QJsonObject &payload, const QString &key)
{
    const auto value = payload.value(key);
    if (!value.isDouble())
        return 0;
    return std::max<std::int64_t>(0, static_cast<std::int64_t>(value.toDouble()));
}

/// What the model made of a payload.
///
/// Surfaced because the alternative is a scene that quietly has fewer things in it than
/// the daemon sent. A display that drops a record and says nothing is worse than one
/// that shows nothing at all: the second is obviously broken, and the first is trusted.
QVariantMap ingestSummary(const archie::visual::IngestReport &report)
{
    QVariantList defects;
    defects.reserve(static_cast<qsizetype>(report.defects.size()));
    for (const auto &defect : report.defects) {
        defects.push_back(QVariantMap{
            {QStringLiteral("path"), QString::fromStdString(defect.path)},
            {QStringLiteral("detail"), QString::fromStdString(defect.detail)},
        });
    }
    return QVariantMap{
        {QStringLiteral("clean"), report.clean()},
        {QStringLiteral("accepted"), report.accepted},
        {QStringLiteral("dropped"), report.dropped},
        // Not a problem — the ordinary signature of a daemon newer than this display —
        // but the answer to "why is the field I added not showing up".
        {QStringLiteral("unknownFields"), report.unknown_fields},
        {QStringLiteral("newerSchema"), report.newer_schema},
        {QStringLiteral("defects"), defects},
    };
}

QVariantList toStringList(const std::vector<std::string> &values)
{
    QVariantList out;
    out.reserve(static_cast<qsizetype>(values.size()));
    for (const auto &value : values)
        out.push_back(QString::fromStdString(value));
    return out;
}

// Position and normal, interleaved 6 floats per vertex -- the exact layout
// MeshGeometry::vertices expects. Packed here rather than in MeshGeometry itself because
// this is the one place that holds an `archie::visual::Mesh` and a reason to know Quick3D
// wants its buffer this way; MeshGeometry's job stops at handing bytes to Quick3D.
QByteArray packInterleavedVertices(const archie::visual::Mesh &mesh)
{
    QByteArray out;
    out.resize(static_cast<qsizetype>(mesh.vertices.size() * 6 * sizeof(float)));
    auto *dst = reinterpret_cast<float *>(out.data());
    for (std::size_t i = 0; i < mesh.vertices.size(); ++i) {
        const archie::visual::Vec3 &v = mesh.vertices[i];
        // A mesh with mismatched vertex/normal counts is not one archie::visual ever
        // produces, but a defaulted normal keeps a display bug from becoming a crash.
        const archie::visual::Vec3 n = i < mesh.normals.size() ? mesh.normals[i] : archie::visual::Vec3{};
        dst[i * 6 + 0] = v.x;
        dst[i * 6 + 1] = v.y;
        dst[i * 6 + 2] = v.z;
        dst[i * 6 + 3] = n.x;
        dst[i * 6 + 4] = n.y;
        dst[i * 6 + 5] = n.z;
    }
    return out;
}

QByteArray packIndices(const archie::visual::Mesh &mesh)
{
    return QByteArray(reinterpret_cast<const char *>(mesh.indices.data()),
                       static_cast<qsizetype>(mesh.indices.size() * sizeof(std::uint32_t)));
}

/// Everything `loadBuildings` produces, computed off the GUI thread. Plain value types
/// only (QVariantMap/QVariantList of QByteArray/QString/int) -- nothing here is a QObject
/// or touches QML, so building this on a worker thread and handing it back by value is
/// safe by construction rather than by care taken at each call site.
struct BuildingLoadResult {
    QVariantMap state;
    QVariantList meshes;
};

/// Reads `ARCHIE_OSM_EXTRACT`, parses it if set, and builds every drawable footprint's
/// geometry. Runs entirely off the GUI thread via QtConcurrent -- a real Geofabrik extract
/// can hold tens of thousands of ways, and parsing plus extruding even a few hundred of
/// them is real work a 16 ms frame budget cannot absorb.
BuildingLoadResult computeBuildingLoadResult(const archie::visual::GlobePolicy &globePolicy)
{
    // A real Geofabrik extract can be a country or a continent; drawing every building it
    // contains as a separate Quick3D model is not a limit this feature has actually been
    // exercised against, so a hard cap is the honest choice over pretending there isn't
    // one. `buildings.reason` says so when it bites.
    constexpr int kMaxBuildingsRendered = 500;

    const char *envPath = std::getenv("ARCHIE_OSM_EXTRACT");
    if (envPath == nullptr || *envPath == '\0') {
        return BuildingLoadResult{
            QVariantMap{
                {QStringLiteral("available"), false},
                {QStringLiteral("loading"), false},
                {QStringLiteral("reason"),
                 QStringLiteral("no .osm.pbf extract configured (set ARCHIE_OSM_EXTRACT)")},
                {QStringLiteral("count"), 0},
            },
            QVariantList{},
        };
    }

    QFile file(QString::fromLocal8Bit(envPath));
    if (!file.open(QIODevice::ReadOnly)) {
        return BuildingLoadResult{
            QVariantMap{
                {QStringLiteral("available"), false},
                {QStringLiteral("loading"), false},
                {QStringLiteral("reason"), QStringLiteral("could not open %1").arg(file.fileName())},
                {QStringLiteral("count"), 0},
            },
            QVariantList{},
        };
    }
    const QByteArray raw = file.readAll();
    const std::vector<std::uint8_t> bytes(raw.begin(), raw.end());

    const archie::osm::ParseOutcome outcome = archie::osm::parse_buildings(bytes);
    if (!outcome.ok()) {
        return BuildingLoadResult{
            QVariantMap{
                {QStringLiteral("available"), false},
                {QStringLiteral("loading"), false},
                {QStringLiteral("reason"), QString::fromStdString(outcome.error)},
                {QStringLiteral("count"), 0},
            },
            QVariantList{},
        };
    }

    // Classified in one honest pass over every building the parser found -- deliberately
    // not `break`ing once the cap is reached. An earlier version did break early, which
    // stopped counting `skippedNoHeight` at the same point: against a fixture with only a
    // couple of buildings that never showed, but a real extract's 17,000+ ways exposed it
    // immediately as a badly wrong "buildings with a known height" figure, because most of
    // the file past the 500th match was simply never looked at to classify it.
    QVariantList meshes;
    int rendered = 0;
    int skippedNoHeight = 0;
    int skippedOverCap = 0;
    for (const archie::osm::Building &building : outcome.buildings) {
        if (!building.height_meters.has_value() || building.footprint.size() < 4) {
            ++skippedNoHeight;
            continue;
        }
        if (rendered >= kMaxBuildingsRendered) {
            ++skippedOverCap;
            continue;
        }

        // Each building projects onto its own local tangent plane, anchored at its own
        // first point: a footprint is small enough that the curvature this approximation
        // ignores is unmeasurable, and anchoring per-building rather than to one shared
        // origin keeps every local coordinate small regardless of where in the extract
        // the building sits.
        const archie::visual::GeoPoint origin = building.footprint.front();
        archie::visual::Ring ring;
        ring.reserve(building.footprint.size());
        for (const archie::visual::GeoPoint &point : building.footprint)
            ring.push_back(archie::visual::local_position(origin, point));

        const auto height = static_cast<float>(*building.height_meters);
        const archie::visual::Mesh localMesh = archie::visual::extrude_polygon(ring, height);
        if (localMesh.vertices.empty()) {
            ++skippedNoHeight;
            continue;
        }
        const archie::visual::Mesh placed =
            archie::visual::place_on_globe(localMesh, origin, globePolicy);

        meshes.push_back(QVariantMap{
            {QStringLiteral("vertices"), packInterleavedVertices(placed)},
            {QStringLiteral("triangleIndices"), packIndices(placed)},
        });
        ++rendered;
    }

    QString reason;
    const auto totalFound = static_cast<int>(outcome.buildings.size());
    if (skippedOverCap > 0) {
        reason = QStringLiteral("showing %1 of %2 buildings with a known height (limit reached)")
                     .arg(rendered)
                     .arg(rendered + skippedOverCap);
    } else if (skippedNoHeight > 0) {
        reason = QStringLiteral("%1 of %2 buildings skipped: no known height")
                     .arg(skippedNoHeight)
                     .arg(totalFound);
    }

    return BuildingLoadResult{
        QVariantMap{
            {QStringLiteral("available"), rendered > 0},
            {QStringLiteral("loading"), false},
            {QStringLiteral("reason"), reason},
            {QStringLiteral("count"), rendered},
            {QStringLiteral("totalFound"), totalFound},
            {QStringLiteral("parseWarnings"), toStringList(outcome.warnings)},
            {QStringLiteral("source"), file.fileName()},
        },
        meshes,
    };
}

/// The model resolved this colour from severity and freshness. Nothing is decided here.
QColor toQColor(const archie::visual::NodeColour &colour)
{
    QColor out;
    out.setRgbF(std::clamp(colour.base.r, 0.0F, 1.0F), std::clamp(colour.base.g, 0.0F, 1.0F),
                std::clamp(colour.base.b, 0.0F, 1.0F), std::clamp(colour.alpha, 0.0F, 1.0F));
    return out;
}

/// The interface calls these views WORLD and CORE. The model calls them Watcher and
/// Cerebra, and the model's names are the ones with the meanings attached: Cerebra is
/// the view of record and shows every kind including ones this build cannot classify,
/// while Watcher shows only what has a place on the globe.
QString modeName(archie::visual::SceneMode mode)
{
    using archie::visual::SceneMode;
    switch (mode) {
    case SceneMode::Cerebra: return QStringLiteral("cerebra");
    case SceneMode::Brain: return QStringLiteral("brain");
    case SceneMode::Watcher: return QStringLiteral("watcher");
    case SceneMode::Fabric: return QStringLiteral("fabric");
    }
    return QStringLiteral("cerebra");
}

std::optional<archie::visual::SceneMode> modeFromName(const QString &name)
{
    using archie::visual::SceneMode;
    if (name == QStringLiteral("cerebra") || name == QStringLiteral("core"))
        return SceneMode::Cerebra;
    if (name == QStringLiteral("watcher") || name == QStringLiteral("world"))
        return SceneMode::Watcher;
    if (name == QStringLiteral("brain")) return SceneMode::Brain;
    if (name == QStringLiteral("fabric")) return SceneMode::Fabric;
    // An unknown name is a bug in the caller, and switching to some default would hide
    // it behind a view that looks deliberate.
    return std::nullopt;
}

/// Whether the camera is settled, and if not, what it is doing.
///
/// Carried so the interface can tell a still frame from a stalled one. A camera that has
/// stopped because it arrived and a camera that has stopped because the model is no
/// longer being stepped look identical on screen.
QString cameraStateName(archie::visual::CameraState state)
{
    using archie::visual::CameraState;
    switch (state) {
    case CameraState::Resting: return QStringLiteral("resting");
    case CameraState::Transitioning: return QStringLiteral("transitioning");
    case CameraState::Focused: return QStringLiteral("focused");
    case CameraState::Returning: return QStringLiteral("returning");
    }
    return QStringLiteral("resting");
}

QString tierName(archie::visual::RenderTier tier)
{
    using archie::visual::RenderTier;
    switch (tier) {
    case RenderTier::Ultra: return QStringLiteral("ultra");
    case RenderTier::Normal: return QStringLiteral("normal");
    case RenderTier::Conserve: return QStringLiteral("conserve");
    case RenderTier::Headless: return QStringLiteral("headless");
    }
    return QStringLiteral("normal");
}

/// Why the governor settled where it did.
///
/// Carried so the interface can say "conserve — CPU saturated" rather than showing a
/// degraded scene with no explanation. A display that quietly gets worse teaches its
/// operator to distrust it.
QString tierReasonText(archie::visual::TierReason reason)
{
    using archie::visual::TierReason;
    switch (reason) {
    case TierReason::DisplayAbsent: return QStringLiteral("no display attached");
    case TierReason::ThermalCritical: return QStringLiteral("thermally critical");
    case TierReason::ThermalThrottling: return QStringLiteral("thermally throttling");
    case TierReason::ThermalWarm: return QStringLiteral("running warm");
    case TierReason::PressureSaturated: return QStringLiteral("the machine is saturated");
    case TierReason::PressureContended: return QStringLiteral("the machine is contended");
    case TierReason::PressureLight: return QStringLiteral("light load");
    case TierReason::PressureIdle: return QStringLiteral("the machine is idle");
    case TierReason::TelemetryUnavailable: return QStringLiteral("telemetry unavailable");
    case TierReason::TelemetryStale: return QStringLiteral("telemetry too old to trust");
    }
    return QStringLiteral("telemetry unavailable");
}

QVariantMap makeEdge(const QVector3D &from, const QVector3D &to, const QColor &color)
{
    const QVector3D delta = to - from;
    const float length = delta.length();
    const auto rotation = QQuaternion::rotationTo(QVector3D(0.0F, 1.0F, 0.0F), delta.normalized());
    return {{QStringLiteral("position"), QVariant::fromValue((from + to) * 0.5F)},
            {QStringLiteral("rotation"), QVariant::fromValue(rotation)},
            {QStringLiteral("length"), length},
            {QStringLiteral("color"), color}};
}
}

ArchieBridge::ArchieBridge(QObject *parent, bool shellMode)
    : QObject(parent), m_observatory(scenePolicy())
{
    // Advancing is not the same as a payload arriving. Freshness decays, entries and
    // departures play out, and the camera moves between snapshots, so the model has to
    // be stepped on its own clock. Sixty times a second; the tier the model reports
    // decides what the renderer does with each frame, not how often it gets one.
    m_frameTimer.setInterval(16);
    m_frameTimer.setTimerType(Qt::PreciseTimer);
    connect(&m_frameTimer, &QTimer::timeout, this, &ArchieBridge::tick);

    m_snapshotTimer.setInterval(1000);
    m_snapshotTimer.setTimerType(Qt::CoarseTimer);
    connect(&m_snapshotTimer, &QTimer::timeout, this, &ArchieBridge::refreshSnapshot);

    m_worldTimer.setInterval(15000);
    m_worldTimer.setTimerType(Qt::VeryCoarseTimer);
    connect(&m_worldTimer, &QTimer::timeout, this, &ArchieBridge::refreshWorld);

    // PSI averages over ten seconds at its shortest window, so polling faster returns
    // the same numbers. Two seconds is slow enough to cost nothing and fast enough that
    // a machine cannot finish thrashing before the display admits it started.
    m_kernelTimer.setInterval(2000);
    m_kernelTimer.setTimerType(Qt::CoarseTimer);
    connect(&m_kernelTimer, &QTimer::timeout, this, &ArchieBridge::refreshKernel);

    // Polling changes semantic state as well as geometry. Keep the default responsive,
    // but let constrained WSL graphics stacks trade update rate for headroom.
    bool orbPollValid = false;
    const int orbPollOverride = qEnvironmentVariableIntValue("ARCHIE_ORB_POLL_MS", &orbPollValid);
    m_orbTimer.setInterval(orbPollValid ? std::clamp(orbPollOverride, 200, 2000) : 500);
    m_orbTimer.setTimerType(Qt::CoarseTimer);
    connect(&m_orbTimer, &QTimer::timeout, this, &ArchieBridge::refreshOrb);

    m_clockTimer.setInterval(1000);
    m_clockTimer.setTimerType(Qt::CoarseTimer);
    connect(&m_clockTimer, &QTimer::timeout, this, &ArchieBridge::updateEarthAngle);

    m_kernelTimer.start();
    if (!shellMode) {
        m_snapshotTimer.start();
        m_worldTimer.start();
        m_orbTimer.start();
        m_clockTimer.start();
    }
    m_frameTimer.start();
    updateEarthAngle();
    // One frame before any payload arrives, so the interface starts from what the model
    // says about an empty observatory rather than from an empty QVariantList that means
    // the same thing by accident.
    tick();
    refreshKernel();
    if (!shellMode) {
        refreshSnapshot();
        refreshWorld();
        refreshOrb();
        // World is opt-in: a shell rendering Orb does not fetch its globe assets.
        refreshGeoFabric();
        loadBuildings();
    }
}

void ArchieBridge::setPresented(bool presented)
{
    if (m_presented == presented) return;
    m_presented = presented;
    const auto object = QJsonObject::fromVariantMap(m_kernel);
    const auto now = QDateTime::currentMSecsSinceEpoch();
    const auto elapsed = m_kernelReceivedAt == 0 ? 0 : std::max<qint64>(0, now - m_kernelReceivedAt);
    m_observatory.ingest_kernel_payload(archie::native::fromJson(object),
        ageOf(object, QStringLiteral("staleness_ms")) + elapsed, m_presented, now);
    tick();
    emit presentedChanged();
}

void ArchieBridge::refreshSnapshot()
{
    request(Projection::Snapshot, QUrl(QString::fromLatin1(snapshotUrl)));
}

void ArchieBridge::refreshWorld()
{
    request(Projection::World, QUrl(QString::fromLatin1(worldUrl)));
}

void ArchieBridge::refreshKernel()
{
    request(Projection::Kernel, QUrl(QString::fromLatin1(kernelUrl)));
}

void ArchieBridge::refreshOrb()
{
    if (m_orbRequestPending)
        return;
    m_orbRequestPending = true;
    request(Projection::Orb, QUrl(QString::fromLatin1(orbUrl)));
}

void ArchieBridge::refreshGeoFabric()
{
    request(Projection::GeoFabric, QUrl(QString::fromLatin1(geoFabricUrl)));
}

void ArchieBridge::loadBuildings()
{
    // Bumped before the background task starts, and checked when it finishes: a result
    // that comes back for a generation that is no longer current was superseded by a
    // later call to this function and must be discarded, not applied.
    const int generation = ++m_buildingsGeneration;

    m_buildings = QVariantMap{
        {QStringLiteral("available"), false},
        {QStringLiteral("loading"), true},
        {QStringLiteral("reason"), QString()},
        {QStringLiteral("count"), 0},
    };
    m_buildingMeshes = QVariantList{};
    emit buildingsChanged();

    // The same GlobePolicy the rest of the scene places markers with (via
    // ObservatoryPolicy::layout::globe) -- a building that used a different radius would
    // sit at the wrong distance from every marker sharing its coordinate. Read here, on
    // the calling (GUI) thread, and copied into the background task rather than read
    // again there: `scenePolicy()` is cheap and pure, but reaching back into anything
    // Qt-affine from a worker thread is a habit worth not forming even where it happens
    // to be safe today.
    const archie::visual::GlobePolicy globePolicy = scenePolicy().layout.globe;

    auto *watcher = new QFutureWatcher<BuildingLoadResult>(this);
    connect(watcher, &QFutureWatcher<BuildingLoadResult>::finished, this, [this, watcher, generation] {
        watcher->deleteLater();
        if (generation != m_buildingsGeneration)
            return;
        const BuildingLoadResult result = watcher->result();
        m_buildings = result.state;
        m_buildingMeshes = result.meshes;
        emit buildingsChanged();
    });
    watcher->setFuture(QtConcurrent::run(&computeBuildingLoadResult, globePolicy));
}

void ArchieBridge::request(Projection projection, const QUrl &url)
{
    QNetworkRequest request(url);
    request.setTransferTimeout(1500);
    request.setRawHeader(QByteArrayLiteral("Accept"), QByteArrayLiteral("application/json"));
    request.setAttribute(QNetworkRequest::CacheLoadControlAttribute, QNetworkRequest::AlwaysNetwork);
    auto *reply = m_network.get(request);
    ++m_pendingRequests;
    emit busyChanged();

    connect(reply, &QNetworkReply::finished, this, [this, reply, projection] {
        const auto body = reply->isOpen() ? reply->readAll() : QByteArray{};
        const auto failure = reply->error();
        const auto message = reply->errorString();
        reply->deleteLater();
        if (projection == Projection::Orb)
            m_orbRequestPending = false;
        m_pendingRequests = std::max(0, m_pendingRequests - 1);
        emit busyChanged();

        if (failure != QNetworkReply::NoError) {
            fail(message);
            return;
        }
        const qsizetype maximumBytes = projection == Projection::Orb
                                           ? maximumOrbProjectionBytes
                                           : maximumProjectionBytes;
        if (body.size() > maximumBytes) {
            fail(projection == Projection::Orb
                     ? QStringLiteral("orb projection exceeded the 8 MiB safety limit")
                     : QStringLiteral("projection exceeded the 2 MiB safety limit"));
            return;
        }
        accept(projection, body);
    });
}

void ArchieBridge::accept(Projection projection, const QByteArray &body)
{
    QJsonParseError parseError;
    const auto document = QJsonDocument::fromJson(body, &parseError);
    if (parseError.error != QJsonParseError::NoError || !document.isObject()) {
        fail(QStringLiteral("invalid typed projection"));
        return;
    }
    setConnected(true);
    if (!m_error.isEmpty()) {
        m_error.clear();
        emit errorChanged();
    }

    if (projection == Projection::Snapshot) {
        // A transport timestamp or a changing memory counter is real data, but it is
        // not a topology event. Hash only the fields that can change the CORE graph
        // or its state so the visual pulse means exactly that, never "one second
        // elapsed".
        const auto object = document.object();
        QJsonObject topology;
        for (const auto *key : {"daemon_reachable", "boot", "modules", "nodes", "models"})
            topology.insert(QString::fromLatin1(key), object.value(QString::fromLatin1(key)));
        const auto digest = QCryptographicHash::hash(
            QJsonDocument(topology).toJson(QJsonDocument::Compact), QCryptographicHash::Sha256);
        m_snapshot = document.object().toVariantMap();
        // Age comes from the daemon, which knows when it captured this; the transport
        // does not. `staleness_ms` is displayd's own measure of how old the snapshot was
        // when it handed it over, and the model ages it from there. Passing zero would
        // make a cached projection claim to be current.
        m_observatory.ingest_snapshot(archie::native::fromJson(object),
                                      ageOf(object, QStringLiteral("staleness_ms")),
                                      QDateTime::currentMSecsSinceEpoch());
        emit snapshotChanged();
        if (digest != m_snapshotDigest) {
            m_snapshotDigest = digest;
            ++m_snapshotRevision;
            emit snapshotRevisionChanged();
        }
        return;
    }

    if (projection == Projection::Kernel) {
        // Pressure numbers move every read on a live machine, so hashing the whole body
        // would bump the revision constantly and mean nothing. Hash only the verdicts:
        // the revision then marks a genuine change of condition, which is the only thing
        // worth animating.
        const auto object = document.object();
        QJsonObject condition;
        for (const auto *key : {"verdict", "worst_resource", "thermal_state",
                                "hottest_zone", "pressure_availability",
                                "thermal_availability"})
            condition.insert(QString::fromLatin1(key), object.value(QString::fromLatin1(key)));
        const auto kernelDigest = QCryptographicHash::hash(
            QJsonDocument(condition).toJson(QJsonDocument::Compact), QCryptographicHash::Sha256);

        m_kernel = object.toVariantMap();
        m_kernelReceivedAt = QDateTime::currentMSecsSinceEpoch();
        // `display_present` is what separates "nothing is being presented" from "the
        // machine is idle", and the two want opposite tiers. This process has a window,
        // so a display is present by construction — asserting it from somewhere else
        // would be inventing a fact we already have.
        m_observatory.ingest_kernel_payload(archie::native::fromJson(object),
                                            ageOf(object, QStringLiteral("staleness_ms")),
                                            m_presented,
                                            QDateTime::currentMSecsSinceEpoch());
        emit kernelChanged();
        if (kernelDigest != m_kernelDigest) {
            m_kernelDigest = kernelDigest;
            ++m_kernelRevision;
            emit kernelRevisionChanged();
        }
        return;
    }

    if (projection == Projection::GeoFabric) {
        // No digest, no revision: this value cannot change between two fetches in the
        // same process, so there is nothing here for a revision counter to be counting.
        m_geoFabric = document.object().toVariantMap();
        emit geoFabricChanged();
        return;
    }

    if (projection == Projection::Orb) {
        m_orb = document.object().toVariantMap();
        ++m_orbRevision;
        emit orbChanged();
        return;
    }

    const auto digest = QCryptographicHash::hash(body, QCryptographicHash::Sha256);
    m_observatory.ingest_world_payload(archie::native::fromJson(document.object()),
                                       ageOf(document.object(), QStringLiteral("staleness_ms")),
                                       QDateTime::currentMSecsSinceEpoch());
    // Stored as it arrived. The scene position, colour and stand-off used to be
    // computed here and are now the model's, so re-projecting on the way in would
    // leave three fields nothing reads and one more place to disagree.
    m_world = document.object().toVariantMap();
    emit worldChanged();
    if (digest != m_worldDigest) {
        m_worldDigest = digest;
        ++m_worldRevision;
        emit worldRevisionChanged();
    }
}


void ArchieBridge::fail(const QString &message)
{
    setConnected(false);
    if (m_error == message)
        return;
    m_error = message.left(240);
    emit errorChanged();
}

void ArchieBridge::setConnected(bool value)
{
    if (m_connected == value)
        return;
    m_connected = value;
    emit connectedChanged();
}

void ArchieBridge::updateEarthAngle()
{
    const auto utc = QDateTime::currentDateTimeUtc().time();
    const float seconds = utc.hour() * 3600.0F + utc.minute() * 60.0F + utc.second();
    const float next = std::fmod(180.0F + seconds / 240.0F, 360.0F);
    if (qFuzzyCompare(m_earthAngle, next))
        return;
    m_earthAngle = next;
    emit earthAngleChanged();
}

void ArchieBridge::tick()
{
    const auto now = QDateTime::currentMSecsSinceEpoch();
    const auto frame = m_observatory.tick(now);
    // Hidden windows retain a slow freshness clock; the model's budget drives
    // visible presentation. Zero is never passed to a repeating QTimer.
    const int interval = m_presented && frame.profile.frame_interval_ms > 0
        ? frame.profile.frame_interval_ms : 1000;
    if (m_frameTimer.interval() != interval) m_frameTimer.setInterval(interval);

    // The record an observation came from, so the inspector can show magnitude and place
    // without a second list to keep in step. The model decides where the marker goes and
    // what colour it is; this only carries the detail it was never asked to hold.
    QHash<QString, QVariant> observationById;
    for (const auto &entry : m_world.value(QStringLiteral("observations")).toList()) {
        const auto record = entry.toMap();
        const auto id = record.value(QStringLiteral("id")).toString();
        if (!id.isEmpty())
            observationById.insert(id, entry);
    }

    QHash<QString, QVector3D> positions;
    QVariantList nodes;
    nodes.reserve(static_cast<qsizetype>(frame.nodes.size()));
    for (const auto &node : frame.nodes) {
        const QVector3D position(node.position.x, node.position.y, node.position.z);
        const auto id = QString::fromStdString(node.id);
        positions.insert(id, position);
        QVariantMap entry{
            {QStringLiteral("id"), id},
            {QStringLiteral("label"), QString::fromStdString(node.label)},
            {QStringLiteral("detail"), QString::number(node.severity)},
            {QStringLiteral("position"), QVariant::fromValue(position)},
            {QStringLiteral("color"), toQColor(node.colour)},
            // Quick3D's `#Sphere` is 100 units across, so this is a node half a unit wide
            // against the six-unit innermost shell. The model bounds its own factor; an
            // unbounded provider magnitude must not be able to produce a node that fills
            // the screen.
            {QStringLiteral("scale"), 0.010F * node.scale},
            {QStringLiteral("emissive"), node.colour.emissive},
            {QStringLiteral("confidence"), node.confidence},
            {QStringLiteral("authoritative"), node.authoritative},
            // Whether this node may move at all. Decided by the model from freshness and
            // what the tier can afford, so no renderer has to remember to ask.
            {QStringLiteral("animate"), node.animate},
            {QStringLiteral("presenceProgress"), node.presence_progress},
            {QStringLiteral("onGlobe"), node.location.has_value()},
        };
        if (const auto found = observationById.constFind(id);
            found != observationById.constEnd()) {
            entry.insert(QStringLiteral("payload"), *found);
        }
        nodes.push_back(std::move(entry));
    }

    QVariantList edges;
    edges.reserve(static_cast<qsizetype>(frame.edges.size()));
    for (const auto &edge : frame.edges) {
        const auto from = QString::fromStdString(edge.from);
        const auto to = QString::fromStdString(edge.to);
        // An edge to a node the budget aggregated away has nowhere to land. The scene
        // reports the aggregation separately; drawing a line into empty space would be
        // the display inventing a relationship it cannot show either end of.
        if (!positions.contains(from) || !positions.contains(to))
            continue;
        edges.push_back(makeEdge(positions.value(from), positions.value(to),
                                 QColor(QStringLiteral("#557786"))));
    }

    const QVariantMap summary = QVariantMap{
        {QStringLiteral("mode"), modeName(frame.mode)},
        {QStringLiteral("incomingMode"), modeName(frame.incoming_mode)},
        {QStringLiteral("morphProgress"), frame.morph_progress},
        {QStringLiteral("tier"), tierName(frame.tier)},
        {QStringLiteral("tierReason"), tierReasonText(frame.tier_reason)},
        // What the tier actually permits, in the renderer's own units. These are the
        // model's numbers, not a second reading of the same telemetry: the display used
        // to map kernel verdicts to a budget word and then map that word to a sample
        // count, which is two chances to disagree with the thing that decided the tier.
        {QStringLiteral("frameIntervalMs"), frame.profile.frame_interval_ms},
        {QStringLiteral("msaaSamples"), frame.profile.msaa_samples},
        {QStringLiteral("maxVisibleNodes"), frame.profile.max_visible_nodes},
        {QStringLiteral("maxParticles"), frame.profile.max_particles},
        {QStringLiteral("postProcessing"), frame.profile.post_processing},
        // Separate from post-processing because it costs CPU rather than GPU, and the
        // two run out at different times.
        {QStringLiteral("ambientMotion"), frame.profile.ambient_motion},
        {QStringLiteral("focused"), QString::fromStdString(frame.focused)},
        {QStringLiteral("totalNodes"), frame.total_nodes},
        // Distinct on purpose: an aggregated node did not fit the tier's budget, and a
        // filtered one was never meant to be in this view. Reporting them as one number
        // would send somebody looking for a budget problem that does not exist.
        {QStringLiteral("aggregated"), frame.aggregated},
        {QStringLiteral("filteredOut"), frame.filtered_out},
        {QStringLiteral("eventsElsewhere"), frame.events_elsewhere},
        {QStringLiteral("dependencyCycle"), toStringList(frame.dependency_cycle)},
        {QStringLiteral("collidingIds"), toStringList(frame.colliding_ids)},
        // Kept apart because the two arrive from different daemons on different
        // schedules, and merging them would make a defect in one look like a defect in
        // the other.
        {QStringLiteral("ingest"), ingestSummary(frame.ingest)},
        {QStringLiteral("worldIngest"), ingestSummary(frame.world_ingest)},
        // The camera is the model's too. It owns the eases between views, the framing of
        // a focused node and the way back when focus is released — and it lets go of a
        // node the graph has forgotten, which a camera parked in QML cannot do because it
        // does not know the node is gone. A renderer animating its own camera against the
        // same durations would be a second clock keeping the same appointment.
        {QStringLiteral("cameraEye"),
         QVariant::fromValue(QVector3D(frame.camera.eye.x, frame.camera.eye.y, frame.camera.eye.z))},
        {QStringLiteral("cameraTarget"),
         QVariant::fromValue(
             QVector3D(frame.camera.target.x, frame.camera.target.y, frame.camera.target.z))},
        {QStringLiteral("cameraFov"), frame.camera.fov_degrees},
        {QStringLiteral("cameraState"), cameraStateName(frame.camera_state)},
    };

    // Only publish a frame that differs from the one already on screen.
    //
    // The model is stepped sixty times a second because it has to be — freshness decays,
    // entries and departures play out, and the camera moves between payloads. But an
    // observatory watching an idle node with nothing animating produces the same frame
    // every time, and emitting it anyway makes QML re-evaluate every binding in the
    // interface forever. That is not a micro-optimisation: it is the difference between
    // an idle display costing nothing and costing a core.
    //
    // The comparison is not free either, and it is deliberately the cheaper half: it
    // fails on the first node that moved, whereas the alternative is a full re-render.
    //
    // Measured rather than assumed, by building it both ways and sampling
    // `TotalProcessorTime` over three twenty-second windows after a twenty-second settle,
    // idle, on a Ryzen 5 4500U: 16.0/18.1/20.8% of one core without this check, and
    // 9.8/10.2/10.1% with it. Roughly half, and steadier — the unguarded figure wanders
    // because it is re-evaluating every binding in the interface sixty times a second.
    //
    // One machine, one scene, one node in it. What this does at a node count where the
    // tier budget actually bites is not known, and the number that would justify tuning
    // it further does not exist yet.
    if (m_sceneNodes == nodes && m_sceneEdges == edges && m_frame == summary)
        return;

    m_sceneNodes = std::move(nodes);
    m_sceneEdges = std::move(edges);
    m_frame = summary;

    emit frameChanged();
}

void ArchieBridge::requestMode(const QString &mode)
{
    if (const auto resolved = modeFromName(mode))
        m_observatory.request_mode(*resolved);
    else
        qWarning("archie-display: unknown scene mode %s", qPrintable(mode));
}

void ArchieBridge::focusNode(const QString &id)
{
    m_observatory.focus(id.toStdString());
}

void ArchieBridge::releaseFocus()
{
    m_observatory.release_focus();
}

void ArchieBridge::orbitGlobe(float longitudeDelta, float latitudeDelta, float zoomSteps)
{
    m_observatory.orbit_globe(longitudeDelta, latitudeDelta, zoomSteps);
    tick();
}
