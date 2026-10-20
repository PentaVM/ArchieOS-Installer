#include "LumenGeometry.h"

#include <QVector3D>

#include <algorithm>
#include <cstring>
#include <cmath>
#include <numbers>
#include <limits>

namespace {

QByteArray bytes(const void *data, std::size_t size)
{
    return QByteArray(static_cast<const char *>(data), static_cast<qsizetype>(size));
}

}  // namespace

LumenGeometry::LumenGeometry(QQuick3DObject *parent) : QQuick3DGeometry(parent) {}

void LumenGeometry::bound(const std::vector<float> &vertices, int stride)
{
    constexpr float big = std::numeric_limits<float>::max();
    QVector3D low(big, big, big);
    QVector3D high(-big, -big, -big);
    for (std::size_t i = 0; i + 2 < vertices.size(); i += static_cast<std::size_t>(stride)) {
        const QVector3D p(vertices[i], vertices[i + 1], vertices[i + 2]);
        low = QVector3D(std::min(low.x(), p.x()), std::min(low.y(), p.y()), std::min(low.z(), p.z()));
        high = QVector3D(std::max(high.x(), p.x()), std::max(high.y(), p.y()), std::max(high.z(), p.z()));
    }
    if (vertices.empty())
        low = high = QVector3D();
    setBounds(low, high);
}

void LumenGeometry::setLines(const std::vector<float> &points)
{
    m_lines = points;
    if (m_ribbons) {
        std::vector<float> vertices;
        std::vector<std::uint32_t> indices;
        vertices.reserve(points.size() / 6 * 40);
        indices.reserve(points.size());
        for (std::size_t k = 0; k + 5 < points.size(); k += 6) {
            const auto first = static_cast<std::uint32_t>(vertices.size() / 10);
            const float seed=std::fmod(float(k/6)*0.61803398875F,1.F);
            for (const auto uv : {std::pair{0.F,0.F},std::pair{1.F,0.F},std::pair{0.F,1.F},std::pair{1.F,1.F}})
                vertices.insert(vertices.end(), {points[k],points[k+1],points[k+2],points[k+3],points[k+4],points[k+5],uv.first,uv.second,seed,0.F});
            indices.insert(indices.end(), {first,first+1,first+2,first+2,first+1,first+3});
        }
        setTinted(vertices,indices);
        // Endpoint B is in the normal channel; include both endpoints in culling.
        bound(points,3);
        return;
    }
    clear();
    if (points.size() >= 6) {
        setStride(3 * sizeof(float));
        setPrimitiveType(PrimitiveType::Lines);
        addAttribute(Attribute::PositionSemantic, 0, Attribute::F32Type);
        setVertexData(bytes(points.data(), points.size() * sizeof(float)));
        bound(points, 3);
    }
    update();
}

void LumenGeometry::setRibbons(bool enabled)
{
    if (enabled == m_ribbons) return;
    m_ribbons = enabled;
    const auto points = m_lines;
    setLines(points);
    emit ribbonsChanged();
}

void LumenGeometry::setSolid(const std::vector<float> &vertices, const std::vector<std::uint32_t> &indices)
{
    // Preserve every original position, normal and index. The original/category
    // fallback also supplies spherical UVs for Xavi's matte-ceramic material.
    std::vector<float> textured;
    textured.reserve(vertices.size()/6*8);
    for(std::size_t k=0;k+5<vertices.size();k+=6){
        textured.insert(textured.end(),vertices.begin()+k,vertices.begin()+k+6);
        const float u=std::atan2(vertices[k+2],-vertices[k])/(2*std::numbers::pi_v<float>)+.5F;
        const float v=.5F-std::atan2(-vertices[k+1],std::hypot(vertices[k],vertices[k+2]))/std::numbers::pi_v<float>;
        textured.insert(textured.end(),{u,v});
    }
    setTextured(textured,indices);
}

void LumenGeometry::setTextured(const std::vector<float> &vertices, const std::vector<std::uint32_t> &indices)
{
    clear();
    if (!vertices.empty() && !indices.empty()) {
        setStride(8 * sizeof(float));
        setPrimitiveType(PrimitiveType::Triangles);
        addAttribute(Attribute::PositionSemantic, 0, Attribute::F32Type);
        addAttribute(Attribute::NormalSemantic, 3 * sizeof(float), Attribute::F32Type);
        addAttribute(Attribute::TexCoord0Semantic, 6 * sizeof(float), Attribute::F32Type);
        addAttribute(Attribute::IndexSemantic, 0, Attribute::U32Type);
        setVertexData(bytes(vertices.data(), vertices.size() * sizeof(float)));
        setIndexData(bytes(indices.data(), indices.size() * sizeof(std::uint32_t)));
        bound(vertices, 8);
    }
    update();
}

void LumenGeometry::setTinted(const std::vector<float> &vertices, const std::vector<std::uint32_t> &indices)
{
    clear();
    if (!vertices.empty() && !indices.empty()) {
        setStride(10 * sizeof(float));
        setPrimitiveType(PrimitiveType::Triangles);
        addAttribute(Attribute::PositionSemantic, 0, Attribute::F32Type);
        addAttribute(Attribute::NormalSemantic, 3 * sizeof(float), Attribute::F32Type);
        addAttribute(Attribute::TexCoord0Semantic, 6 * sizeof(float), Attribute::F32Type);
        addAttribute(Attribute::TexCoord1Semantic, 8 * sizeof(float), Attribute::F32Type);
        addAttribute(Attribute::IndexSemantic, 0, Attribute::U32Type);
        setVertexData(bytes(vertices.data(), vertices.size() * sizeof(float)));
        setIndexData(bytes(indices.data(), indices.size() * sizeof(std::uint32_t)));
        bound(vertices, 10);
    }
    update();
}

void LumenGeometry::setQuad()
{
    const std::vector<float> vertices{-1, -1, 0, 0, 0, 1, 1, -1, 0, 0, 0, 1,
                                      1,  1,  0, 0, 0, 1, -1, 1, 0, 0, 0, 1};
    setSolid(vertices, {0, 1, 2, 0, 2, 3});
}

LumenInstancing::LumenInstancing(QQuick3DObject *parent) : QQuick3DInstancing(parent) {}

void LumenInstancing::setTable(QByteArray table, int count)
{
    if (m_count == std::max(0, count) && m_table == table) return;
    m_table = std::move(table);
    m_count = std::max(0, count);
    markDirty();
}

QByteArray LumenInstancing::getInstanceBuffer(int *instanceCount)
{
    if (instanceCount)
        *instanceCount = m_count;
    return m_table;
}
