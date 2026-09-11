#include "archie/visual/palette.hpp"

namespace archie::visual {

NodeColour colour_for(int severity, Freshness freshness, NodePresence presence,
                      bool authoritative, RenderTier tier, const Palette& palette) noexcept {
    const int top = static_cast<int>(palette.severity.size()) - 1;
    // Clamped at both ends. A negative severity is a payload defect and must not read as
    // calmer than calm; one above the scale must not wrap round to it.
    const int level = severity < 0 ? 0 : (severity > top ? top : severity);

    NodeColour colour{};
    colour.base = palette.severity[static_cast<std::size_t>(level)];

    // Freshness touches opacity only. Fading a stale critical node toward the calm end
    // of the palette would turn "we have lost contact with something that was on fire"
    // into "everything is fine".
    colour.alpha = confidence_of(freshness);

    switch (presence) {
        case NodePresence::Departing:
            colour.alpha *= palette.departing_alpha;
            break;
        case NodePresence::Entering:
        case NodePresence::Present:
            break;
    }

    // A separate question from age, and it touches opacity for the same reason: how
    // strongly a thing is drawn is how much the display can vouch for it, and being
    // second-hand is one of the ways it can vouch for it less. Like freshness, it does
    // not touch hue -- an unauthoritative report of something critical is still a report
    // of something critical.
    if (!authoritative) {
        colour.alpha *= palette.unauthoritative_alpha;
    }

    const float span = static_cast<float>(top);
    const float t = span > 0.0F ? static_cast<float>(level) / span : 0.0F;
    colour.emissive =
        palette.emissive_floor + (palette.emissive_ceiling - palette.emissive_floor) * t;

    // Glow is the first thing an austere tier gives up: it is the most expensive part of
    // a node to draw and the least necessary, since hue and luminance already carry the
    // severity on their own.
    switch (tier) {
        case RenderTier::Ultra:
        case RenderTier::Normal:
            break;
        case RenderTier::Conserve:
        case RenderTier::Headless:
            colour.emissive = 0.0F;
            break;
    }

    // Stale data does not glow either. Emissive reads as activity, and activity is a
    // claim about now.
    switch (freshness) {
        case Freshness::Live:
        case Freshness::Lagging:
            break;
        case Freshness::Stale:
        case Freshness::Expired:
            colour.emissive = 0.0F;
            break;
    }

    return colour;
}

}  // namespace archie::visual
