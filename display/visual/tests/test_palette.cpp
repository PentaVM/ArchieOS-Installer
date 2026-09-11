// Colour, and the rule that it is never the only thing carrying a fact.

#include "archie/visual/palette.hpp"

#include "harness.hpp"

using namespace archie::visual;

namespace {

constexpr Palette kPalette{};

NodeColour paint(int severity, Freshness freshness = Freshness::Live,
                 NodePresence presence = NodePresence::Present,
                 RenderTier tier = RenderTier::Normal, bool authoritative = true) {
    return colour_for(severity, freshness, presence, authoritative, tier, kPalette);
}

void the_ordering_survives_without_colour_vision() {
    CASE("luminance rises with severity, so grey is enough to read the scale");

    float previous = -1.0F;
    bool monotone = true;
    for (int severity = 0; severity <= 3; ++severity) {
        const float value = luminance(paint(severity).base);
        if (value <= previous) {
            monotone = false;
        }
        previous = value;
    }

    CHECK_BECAUSE(monotone,
                  "a display that says 'red means trouble' has told roughly one man in twelve "
                  "nothing at all; encoding severity in luminance as well means the ordering "
                  "survives greyscale, a bad photograph, and colour blindness");

    // And the gaps are large enough to see, not merely different in the seventh decimal.
    for (int severity = 1; severity <= 3; ++severity) {
        const float step =
            luminance(paint(severity).base) - luminance(paint(severity - 1).base);
        CHECK_BECAUSE(step > 0.03F,
                      "two steps that differ by less than a few percent of luminance are the "
                      "same shade to anyone not comparing them side by side");
    }
}

void severity_beyond_the_scale_clamps_rather_than_wrapping() {
    CASE("an unknown severity never comes out looking calm");

    CHECK(paint(99).base == paint(3).base);
    CHECK_BECAUSE(luminance(paint(99).base) > luminance(paint(0).base),
                  "the exact failure mode of every colour table that used a modulo: a severity "
                  "the display has never seen wrapping round to the quiet end of the palette");

    // And a negative severity, which can only be a payload defect, is not calmer than calm.
    CHECK(paint(-5).base == paint(0).base);
}

void age_never_changes_what_a_node_says() {
    CASE("a stale critical node is still a critical node");

    const NodeColour live = paint(3, Freshness::Live);
    const NodeColour expired = paint(3, Freshness::Expired);

    CHECK_BECAUSE(live.base == expired.base,
                  "fading a stale critical node toward the calm end would turn 'we have lost "
                  "contact with something that was on fire' into 'everything is fine', which "
                  "is the most dangerous sentence a display can say");
    CHECK(expired.alpha < live.alpha);
}

void opacity_follows_confidence() {
    CASE("how strongly a node is drawn is how much it can be vouched for");

    CHECK_EQ(paint(1, Freshness::Live).alpha, confidence_of(Freshness::Live));
    CHECK_EQ(paint(1, Freshness::Lagging).alpha, confidence_of(Freshness::Lagging));
    CHECK_EQ(paint(1, Freshness::Stale).alpha, confidence_of(Freshness::Stale));
    CHECK_EQ(paint(1, Freshness::Expired).alpha, confidence_of(Freshness::Expired));

    CHECK_BECAUSE(paint(1, Freshness::Expired).alpha > 0.0F,
                  "a source that has gone dark must stay visible, or the operator cannot tell "
                  "it from a healthy node with nothing to report");
}

void a_departing_node_is_dimmer_than_a_present_one() {
    CASE("leaving is visible without changing what the node is");

    const NodeColour present = paint(2, Freshness::Live, NodePresence::Present);
    const NodeColour departing = paint(2, Freshness::Live, NodePresence::Departing);

    CHECK(departing.alpha < present.alpha);
    CHECK(departing.base == present.base);

    // Entering is not dimmed: an arriving node should be at full strength, with the
    // arrival carried by motion rather than by looking like it is already leaving.
    CHECK_EQ(paint(2, Freshness::Live, NodePresence::Entering).alpha, present.alpha);
}

void glow_rises_with_severity() {
    CASE("the alarm channel in a 3D scene");

    CHECK(paint(3).emissive > paint(2).emissive);
    CHECK(paint(2).emissive > paint(1).emissive);
    CHECK(paint(1).emissive > paint(0).emissive);
    CHECK_EQ(paint(0).emissive, kPalette.emissive_floor);
    CHECK_EQ(paint(3).emissive, kPalette.emissive_ceiling);
}

void an_austere_tier_gives_up_its_glow_first() {
    CASE("the most expensive channel is the most expendable");

    CHECK(paint(3, Freshness::Live, NodePresence::Present, RenderTier::Ultra).emissive > 0.0F);
    CHECK(paint(3, Freshness::Live, NodePresence::Present, RenderTier::Normal).emissive > 0.0F);
    CHECK_EQ(paint(3, Freshness::Live, NodePresence::Present, RenderTier::Conserve).emissive,
             0.0F);
    CHECK_EQ(paint(3, Freshness::Live, NodePresence::Present, RenderTier::Headless).emissive,
             0.0F);

    CHECK_BECAUSE(paint(3, Freshness::Live, NodePresence::Present, RenderTier::Conserve).base ==
                      paint(3, Freshness::Live, NodePresence::Present, RenderTier::Ultra).base,
                  "hue and luminance already carry the severity on their own, so dropping the "
                  "glow costs the scene nothing an operator needs");
}

void stale_data_does_not_glow() {
    CASE("emissive reads as activity, and activity is a claim about now");

    CHECK(paint(3, Freshness::Lagging).emissive > 0.0F);
    CHECK_BECAUSE(paint(3, Freshness::Stale).emissive == 0.0F,
                  "a node pulsing with light from a source that stopped reporting four minutes "
                  "ago is the same lie as a node that is still moving");
    CHECK_EQ(paint(3, Freshness::Expired).emissive, 0.0F);

    // The colour itself is untouched: it still says critical.
    CHECK(paint(3, Freshness::Expired).base == paint(3, Freshness::Live).base);
}

void being_second_hand_is_not_being_wrong() {
    CASE("authority reduces how strongly a node is drawn, and nothing else");

    const NodeColour firsthand = paint(3, Freshness::Live, NodePresence::Present,
                                       RenderTier::Normal, true);
    const NodeColour secondhand = paint(3, Freshness::Live, NodePresence::Present,
                                        RenderTier::Normal, false);

    CHECK(secondhand.alpha < firsthand.alpha);
    CHECK_BECAUSE(secondhand.base == firsthand.base,
                  "an unauthoritative report of something critical is still a report of "
                  "something critical, and fading it toward calm would be the same lie "
                  "staleness is not allowed to tell");

    CHECK_BECAUSE(secondhand.alpha > paint(3, Freshness::Expired).alpha,
                  "unauthoritative is not wrong, it is second-hand; drawing it as faintly as a "
                  "dead source tells the operator to ignore what may be the only report they "
                  "have");
}

void age_and_authority_are_separate_channels() {
    CASE("a current second-hand reading is not the same as a stale first-hand one");

    const NodeColour stale_authority = paint(2, Freshness::Stale, NodePresence::Present,
                                             RenderTier::Normal, true);
    const NodeColour live_hearsay = paint(2, Freshness::Live, NodePresence::Present,
                                          RenderTier::Normal, false);

    CHECK_BECAUSE(live_hearsay.alpha != stale_authority.alpha,
                  "collapsing the two into one number tells the operator the wrong thing about "
                  "both: one is old and trustworthy, the other current and second-hand");
    CHECK(live_hearsay.alpha > stale_authority.alpha);

    // They compound, because both reasons to doubt apply at once.
    const NodeColour both = paint(2, Freshness::Stale, NodePresence::Present, RenderTier::Normal,
                                  false);
    CHECK(both.alpha < stale_authority.alpha && both.alpha < live_hearsay.alpha);
    CHECK(both.alpha > 0.0F);
}

void every_channel_stays_in_range() {
    CASE("nothing leaves a value a renderer has to guess about");

    for (int severity = -2; severity <= 8; ++severity) {
        for (const Freshness freshness : {Freshness::Live, Freshness::Lagging, Freshness::Stale,
                                          Freshness::Expired}) {
            for (const NodePresence presence : {NodePresence::Entering, NodePresence::Present,
                                                NodePresence::Departing}) {
                for (const RenderTier tier : {RenderTier::Ultra, RenderTier::Normal,
                                              RenderTier::Conserve, RenderTier::Headless}) {
                  for (const bool authoritative : {true, false}) {
                    const NodeColour colour =
                        colour_for(severity, freshness, presence, authoritative, tier, kPalette);
                    CHECK(colour.base.r >= 0.0F && colour.base.r <= 1.0F);
                    CHECK(colour.base.g >= 0.0F && colour.base.g <= 1.0F);
                    CHECK(colour.base.b >= 0.0F && colour.base.b <= 1.0F);
                    CHECK(colour.alpha > 0.0F && colour.alpha <= 1.0F);
                    CHECK(colour.emissive >= 0.0F && colour.emissive <= 1.0F);
                  }
                }
            }
        }
    }
}

}  // namespace

int main() {
    the_ordering_survives_without_colour_vision();
    severity_beyond_the_scale_clamps_rather_than_wrapping();
    age_never_changes_what_a_node_says();
    opacity_follows_confidence();
    a_departing_node_is_dimmer_than_a_present_one();
    glow_rises_with_severity();
    an_austere_tier_gives_up_its_glow_first();
    stale_data_does_not_glow();
    being_second_hand_is_not_being_wrong();
    age_and_authority_are_separate_channels();
    every_channel_stays_in_range();
    return archie::visual::test::report();
}
