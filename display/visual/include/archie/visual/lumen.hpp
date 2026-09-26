#pragma once

// Lumenfield, the orb's drawing: what moves, what is decoration and what shape it has.
//
// Where every node sits is decided by the TFOG model in Rust (`archie_orb::lumen`), which
// serves a composed scene of anchors. This file is the renderer's half, the part of the
// design that runs every frame or turns data into triangles:
//
// - the spring that pulls each orb to its anchor, and the push away from the pointer;
// - the spread of a selected module's members, so the eye can separate them;
// - the curves relations are drawn along;
// - the satellites around a module and the ghost particles around the field, which are
//   density only: never selectable, never counted, never an entity;
// - the membrane, a sphere swollen where the orbs gather and tinted by the nearest one;
// - the meshes the orbs, the selection ring and the orbital guides are made of.
//
// Everything here is deterministic. A seed decides what the design took from a random
// source, so the same scene always draws the same picture.

#include <cstddef>
#include <cstdint>
#include <optional>
#include <string_view>
#include <vector>

#include "archie/visual/camera.hpp"
#include "archie/visual/mesh.hpp"

namespace archie::visual::lumen {

struct Rgb {
    float r = 0.0F;
    float g = 0.0F;
    float b = 0.0F;

    friend bool operator==(const Rgb& a, const Rgb& b) noexcept {
        return a.r == b.r && a.g == b.g && a.b == b.b;
    }
};

/// `#rrggbb`, as the design's colour settings are written. Anything else is `nullopt`.
[[nodiscard]] std::optional<Rgb> parse_hex(std::string_view text) noexcept;

/// sRGB to linear light, per channel, as a shader expects its inputs.
[[nodiscard]] Rgb to_linear(const Rgb& srgb) noexcept;

[[nodiscard]] Rgb mix(const Rgb& a, const Rgb& b, float t) noexcept;

/// A number in [0, 1] from an integer, the same for the same integer everywhere.
[[nodiscard]] float hash01(std::uint32_t n) noexcept;

/// One orb in the physics: where the model put it, where it is, how it is moving.
struct Body {
    Vec3 anchor;
    Vec3 position;
    Vec3 velocity;
    /// The root never moves and is never pushed.
    bool fixed = false;
};

/// The design's motion controls.
struct Motion {
    /// Push away from the pointer ("Respuesta al puntero").
    float reaction = 145.0F;
    /// How far that push reaches ("Radio de interacción").
    float reach = 46.0F;
    /// Pull back to the anchor ("Retorno al campo").
    float spring = 25.0F;
    /// Velocity kept per 60th of a second ("Amortiguación").
    float damping = 0.85F;
    /// Nothing leaves this sphere.
    float radius = 200.0F;
};

/// Advances every body by `dt` seconds (clamped to 50 ms). Returns whether anything is
/// still moving, so a caller can stop stepping a field at rest.
bool step(std::vector<Body>& bodies, const std::optional<Vec3>& pointer, float dt,
          const Motion& motion);

/// Anchors with a selected branch's members pushed out from it by `spread` (0..0.6),
/// never beyond 1.05 times the field radius. `parent[i]` is i's parent index, or -1.
[[nodiscard]] std::vector<Vec3> focus_region(const std::vector<Vec3>& anchors,
                                             const std::vector<int>& parent, int selected,
                                             float spread, float field_radius);

/// A point along the arc relations are drawn on: bowed sideways and a little upwards.
[[nodiscard]] Vec3 curve(const Vec3& a, const Vec3& b, float phase, float weight = 0.13F) noexcept;

/// Satellites a module of `members` entities gets: more members, more density, bounded.
[[nodiscard]] int satellite_budget(std::size_t members) noexcept;

/// Where those satellites sit around their module's centre.
[[nodiscard]] std::vector<Vec3> satellite_offsets(int count, float radius, std::uint32_t seed);

/// How far satellites spread around a module of `members` entities.
[[nodiscard]] float satellite_radius(std::size_t members, float field_radius) noexcept;

/// Ghost particles gathered around the modules in proportion to their size; around the
/// whole field if there are no modules.
[[nodiscard]] std::vector<Vec3> ghosts(const std::vector<Vec3>& centres,
                                       const std::vector<std::size_t>& members, int count,
                                       float field_radius);

/// Which tree links to draw as the faint backbone: 70% of the budget on links to
/// branches, the rest on links to leaves, both sampled evenly. Returns link indices.
[[nodiscard]] std::vector<std::size_t> backbone(const std::vector<bool>& target_is_branch,
                                                std::size_t limit);

/// The nodes whose link to their parent is lit when `selected` is hovered or chosen: its
/// ancestors, its children, up to 24 siblings, then its grandchildren.
[[nodiscard]] std::vector<std::size_t> neighbourhood(const std::vector<int>& parent,
                                                     int selected, std::size_t limit);

/// Vertex layout of the membrane: position, normal, colour (rgba), 10 floats a vertex.
struct Membrane {
    std::vector<float> vertices;
    TriangleIndices indices;
    float radius = 0.0F;
};

/// The membrane around `nodes`. Its radius covers 96% of them (the whole field in the
/// legacy layout); it swells where they gather and takes the colour of the nearest one,
/// warmed towards `warm`.
[[nodiscard]] Membrane membrane(const std::vector<Vec3>& nodes, const std::vector<Rgb>& colours,
                                const Rgb& warm, float field_radius, bool legacy,
                                float shell_scale, int rings = 40, int segments = 56);

/// A unit sphere made by subdividing an icosahedron `subdivisions` times.
[[nodiscard]] Mesh icosphere(int subdivisions);

/// A ring in the XY plane facing +Z, for the selection marker.
[[nodiscard]] Mesh torus(float major, float minor, int segments, int sides);

/// The orbital guides: arcs around the field, as line-segment pairs, three families.
struct Guides {
    std::vector<Vec3> points;
    /// Which family each segment belongs to (0 warm, 1 cool, 2 warm), one per pair.
    std::vector<int> family;
};
[[nodiscard]] Guides guides(float field_radius);

}  // namespace archie::visual::lumen
