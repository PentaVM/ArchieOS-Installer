#pragma once

// Colour, and the rule that it must never be the only thing carrying a fact.
//
// A status display that says "red means trouble" has told roughly one man in twelve
// nothing at all. So severity is encoded twice over: hue moves from cool to warm, and
// *luminance rises monotonically with it*, which means the ordering survives being
// printed in grey, photographed badly, or seen by someone who does not separate red
// from green. That property is checkable, and it is checked.
//
// Freshness is a separate channel and is not allowed to touch hue. A stale critical
// node is still a critical node: fading it toward calm would turn "we have lost contact
// with something that was on fire" into "everything is fine", which is the most
// dangerous sentence a display can say. Age reduces confidence — alpha and glow — and
// leaves the colour saying what it always said.
//
// Nothing here is decorative. Every value in the returned colour comes from a fact the
// node actually has.

#include <array>

#include "archie/visual/budget.hpp"
#include "archie/visual/freshness.hpp"
#include "archie/visual/graph.hpp"

namespace archie::visual {

struct Rgb {
    float r = 0.0F;
    float g = 0.0F;
    float b = 0.0F;

    friend bool operator==(const Rgb& a, const Rgb& c) noexcept {
        return a.r == c.r && a.g == c.g && a.b == c.b;
    }
};

/// Relative luminance, the standard coefficients.
///
/// This is the number the accessibility rule is stated in: if it rises with severity,
/// the ordering is legible without colour vision at all.
[[nodiscard]] constexpr float luminance(const Rgb& colour) noexcept {
    return 0.2126F * colour.r + 0.7152F * colour.g + 0.0722F * colour.b;
}

/// How a node is painted.
struct NodeColour {
    Rgb base{};
    /// Opacity, from freshness. Never zero: a source that has gone dark must stay
    /// visible, or the operator cannot tell it from a healthy node with nothing to say.
    float alpha = 1.0F;
    /// Self-illumination. The real alarm channel in a 3D scene, and the first thing a
    /// conserving tier gives up.
    float emissive = 0.0F;
};

struct Palette {
    /// Severity 0 through 3. Cool and dim to warm and bright, with luminance rising at
    /// every step so the ordering does not depend on hue.
    std::array<Rgb, 4> severity{
        Rgb{0.20F, 0.30F, 0.40F},  // quiet: recedes into the scene
        Rgb{0.55F, 0.50F, 0.25F},  // noticed
        Rgb{0.90F, 0.55F, 0.20F},  // actionable
        Rgb{1.00F, 0.55F, 0.50F},  // broken: the brightest thing on screen
    };

    /// Glow at severity 0 and at the top of the scale; interpolated between.
    float emissive_floor = 0.0F;
    float emissive_ceiling = 0.9F;

    /// A departing node is drawn at this fraction of its opacity, on top of whatever
    /// freshness already cost it.
    float departing_alpha = 0.55F;

    /// A node whose source was not the authority for it, likewise.
    ///
    /// Deliberately a gentler reduction than staleness. Unauthoritative is not wrong; it
    /// is second-hand, and drawing it as faintly as a dead source would tell the
    /// operator to ignore something that may be the only report they have.
    float unauthoritative_alpha = 0.78F;
};

/// Paint one node.
///
/// `severity` beyond the top of the scale clamps rather than wrapping. Wrapping would
/// make a severity the display has never seen come out looking calm, which is the exact
/// failure mode of every colour table that used a modulo.
[[nodiscard]] NodeColour colour_for(int severity, Freshness freshness, NodePresence presence,
                                    bool authoritative, RenderTier tier,
                                    const Palette& palette) noexcept;

}  // namespace archie::visual
