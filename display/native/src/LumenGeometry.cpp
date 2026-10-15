#include "LumenGeometry.h"

#include <QVector3D>

#include <algorithm>
#include <cstring>
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

void LumenGeometry::setSolid(const std::vector<float> &vertices, const std::vector<std::uint32_t> &indices)
{
    clear();
    if (!vertices.empty() && !indices.empty()) {
        setStride(6 * sizeof(float));
        setPrimitiveType(PrimitiveType::Triangles);
        addAttribute(Attribute::PositionSemantic, 0, Attribute::F32Type);
        addAttribute(Attribute::NormalSemantic, 3 * sizeof(float), Attribute::F32Type);
        addAttribute(Attribute::IndexSemantic, 0, Attribute::U32Type);
        setVertexData(bytes(vertices.data(), vertices.size() * sizeof(float)));
        setIndexData(bytes(indices.data(), indices.size() * sizeof(std::uint32_t)));
        bound(vertices, 6);
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
