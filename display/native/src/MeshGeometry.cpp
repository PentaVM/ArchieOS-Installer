#include "MeshGeometry.h"

MeshGeometry::MeshGeometry(QQuick3DGeometry *parent) : QQuick3DGeometry(parent)
{
}

void MeshGeometry::setVertices(const QByteArray &data)
{
    if (m_vertices == data)
        return;
    m_vertices = data;
    rebuild();
    emit meshChanged();
}

void MeshGeometry::setTriangleIndices(const QByteArray &data)
{
    if (m_triangleIndices == data)
        return;
    m_triangleIndices = data;
    rebuild();
    emit meshChanged();
}

void MeshGeometry::rebuild()
{
    clear();
    if (m_vertices.isEmpty() || m_triangleIndices.isEmpty()) {
        // Nothing real to draw -- an empty footprint, or one still waiting on data --
        // is a cleared geometry, not a shape made of zero-length garbage.
        update();
        return;
    }

    constexpr int kStride = 6 * static_cast<int>(sizeof(float));  // position.xyz + normal.xyz
    setStride(kStride);
    setVertexData(m_vertices);
    addAttribute(QQuick3DGeometry::Attribute::PositionSemantic, 0,
                 QQuick3DGeometry::Attribute::F32Type);
    addAttribute(QQuick3DGeometry::Attribute::NormalSemantic, 3 * static_cast<int>(sizeof(float)),
                 QQuick3DGeometry::Attribute::F32Type);

    setIndexData(m_triangleIndices);
    addAttribute(QQuick3DGeometry::Attribute::IndexSemantic, 0,
                 QQuick3DGeometry::Attribute::U32Type);

    setPrimitiveType(QQuick3DGeometry::PrimitiveType::Triangles);
    update();
}
