#include "archie/visual/lumen.hpp"

#include <algorithm>
#include <array>
#include <cmath>
#include <map>
#include <numbers>
#include <utility>

namespace archie::visual::lumen {

namespace {

constexpr float kTau = 2.0F * std::numbers::pi_v<float>;
constexpr float kGoldenAngle = 2.399963229728653F;

Vec3 add(const Vec3& a, const Vec3& b) { return Vec3{a.x + b.x, a.y + b.y, a.z + b.z}; }
Vec3 sub(const Vec3& a, const Vec3& b) { return Vec3{a.x - b.x, a.y - b.y, a.z - b.z}; }
Vec3 scale(const Vec3& a, float s) { return Vec3{a.x * s, a.y * s, a.z * s}; }
float dot(const Vec3& a, const Vec3& b) { return a.x * b.x + a.y * b.y + a.z * b.z; }
float length(const Vec3& a) { return std::sqrt(dot(a, a)); }
Vec3 cross(const Vec3& a, const Vec3& b) {
    return Vec3{a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x};
}
Vec3 normalize(const Vec3& a) {
    const float l = length(a);
    return l > 1e-12F ? scale(a, 1.0F / l) : Vec3{0.0F, 0.0F, 1.0F};
}

/// The design's `random`: a multiply-add and a fold, as 32-bit integer arithmetic.
float scatter(std::uint32_t seed) noexcept {
    std::uint32_t x = (seed + 1U) * 1664525U + 1013904223U;
    x ^= x >> 16U;
    return static_cast<float>(x) / 4294967295.0F;
}

/// The design's satellite fraction: a 32-bit avalanche.
float avalanche(std::uint32_t value) noexcept {
    std::uint32_t x = value + 0x9e3779b9U;
    x = (x ^ (x >> 16U)) * 0x7feb352dU;
    x = (x ^ (x >> 15U)) * 0x846ca68bU;
    return static_cast<float>(x ^ (x >> 16U)) / 4294967295.0F;
}

float srgb_channel(float c) noexcept {
    return c <= 0.04045F ? c / 12.92F : std::pow((c + 0.055F) / 1.055F, 2.4F);
}

std::vector<std::size_t> evenly(const std::vector<std::size_t>& items, std::size_t limit) {
    const std::size_t count = std::min(items.size(), limit);
    std::vector<std::size_t> out;
    out.reserve(count);
    for (std::size_t i = 0; i < count; ++i) {
        out.push_back(items[i * items.size() / count]);
    }
    return out;
}

std::vector<std::vector<std::size_t>> children_of(const std::vector<int>& parent) {
    std::vector<std::vector<std::size_t>> children(parent.size());
    for (std::size_t i = 0; i < parent.size(); ++i) {
        const int p = parent[i];
        if (p >= 0 && static_cast<std::size_t>(p) < parent.size() && static_cast<std::size_t>(p) != i) {
            children[static_cast<std::size_t>(p)].push_back(i);
        }
    }
    return children;
}

}  // namespace

std::optional<Rgb> parse_hex(std::string_view text) noexcept {
    if (text.size() != 7 || text[0] != '#') {
        return std::nullopt;
    }
    std::array<float, 3> channels{};
    for (std::size_t c = 0; c < 3; ++c) {
        int value = 0;
        for (std::size_t k = 0; k < 2; ++k) {
            const char h = text[1 + c * 2 + k];
            int digit = 0;
            if (h >= '0' && h <= '9') {
                digit = h - '0';
            } else if (h >= 'a' && h <= 'f') {
                digit = h - 'a' + 10;
            } else if (h >= 'A' && h <= 'F') {
                digit = h - 'A' + 10;
            } else {
                return std::nullopt;
            }
            value = value * 16 + digit;
        }
        channels[c] = static_cast<float>(value) / 255.0F;
    }
    return Rgb{channels[0], channels[1], channels[2]};
}

Rgb to_linear(const Rgb& srgb) noexcept {
    return Rgb{srgb_channel(srgb.r), srgb_channel(srgb.g), srgb_channel(srgb.b)};
}

Rgb mix(const Rgb& a, const Rgb& b, float t) noexcept {
    return Rgb{a.r + (b.r - a.r) * t, a.g + (b.g - a.g) * t, a.b + (b.b - a.b) * t};
}

float hash01(std::uint32_t n) noexcept {
    std::uint32_t t = n * 1664525U + 1013904223U;
    t ^= t >> 16U;
    t *= 2246822519U;
    t ^= t >> 13U;
    return static_cast<float>(t) / 4294967295.0F;
}

bool step(std::vector<Body>& bodies, const std::optional<Vec3>& pointer, float dt,
          const Motion& motion) {
    const float h = std::clamp(dt, 0.0F, 0.05F);
    const float damping = std::pow(std::clamp(motion.damping, 0.0F, 0.999F), h * 60.0F);
    bool moving = false;
    for (Body& body : bodies) {
        if (body.fixed) {
            body.position = body.anchor;
            body.velocity = Vec3{};
            continue;
        }
        Vec3 acceleration = scale(sub(body.anchor, body.position), motion.spring);
        if (pointer && motion.reach > 0.0F) {
            Vec3 away = sub(body.position, *pointer);
            float distance = length(away);
            if (distance < 0.001F) {
                away = normalize(length(body.anchor) > 0.0F ? body.anchor : Vec3{1.0F, 0.0F, 0.0F});
                distance = 1.0F;
            }
            const float falloff = std::max(0.0F, 1.0F - distance / motion.reach);
            acceleration = add(acceleration,
                               scale(away, motion.reaction * falloff * falloff / distance));
        }
        body.velocity = scale(add(body.velocity, scale(acceleration, h)), damping);
        body.position = add(body.position, scale(body.velocity, h));
        const float distance = length(body.position);
        if (distance > motion.radius && distance > 0.0F) {
            body.position = scale(body.position, motion.radius / distance);
            body.velocity = scale(body.velocity, 0.3F);
        }
        if (!std::isfinite(body.position.x) || !std::isfinite(body.position.y)
            || !std::isfinite(body.position.z)) {
            body.position = body.anchor;
            body.velocity = Vec3{};
        }
        if (std::fabs(body.velocity.x) + std::fabs(body.velocity.y) + std::fabs(body.velocity.z)
            > 0.004F) {
            moving = true;
        }
    }
    return moving;
}

std::vector<Vec3> focus_region(const std::vector<Vec3>& anchors, const std::vector<int>& parent,
                               int selected, float spread, float field_radius) {
    std::vector<Vec3> out = anchors;
    if (selected < 0 || static_cast<std::size_t>(selected) >= anchors.size() || spread <= 0.0F
        || parent.size() != anchors.size()) {
        return out;
    }
    const Vec3 origin = anchors[static_cast<std::size_t>(selected)];
    const float limit = field_radius * 1.05F;
    for (std::size_t i = 0; i < anchors.size(); ++i) {
        if (static_cast<int>(i) == selected) {
            continue;
        }
        bool inside = false;
        int at = parent[i];
        for (std::size_t guard = 0; at >= 0 && guard < parent.size(); ++guard) {
            if (at == selected) {
                inside = true;
                break;
            }
            at = parent[static_cast<std::size_t>(at)];
        }
        if (!inside) {
            continue;
        }
        Vec3 target = add(origin, scale(sub(anchors[i], origin), 1.0F + spread));
        const float distance = length(target);
        if (distance > limit) {
            target = scale(target, limit / distance);
        }
        out[i] = target;
    }
    return out;
}

Vec3 curve(const Vec3& a, const Vec3& b, float phase, float weight) noexcept {
    const Vec3 d = sub(b, a);
    const float span = length(d) > 0.0F ? length(d) : 1.0F;
    const float bend = span * weight * std::sin(std::numbers::pi_v<float> * phase);
    const float flat = std::hypot(d.x, d.z) > 0.0F ? std::hypot(d.x, d.z) : 1.0F;
    return Vec3{a.x + d.x * phase - d.z / flat * bend,
                a.y + d.y * phase + span * 0.065F * std::sin(std::numbers::pi_v<float> * phase),
                a.z + d.z * phase + d.x / flat * bend};
}

int satellite_budget(std::size_t members) noexcept {
    const double root = std::sqrt(static_cast<double>(std::max<std::size_t>(1, members)));
    return std::clamp(static_cast<int>(std::lround(root * 15.0)), 12, 260);
}

std::vector<Vec3> satellite_offsets(int count, float radius, std::uint32_t seed) {
    std::vector<Vec3> out;
    if (count <= 0) {
        return out;
    }
    out.reserve(static_cast<std::size_t>(count));
    for (int i = 0; i < count; ++i) {
        const float z = 1.0F - 2.0F * (static_cast<float>(i) + 0.5F) / static_cast<float>(count);
        const float azimuth = static_cast<float>(i) * kGoldenAngle + static_cast<float>(seed) * 0.029F;
        const float distance =
            radius * (0.29F + std::pow(avalanche(static_cast<std::uint32_t>(i) + seed * 17U), 0.73F) * 0.71F);
        const float ring = std::sqrt(std::max(0.0F, 1.0F - z * z)) * distance;
        out.push_back(Vec3{std::cos(azimuth) * ring, z * distance, std::sin(azimuth) * ring});
    }
    return out;
}

float satellite_radius(std::size_t members, float field_radius) noexcept {
    return std::min(field_radius * 0.16F,
                    8.0F + std::log1p(static_cast<float>(std::max<std::size_t>(1, members))) * 5.0F);
}

std::vector<Vec3> ghosts(const std::vector<Vec3>& centres, const std::vector<std::size_t>& members,
                         int count, float field_radius) {
    std::vector<Vec3> out;
    if (count <= 0) {
        return out;
    }
    out.reserve(static_cast<std::size_t>(count));
    if (centres.empty() || members.size() != centres.size()) {
        for (int i = 0; i < count; ++i) {
            const auto r = static_cast<std::uint32_t>(i);
            const float s = hash01(r * 7U + 1U) * kTau;
            const float o = hash01(r * 7U + 2U) * 2.0F - 1.0F;
            const float h = std::sqrt(std::max(0.0F, 1.0F - o * o));
            const float c = field_radius * (0.12F + std::pow(hash01(r * 7U + 3U), 0.86F) * 0.79F);
            out.push_back(Vec3{std::cos(s) * h * c, std::sin(s) * h * c, o * c});
        }
        return out;
    }
    std::vector<float> mass(centres.size());
    float total = 0.0F;
    for (std::size_t i = 0; i < centres.size(); ++i) {
        mass[i] = std::sqrt(static_cast<float>(std::max<std::size_t>(1, members[i])));
        total += mass[i];
    }
    for (int i = 0; i < count; ++i) {
        const auto n = static_cast<std::uint32_t>(i);
        float vote = scatter(n * 71U + 33U) * total;
        std::size_t chosen = centres.size() - 1;
        for (std::size_t c = 0; c < centres.size(); ++c) {
            vote -= mass[c];
            if (vote <= 0.0F) {
                chosen = c;
                break;
            }
        }
        const float angle = scatter(n * 11U + 1U) * kTau;
        const float height = scatter(n * 11U + 2U) * 2.0F - 1.0F;
        const float ring = std::sqrt(std::max(0.0F, 1.0F - height * height));
        const float distance =
            (0.24F + std::pow(scatter(n * 11U + 3U), 0.65F) * 0.76F)
            * std::min(field_radius * 0.18F,
                       10.0F + std::log1p(static_cast<float>(members[chosen])) * 5.0F);
        out.push_back(add(centres[chosen], Vec3{std::cos(angle) * ring * distance, height * distance,
                                                std::sin(angle) * ring * distance}));
    }
    return out;
}

std::vector<std::size_t> backbone(const std::vector<bool>& target_is_branch, std::size_t limit) {
    std::vector<std::size_t> branches;
    std::vector<std::size_t> leaves;
    for (std::size_t i = 0; i < target_is_branch.size(); ++i) {
        (target_is_branch[i] ? branches : leaves).push_back(i);
    }
    const std::size_t branch_budget =
        std::min(branches.size(), static_cast<std::size_t>(std::ceil(static_cast<double>(limit) * 0.7)));
    std::vector<std::size_t> out = evenly(branches, branch_budget);
    const std::vector<std::size_t> rest = evenly(leaves, limit > branch_budget ? limit - branch_budget : 0);
    out.insert(out.end(), rest.begin(), rest.end());
    return out;
}

std::vector<std::size_t> neighbourhood(const std::vector<int>& parent, int selected,
                                       std::size_t limit) {
    std::vector<std::size_t> out;
    if (selected < 0 || static_cast<std::size_t>(selected) >= parent.size()) {
        return out;
    }
    const auto children = children_of(parent);
    std::vector<bool> seen(parent.size(), false);
    const auto take = [&](std::size_t id) {
        if (out.size() < limit && !seen[id] && parent[id] >= 0) {
            out.push_back(id);
            seen[id] = true;
        }
    };
    int at = selected;
    for (std::size_t guard = 0; at >= 0 && guard < parent.size(); ++guard) {
        take(static_cast<std::size_t>(at));
        at = parent[static_cast<std::size_t>(at)];
    }
    const auto own = static_cast<std::size_t>(selected);
    for (const std::size_t child : children[own]) {
        take(child);
    }
    if (parent[own] >= 0) {
        const auto& siblings = children[static_cast<std::size_t>(parent[own])];
        for (std::size_t s = 0; s < siblings.size() && s < 24; ++s) {
            take(siblings[s]);
        }
    }
    for (const std::size_t child : children[own]) {
        for (const std::size_t grandchild : children[child]) {
            take(grandchild);
        }
    }
    return out;
}

Membrane membrane(const std::vector<Vec3>& nodes, const std::vector<Rgb>& colours, const Rgb& warm,
                  float field_radius, bool legacy, float shell_scale, int rings, int segments) {
    Membrane out;
    rings = std::clamp(rings, 4, 128);
    segments = std::clamp(segments, 6, 192);
    std::vector<float> distances;
    distances.reserve(nodes.size());
    for (const Vec3& n : nodes) {
        distances.push_back(length(n));
    }
    std::sort(distances.begin(), distances.end());
    const float p96 = distances.empty()
                          ? field_radius * 0.7F
                          : distances[std::min(distances.size() - 1,
                                               static_cast<std::size_t>(static_cast<float>(distances.size()) * 0.96F))];
    const float radius = (legacy ? field_radius : std::min(field_radius, p96 * 1.15F)) * shell_scale;
    out.radius = radius;
    const std::size_t sampled = std::min<std::size_t>(nodes.size(), 320);

    const auto columns = static_cast<std::size_t>(segments) + 1;
    std::vector<Vec3> positions;
    std::vector<Rgb> tints;
    positions.reserve(columns * static_cast<std::size_t>(rings + 1));
    for (int iy = 0; iy <= rings; ++iy) {
        const float v = static_cast<float>(iy) / static_cast<float>(rings);
        for (int ix = 0; ix <= segments; ++ix) {
            const float u = static_cast<float>(ix) / static_cast<float>(segments);
            const Vec3 h = normalize(Vec3{-std::cos(u * kTau) * std::sin(v * std::numbers::pi_v<float>),
                                          std::cos(v * std::numbers::pi_v<float>),
                                          std::sin(u * kTau) * std::sin(v * std::numbers::pi_v<float>)});
            float gathered = 0.0F;
            float nearest = -1.0F;
            Rgb tint = warm;
            for (std::size_t n = 0; n < sampled; ++n) {
                const float u_len = length(nodes[n]) > 0.0F ? length(nodes[n]) : 1.0F;
                const float p = dot(h, nodes[n]) / u_len;
                if (p > 0.83F) {
                    const float t = (p - 0.83F) / 0.17F;
                    gathered += t * t;
                }
                if (p > nearest) {
                    nearest = p;
                    tint = n < colours.size() ? colours[n] : warm;
                }
            }
            const float swell = 1.0F + std::min(0.11F, gathered * 0.004F)
                                + std::sin(h.x * 19.0F + h.y * 14.0F) * 0.009F;
            positions.push_back(scale(h, radius * swell));
            tints.push_back(mix(tint, warm, 0.46F));
        }
    }
    for (int iy = 0; iy < rings; ++iy) {
        for (int ix = 0; ix < segments; ++ix) {
            const auto a = static_cast<std::uint32_t>(static_cast<std::size_t>(iy) * columns + static_cast<std::size_t>(ix) + 1);
            const auto b = static_cast<std::uint32_t>(static_cast<std::size_t>(iy) * columns + static_cast<std::size_t>(ix));
            const auto c = static_cast<std::uint32_t>(static_cast<std::size_t>(iy + 1) * columns + static_cast<std::size_t>(ix));
            const auto d = static_cast<std::uint32_t>(static_cast<std::size_t>(iy + 1) * columns + static_cast<std::size_t>(ix) + 1);
            if (iy != 0) {
                out.indices.insert(out.indices.end(), {a, b, d});
            }
            if (iy != rings - 1) {
                out.indices.insert(out.indices.end(), {b, c, d});
            }
        }
    }
    std::vector<Vec3> normals(positions.size());
    for (std::size_t t = 0; t + 2 < out.indices.size(); t += 3) {
        const Vec3& p0 = positions[out.indices[t]];
        const Vec3& p1 = positions[out.indices[t + 1]];
        const Vec3& p2 = positions[out.indices[t + 2]];
        const Vec3 face = cross(sub(p1, p0), sub(p2, p0));
        for (std::size_t k = 0; k < 3; ++k) {
            normals[out.indices[t + k]] = add(normals[out.indices[t + k]], face);
        }
    }
    out.vertices.reserve(positions.size() * 10);
    for (std::size_t i = 0; i < positions.size(); ++i) {
        // The seam's duplicated vertices and the poles gather no faces of their own;
        // outwards is the right normal for them on a sphere.
        const Vec3 n = length(normals[i]) > 1e-8F ? normalize(normals[i]) : normalize(positions[i]);
        out.vertices.insert(out.vertices.end(), {positions[i].x, positions[i].y, positions[i].z, n.x, n.y,
                                                 n.z, tints[i].r, tints[i].g, tints[i].b, 1.0F});
    }
    return out;
}

Mesh icosphere(int subdivisions) {
    subdivisions = std::clamp(subdivisions, 0, 5);
    const float t = (1.0F + std::sqrt(5.0F)) / 2.0F;
    Mesh mesh;
    mesh.vertices = {{-1, t, 0}, {1, t, 0},  {-1, -t, 0}, {1, -t, 0}, {0, -1, t}, {0, 1, t},
                     {0, -1, -t}, {0, 1, -t}, {t, 0, -1},  {t, 0, 1},  {-t, 0, -1}, {-t, 0, 1}};
    for (Vec3& v : mesh.vertices) {
        v = normalize(v);
    }
    mesh.indices = {0, 11, 5, 0, 5,  1, 0, 1, 7, 0, 7,  10, 0, 10, 11, 1, 5, 9, 5, 11,
                    4, 11, 10, 2, 10, 7, 6, 7, 1, 8, 3, 9,  4, 3,  4,  2, 3, 2, 6, 3,
                    6, 8,  3, 8,  9, 4, 9, 5, 2, 4, 11, 6, 2,  10, 8,  6, 7, 9, 8, 1};
    for (int level = 0; level < subdivisions; ++level) {
        std::map<std::pair<std::uint32_t, std::uint32_t>, std::uint32_t> midpoints;
        const auto midpoint = [&](std::uint32_t a, std::uint32_t b) {
            const auto key = std::minmax(a, b);
            if (const auto found = midpoints.find(key); found != midpoints.end()) {
                return found->second;
            }
            const auto index = static_cast<std::uint32_t>(mesh.vertices.size());
            mesh.vertices.push_back(normalize(scale(add(mesh.vertices[a], mesh.vertices[b]), 0.5F)));
            midpoints.emplace(key, index);
            return index;
        };
        TriangleIndices next;
        next.reserve(mesh.indices.size() * 4);
        for (std::size_t i = 0; i + 2 < mesh.indices.size(); i += 3) {
            const std::uint32_t a = mesh.indices[i];
            const std::uint32_t b = mesh.indices[i + 1];
            const std::uint32_t c = mesh.indices[i + 2];
            const std::uint32_t ab = midpoint(a, b);
            const std::uint32_t bc = midpoint(b, c);
            const std::uint32_t ca = midpoint(c, a);
            next.insert(next.end(), {a, ab, ca, b, bc, ab, c, ca, bc, ab, bc, ca});
        }
        mesh.indices = std::move(next);
    }
    mesh.normals = mesh.vertices;
    return mesh;
}

Mesh torus(float major, float minor, int segments, int sides) {
    segments = std::clamp(segments, 3, 256);
    sides = std::clamp(sides, 3, 64);
    Mesh mesh;
    for (int j = 0; j <= sides; ++j) {
        for (int i = 0; i <= segments; ++i) {
            const float u = static_cast<float>(i) / static_cast<float>(segments) * kTau;
            const float v = static_cast<float>(j) / static_cast<float>(sides) * kTau;
            const Vec3 centre{major * std::cos(u), major * std::sin(u), 0.0F};
            const Vec3 point{(major + minor * std::cos(v)) * std::cos(u),
                             (major + minor * std::cos(v)) * std::sin(u), minor * std::sin(v)};
            mesh.vertices.push_back(point);
            mesh.normals.push_back(normalize(sub(point, centre)));
        }
    }
    const auto row = static_cast<std::uint32_t>(segments + 1);
    for (int j = 1; j <= sides; ++j) {
        for (int i = 1; i <= segments; ++i) {
            const auto uj = static_cast<std::uint32_t>(j);
            const auto ui = static_cast<std::uint32_t>(i);
            const std::uint32_t a = row * uj + ui - 1;
            const std::uint32_t b = row * (uj - 1) + ui - 1;
            const std::uint32_t c = row * (uj - 1) + ui;
            const std::uint32_t d = row * uj + ui;
            mesh.indices.insert(mesh.indices.end(), {a, b, d, b, c, d});
        }
    }
    return mesh;
}

Guides guides(float field_radius) {
    Guides out;
    for (int e = 0; e < 3; ++e) {
        const float rx = e == 1 ? 0.84F : (e == 2 ? -0.53F : 0.1F);
        const float rz = static_cast<float>(e) * 0.39F;
        const auto turn = [rx, rz](const Vec3& p) {
            // The design's Euler XYZ: about Z first, then X.
            const Vec3 z{p.x * std::cos(rz) - p.y * std::sin(rz), p.x * std::sin(rz) + p.y * std::cos(rz), p.z};
            return Vec3{z.x, z.y * std::cos(rx) - z.z * std::sin(rx), z.y * std::sin(rx) + z.z * std::cos(rx)};
        };
        for (int a = 0; a < 11; ++a) {
            if ((a + e * 3) % 5 == 0) {
                continue;
            }
            const float start = static_cast<float>(a) / 11.0F * kTau + static_cast<float>(e) * 0.24F;
            Vec3 previous{};
            for (int h = 0; h <= 12; ++h) {
                const float c = start + static_cast<float>(h) / 12.0F * kTau / 11.0F * 0.78F;
                const float l = c * 3.0F + static_cast<float>(e) * 2.0F;
                const float reach = field_radius * (0.92F + static_cast<float>(e) * 0.07F);
                const Vec3 point = turn(Vec3{std::cos(c) * reach, std::sin(l) * field_radius * 0.08F,
                                             std::sin(c) * reach});
                if (h > 0) {
                    out.points.push_back(previous);
                    out.points.push_back(point);
                    out.family.push_back(e);
                }
                previous = point;
            }
        }
    }
    return out;
}

}  // namespace archie::visual::lumen
