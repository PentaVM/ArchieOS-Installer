#pragma once

// The buffers Lumenfield draws from, in the three shapes it needs: bare line segments
// (links, relations, fibres, guides), solid triangles with normals (orbs, the selection
// ring) and tinted triangles (the membrane, whose colour travels in two UV channels:
// rg in UV0, ba in UV1, because those are the attributes every custom material can read).
//
// Nothing here decides a shape. `LumenField` and `archie::visual::lumen` do; this only
// tells Quick3D how the bytes are laid out.

#include <QQuick3DGeometry>
#include <QQuick3DInstancing>
#include <QtQml/qqmlregistration.h>

#include <cstdint>
#include <vector>

class LumenGeometry : public QQuick3DGeometry
{
    Q_OBJECT
    QML_ELEMENT
    Q_PROPERTY(bool ribbons READ ribbons WRITE setRibbons NOTIFY ribbonsChanged)

public:
    explicit LumenGeometry(QQuick3DObject *parent = nullptr);
    bool ribbons() const { return m_ribbons; }
    void setRibbons(bool enabled);

    /// Segment pairs, three floats a point.
    void setLines(const std::vector<float> &points);
    /// Input position/normal, six floats; uploads the retained mesh with spherical UVs.
    void setSolid(const std::vector<float> &vertices, const std::vector<std::uint32_t> &indices);
    /// Rust-authored geometry: position, normal and UV, eight floats per vertex.
    void setTextured(const std::vector<float> &vertices, const std::vector<std::uint32_t> &indices);
    /// Position, normal and rgba, ten floats a vertex.
    void setTinted(const std::vector<float> &vertices, const std::vector<std::uint32_t> &indices);
    /// One quad facing +Z, corners at ±1, for billboards the vertex shader turns to the camera.
    void setQuad();

signals:
    void ribbonsChanged();

private:
    void bound(const std::vector<float> &vertices, int stride);
    bool m_ribbons = false;
    std::vector<float> m_lines;
};

/// A table of instances handed straight to Quick3D. The table is built by `LumenField`
/// in `QQuick3DInstancing::InstanceTableEntry` layout.
class LumenInstancing : public QQuick3DInstancing
{
    Q_OBJECT
    QML_ELEMENT

public:
    explicit LumenInstancing(QQuick3DObject *parent = nullptr);

    void setTable(QByteArray table, int count);

protected:
    QByteArray getInstanceBuffer(int *instanceCount) override;

private:
    QByteArray m_table;
    int m_count = 0;
};
