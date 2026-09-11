// World observations: a place, a grading, and a question about who said so.

#include <string>

#include "archie/visual/ingest.hpp"
#include "harness.hpp"

using namespace archie::visual;

namespace {

Value position(double latitude, double longitude) {
    return Value::object({member("latitude", Value{latitude}),
                          member("longitude", Value{longitude}),
                          member("depth_km", Value{10.0})});
}

Value quake(const std::string& id, const char* grade, double magnitude, bool tsunami = false,
            bool authoritative = true) {
    return Value::object({
        member("id", Value{id}),
        member("occurred_at", Value{0}),
        member("position", position(35.0, 139.0)),
        member("summary", Value{"M " + std::to_string(magnitude) + " near somewhere"}),
        member("details", Value::object({
                              member("kind", Value{"earthquake"}),
                              member("magnitude", Value{magnitude}),
                              member("severity", Value{grade}),
                              member("tsunami_warning", Value{tsunami}),
                          })),
        member("provenance", Value::object({
                                 member("authority", Value{"USGS"}),
                                 member("dataset", Value{"all_hour"}),
                                 member("authoritative", Value{authoritative}),
                             })),
    });
}

Value world_of(std::vector<Value> observations) {
    return Value::object({member("schema_version", Value{1}),
                          member("observations", Value::array(std::move(observations)))});
}

void a_well_formed_observation_becomes_a_placed_node() {
    CASE("end to end: a quake arrives with a place, a size and a grading");

    IngestReport report{};
    const std::vector<NodeSpec> nodes =
        ingest_world(world_of({quake("us7000abcd", "strong", 6.4)}), Freshness::Live, report);

    CHECK_EQ(nodes.size(), 1U);
    CHECK(report.clean());
    CHECK_EQ(nodes[0].kind, NodeKind::Observation);
    CHECK_EQ(shape_of(nodes[0].kind), NodeShape::Sphere);
    CHECK_EQ(nodes[0].severity, 3);
    CHECK(nodes[0].magnitude > 6.3F && nodes[0].magnitude < 6.5F);
    CHECK_BECAUSE(nodes[0].location.has_value(),
                  "an observation without a place cannot be plotted, and the whole point of "
                  "the Watcher view is that these are somewhere");
    CHECK(nodes[0].location->latitude == 35.0 && nodes[0].location->longitude == 139.0);
    CHECK(nodes[0].authoritative);
}

void the_grading_follows_the_providers_own_scale() {
    CASE("every earthquake severity label this build knows");

    bool recognised = false;
    CHECK_EQ(severity_for_earthquake("trace", recognised), 0);
    CHECK(recognised);
    CHECK_EQ(severity_for_earthquake("minor", recognised), 1);
    CHECK_EQ(severity_for_earthquake("moderate", recognised), 2);
    CHECK_EQ(severity_for_earthquake("strong", recognised), 3);
    CHECK_BECAUSE(severity_for_earthquake("major", recognised) == 4,
                  "past the top of the palette on purpose: a Major event is drawn in the same "
                  "colour as a Strong one, and outranks it for the render budget, which is "
                  "what decides whether it is drawn at all on a busy globe");
    CHECK(recognised);

    CHECK(severity_for_earthquake("cataclysmic", recognised) == 1);
    CHECK(!recognised);
}

void a_tsunami_warning_outranks_the_magnitude() {
    CASE("a fact about consequence, not about the event");

    IngestReport report{};
    const std::vector<NodeSpec> nodes = ingest_world(
        world_of({quake("quiet", "moderate", 5.0, false), quake("warned", "moderate", 5.0, true)}),
        Freshness::Live, report);

    CHECK_EQ(nodes.size(), 2U);
    CHECK_EQ(nodes[0].severity, 2);
    CHECK_BECAUSE(nodes[1].severity == 3,
                  "a moderate quake with a warning attached is what an operator needs to see "
                  "first, and grading it by magnitude alone would put it below a larger event "
                  "that threatens nobody");
}

void silence_about_authority_is_not_authority() {
    CASE("a payload that does not say its source was entitled has not said it was");

    IngestReport report{};

    // No provenance block at all.
    Value bare = quake("bare", "minor", 3.0);
    const std::vector<Member>* fields = bare.as_object();
    std::vector<Member> without;
    for (const Member& field : *fields) {
        if (field.key != "provenance") {
            without.push_back(Member{field.key, field.value});
        }
    }
    const std::vector<NodeSpec> nodes =
        ingest_world(world_of({Value::object(std::move(without))}), Freshness::Live, report);

    CHECK_EQ(nodes.size(), 1U);
    CHECK_BECAUSE(!nodes[0].authoritative,
                  "defaulting to true is the optimistic reading of silence, and this field "
                  "exists precisely to be sceptical about where a claim came from");
    CHECK_BECAUSE(report.defects.empty(),
                  "a provider that does not report authority is not a malformed payload");
}

void a_declared_fallback_source_stays_unauthoritative() {
    CASE("the flag is honoured in both directions");

    IngestReport report{};
    const std::vector<NodeSpec> nodes = ingest_world(
        world_of({quake("cached", "minor", 3.0, false, false)}), Freshness::Live, report);

    CHECK_EQ(nodes.size(), 1U);
    CHECK_BECAUSE(!nodes[0].authoritative,
                  "a last-known-good batch from a fallback source is current and "
                  "unauthoritative at the same time; a display that collapses those two into "
                  "one number tells the operator the wrong thing about both");
    CHECK_EQ(nodes[0].freshness, Freshness::Live);
}

void an_observation_with_no_place_is_dropped() {
    CASE("a marker somewhere arbitrary is a claim about where something happened");

    IngestReport missing{};
    const std::vector<NodeSpec> no_position = ingest_world(
        world_of({Value::object({member("id", Value{"nowhere"}),
                                 member("details", Value::object({member("kind",
                                                                         Value{"earthquake"})}))})}),
        Freshness::Live, missing);
    CHECK(no_position.empty());
    CHECK_EQ(missing.dropped, 1);
    CHECK_BECAUSE(missing.count_of(DefectKind::MissingField) >= 1,
                  "unlike a missing grade there is no reduced form worth drawing: inventing a "
                  "coordinate is worse than omitting the record and saying so");

    IngestReport wrong{};
    const std::vector<NodeSpec> bad_position =
        ingest_world(world_of({Value::object({
                         member("id", Value{"bad"}),
                         member("position", Value::object({member("latitude", Value{"north"}),
                                                           member("longitude", Value{139.0})})),
                     })}),
                     Freshness::Live, wrong);
    CHECK(bad_position.empty());
    CHECK_EQ(wrong.dropped, 1);
    CHECK_EQ(wrong.count_of(DefectKind::WrongType), 1);
}

void an_unknown_grading_is_placed_and_marked() {
    CASE("a scale this build has never seen still goes on the globe");

    IngestReport report{};
    const std::vector<NodeSpec> nodes =
        ingest_world(world_of({quake("future", "cataclysmic", 9.9)}), Freshness::Live, report);

    CHECK_EQ(nodes.size(), 1U);
    CHECK_EQ(nodes[0].severity, 1);
    CHECK_EQ(report.count_of(DefectKind::UnknownEnum), 1);
    CHECK_EQ(report.dropped, 0);
}

void an_unknown_observation_kind_still_appears() {
    CASE("a display predating a provider must not leave a hole in the globe");

    IngestReport report{};
    const std::vector<NodeSpec> nodes = ingest_world(
        world_of({Value::object({
            member("id", Value{"storm.1"}),
            member("position", position(-12.0, -70.0)),
            member("summary", Value{"Cyclone"}),
            member("details", Value::object({member("kind", Value{"cyclone"}),
                                             member("magnitude", Value{3.0})})),
        })}),
        Freshness::Live, report);

    CHECK_EQ(nodes.size(), 1U);
    CHECK_BECAUSE(nodes[0].location.has_value(),
                  "we know where it is even though we do not know what it is, and where is "
                  "the more useful of the two");
    CHECK_EQ(nodes[0].severity, 1);
    CHECK_EQ(report.count_of(DefectKind::UnknownEnum), 1);
    CHECK(nodes[0].magnitude > 2.9F && nodes[0].magnitude < 3.1F);
}

void a_missing_summary_falls_back_to_the_identifier() {
    CASE("a marker with no label is still worth plotting");

    IngestReport report{};
    const std::vector<NodeSpec> nodes = ingest_world(
        world_of({Value::object({
            member("id", Value{"us7000zzzz"}),
            member("position", position(0.0, 0.0)),
            member("details", Value::object({member("kind", Value{"earthquake"}),
                                             member("severity", Value{"minor"})})),
        })}),
        Freshness::Live, report);

    CHECK_EQ(nodes.size(), 1U);
    CHECK_BECAUSE(nodes[0].label == std::string{"us7000zzzz"},
                  "it only has to be identifiable when the operator clicks it");
}

void one_bad_record_costs_one_record() {
    CASE("the rest of the globe still arrives");

    IngestReport report{};
    const std::vector<NodeSpec> nodes = ingest_world(
        world_of({
            quake("good.1", "minor", 3.0),
            Value{"not a record"},
            Value::object({member("id", Value{"no.place"})}),
            quake("good.2", "strong", 6.1),
        }),
        Freshness::Live, report);

    CHECK_EQ(nodes.size(), 2U);
    CHECK_EQ(report.dropped, 2);
    CHECK_EQ(report.accepted, 2);
    CHECK_EQ(nodes[0].id, std::string{"good.1"});
    CHECK_EQ(nodes[1].id, std::string{"good.2"});
}

void a_newer_world_schema_is_read_anyway() {
    CASE("the fields this build understands are still worth having");

    IngestReport report{};
    const std::vector<NodeSpec> nodes = ingest_world(
        Value::object({member("schema_version", Value{47}),
                       member("observations", Value::array({quake("q", "moderate", 5.0)}))}),
        Freshness::Live, report);

    CHECK(report.newer_schema);
    CHECK_EQ(nodes.size(), 1U);
    CHECK_EQ(nodes[0].severity, 2);
}

void a_payload_of_the_wrong_shape_yields_nothing_and_says_why() {
    CASE("garbage in, nothing out, and a reason");

    IngestReport not_object{};
    CHECK(ingest_world(Value{42}, Freshness::Live, not_object).empty());
    CHECK_EQ(not_object.count_of(DefectKind::NotAnObject), 1);

    IngestReport no_list{};
    CHECK(ingest_world(Value::object({member("providers", Value::array({}))}), Freshness::Live,
                       no_list)
              .empty());
    CHECK_EQ(no_list.count_of(DefectKind::MissingField), 1);

    IngestReport quiet{};
    CHECK_BECAUSE(ingest_world(world_of({}), Freshness::Live, quiet).empty() && quiet.clean(),
                  "an hour with no earthquakes is a legitimate state of the world, not a "
                  "defect in the payload");
}

void freshness_travels_with_the_observations() {
    CASE("a globe drawn from a stale batch says so on every marker");

    IngestReport report{};
    const std::vector<NodeSpec> nodes =
        ingest_world(world_of({quake("a", "minor", 3.0), quake("b", "strong", 6.0)}),
                     Freshness::Stale, report);

    CHECK_EQ(nodes.size(), 2U);
    for (const NodeSpec& node : nodes) {
        CHECK_EQ(node.freshness, Freshness::Stale);
    }
}

void a_field_from_a_newer_provider_is_counted_not_refused() {
    CASE("forward compatibility on the globe as well");

    IngestReport report{};
    const std::vector<NodeSpec> nodes = ingest_world(
        world_of({Value::object({
            member("id", Value{"q"}),
            member("position", position(1.0, 2.0)),
            member("details", Value::object({member("kind", Value{"earthquake"}),
                                             member("severity", Value{"minor"})})),
            member("shakemap_url", Value{"https://example.invalid/x"}),
            member("aftershock_probability", Value{0.4}),
        })}),
        Freshness::Live, report);

    CHECK_EQ(nodes.size(), 1U);
    CHECK_EQ(report.unknown_fields, 2);
    CHECK(report.defects.empty());
}

}  // namespace

int main() {
    a_well_formed_observation_becomes_a_placed_node();
    the_grading_follows_the_providers_own_scale();
    a_tsunami_warning_outranks_the_magnitude();
    silence_about_authority_is_not_authority();
    a_declared_fallback_source_stays_unauthoritative();
    an_observation_with_no_place_is_dropped();
    an_unknown_grading_is_placed_and_marked();
    an_unknown_observation_kind_still_appears();
    a_missing_summary_falls_back_to_the_identifier();
    one_bad_record_costs_one_record();
    a_newer_world_schema_is_read_anyway();
    a_payload_of_the_wrong_shape_yields_nothing_and_says_why();
    freshness_travels_with_the_observations();
    a_field_from_a_newer_provider_is_counted_not_refused();
    return archie::visual::test::report();
}
