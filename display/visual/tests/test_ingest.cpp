// The trust boundary: what a display does when the payload is not what it expected.

#include "archie/visual/ingest.hpp"

#include <algorithm>
#include <string>

#include "harness.hpp"

using namespace archie::visual;

namespace {

Value module_record(const std::string& id, const std::string& state,
                    std::vector<Value> dependencies = {}) {
    return Value::object({
        member("id", Value{id}),
        member("version", Value{"0.1.0"}),
        member("state", Value{state}),
        member("explanation", Value{state}),
        member("dependencies", Value::array(std::move(dependencies))),
    });
}

Value snapshot_with(std::vector<Value> modules) {
    return Value::object({member("modules", Value::array(std::move(modules)))});
}

Value availability(const char* state) {
    return Value::object({member("state", Value{state})});
}

/// A kernel payload carrying both availability tags, as the daemon always sends it.
/// Building partial ones by hand invites a test that passes for the wrong reason.
Value kernel_payload(std::vector<Member> fields) {
    std::vector<Member> all{member("pressure_availability", availability("unavailable")),
                            member("thermal_availability", availability("unavailable"))};
    for (Member& field : fields) {
        // Later entries win: `find` takes the first match, so an override has to replace
        // rather than follow.
        const auto existing =
            std::find_if(all.begin(), all.end(),
                         [&](const Member& m) { return m.key == field.key; });
        if (existing != all.end()) {
            existing->value = std::move(field.value);
        } else {
            all.push_back(std::move(field));
        }
    }
    return Value::object(std::move(all));
}

// ---------------------------------------------------------------------------
// The happy path, so the failure paths mean something.
// ---------------------------------------------------------------------------

void a_well_formed_payload_arrives_intact() {
    CASE("a good snapshot produces nodes and a clean report");

    IngestReport report{};
    const std::vector<NodeSpec> nodes = ingest_modules(
        snapshot_with({module_record("core.archie", "loaded"),
                       module_record("science.physics", "unsatisfied", {Value{"core.archie"}})}),
        Freshness::Live, report);

    CHECK_EQ(nodes.size(), 2U);
    CHECK(report.clean());
    CHECK_EQ(report.accepted, 2);
    CHECK_EQ(report.dropped, 0);
    CHECK_EQ(nodes[0].id, std::string{"core.archie"});
    CHECK_EQ(nodes[0].kind, NodeKind::Module);
    CHECK_EQ(nodes[0].severity, 0);
    CHECK_EQ(nodes[1].severity, 1);
    CHECK_EQ(nodes[1].depends_on.size(), 1U);
    CHECK_BECAUSE(nodes[1].magnitude == 1.0F,
                  "size comes from how much the node leans on others, which is a real "
                  "structural fact rather than a number invented to make the scene look busy");
}

void module_states_are_graded_by_what_an_operator_can_do() {
    CASE("severity follows the upstream enum's own documentation");

    bool recognised = false;
    // Deliberately off is not a problem.
    CHECK_EQ(severity_for_module_state("loaded", recognised), 0);
    CHECK(recognised);
    CHECK_EQ(severity_for_module_state("disabled", recognised), 0);
    CHECK_EQ(severity_for_module_state("not-in-profile", recognised), 0);

    // Cannot run here. Expected on some nodes; not actionable.
    CHECK_EQ(severity_for_module_state("unsatisfied", recognised), 1);
    CHECK_EQ(severity_for_module_state("wrong-arch", recognised), 1);

    // Blocked by something a person can change.
    CHECK_EQ(severity_for_module_state("missing-dep", recognised), 2);
    CHECK_EQ(severity_for_module_state("denied", recognised), 2);

    // Broken.
    CHECK_EQ(severity_for_module_state("invalid", recognised), 3);
    CHECK(recognised);

    CHECK_BECAUSE(severity_for_module_state("loaded", recognised) <
                      severity_for_module_state("invalid", recognised),
                  "the ordering is the whole point: colour is derived from this, so a "
                  "grading that put a working module above a broken one would paint the "
                  "healthy machine as the emergency");
}

// ---------------------------------------------------------------------------
// A missing field is not a zero.
// ---------------------------------------------------------------------------

void an_absent_state_is_not_a_healthy_state() {
    CASE("a module we can name but cannot grade is not drawn as fine");

    IngestReport report{};
    const std::vector<NodeSpec> nodes = ingest_modules(
        snapshot_with({Value::object({member("id", Value{"ungraded"})})}), Freshness::Live,
        report);

    CHECK_EQ(nodes.size(), 1U);
    CHECK_BECAUSE(nodes[0].severity != 0,
                  "reading an absent severity as 0 draws a healthy node for something the "
                  "display knows nothing about, which is worse than showing nothing because "
                  "it is indistinguishable from good news");
    CHECK_EQ(report.count_of(DefectKind::MissingField), 1);
    CHECK_BECAUSE(report.accepted == 1,
                  "it is still worth drawing: we know the module exists, we just cannot "
                  "grade it");
}

void a_number_where_a_string_belongs_is_not_coerced() {
    CASE("wrong types are reported, never quietly converted");

    IngestReport report{};
    const std::vector<NodeSpec> nodes = ingest_modules(
        snapshot_with({Value::object({member("id", Value{"m"}), member("state", Value{7})})}),
        Freshness::Live, report);

    CHECK_EQ(nodes.size(), 1U);
    CHECK_EQ(report.count_of(DefectKind::WrongType), 1);
    CHECK_BECAUSE(nodes[0].severity != 0,
                  "a payload that sends a number where it promised a string has a bug, and "
                  "guessing at what it meant means the bug ships");
}

// ---------------------------------------------------------------------------
// An unknown value is not an error.
// ---------------------------------------------------------------------------

void a_state_from_a_newer_daemon_is_still_drawn() {
    CASE("an unrecognised enum is reported and used, not refused");

    IngestReport report{};
    const std::vector<NodeSpec> nodes = ingest_modules(
        snapshot_with({module_record("future.module", "quantum-entangled")}), Freshness::Live,
        report);

    CHECK_EQ(nodes.size(), 1U);
    CHECK_EQ(report.dropped, 0);
    CHECK_EQ(report.count_of(DefectKind::UnknownEnum), 1);
    CHECK_BECAUSE(nodes[0].severity == 1,
                  "not claimed to be fine and not claimed to be critical: shown as something "
                  "to look at, which is exactly what it is");
    CHECK_BECAUSE(report.accepted == 1,
                  "refusing it would mean every upgrade of the control plane blanks part of "
                  "the observatory until the observatory is rebuilt to match");
}

void fields_this_build_has_never_seen_are_counted_and_ignored() {
    CASE("forward compatibility: a newer daemon's extra fields are not a defect");

    IngestReport report{};
    const std::vector<NodeSpec> nodes = ingest_modules(
        snapshot_with({Value::object({
            member("id", Value{"m"}),
            member("state", Value{"loaded"}),
            member("quantum_coherence", Value{0.94}),
            member("successor_of", Value{"m.old"}),
        })}),
        Freshness::Live, report);

    CHECK_EQ(nodes.size(), 1U);
    CHECK_EQ(nodes[0].severity, 0);
    CHECK_EQ(report.unknown_fields, 2);
    CHECK_BECAUSE(report.defects.empty(),
                  "an unrecognised field is the ordinary signature of a daemon newer than the "
                  "display, not something to warn about");
}

void a_newer_schema_is_noted_and_still_read() {
    CASE("a kernel snapshot from a newer daemon is used, with a note");

    IngestReport report{};
    const KernelReading reading = ingest_kernel(
        Value::object({
            member("schema_version", Value{99}),
            member("pressure_availability", Value::object({member("state", Value{"available"})})),
            member("verdict", Value{"idle"}),
            member("thermal_availability", Value::object({member("state", Value{"available"})})),
            member("thermal_state", Value{"nominal"}),
        }),
        Freshness::Live, true, report);

    CHECK(report.newer_schema);
    CHECK_EQ(report.count_of(DefectKind::NewerSchema), 1);
    CHECK_BECAUSE(reading.verdict.has_value() && *reading.verdict == PressureVerdict::Idle,
                  "the fields this build does understand are still worth reading; refusing "
                  "the whole payload over a version number blanks a working display");
    CHECK_EQ(ceiling_for(reading).tier, RenderTier::Ultra);
}

// ---------------------------------------------------------------------------
// One bad record costs one record.
// ---------------------------------------------------------------------------

void a_malformed_entry_does_not_cost_the_payload() {
    CASE("the rest of the modules still arrive");

    IngestReport report{};
    const std::vector<NodeSpec> nodes = ingest_modules(
        snapshot_with({
            module_record("good.one", "loaded"),
            Value{"this is not a module record at all"},
            Value::object({member("state", Value{"loaded"})}),  // no id
            module_record("good.two", "loaded"),
        }),
        Freshness::Live, report);

    CHECK_BECAUSE(nodes.size() == 2U,
                  "a parser that gave up on the first defect would hand the operator an empty "
                  "screen because one module in forty had a null where a string belonged");
    CHECK_EQ(report.dropped, 2);
    CHECK_EQ(report.accepted, 2);
    CHECK_EQ(nodes[0].id, std::string{"good.one"});
    CHECK_EQ(nodes[1].id, std::string{"good.two"});
}

void a_record_with_no_identity_is_unfixable() {
    CASE("an empty id is dropped rather than invented");

    IngestReport report{};
    const std::vector<NodeSpec> nodes = ingest_modules(
        snapshot_with({Value::object({member("id", Value{""}), member("state", Value{"loaded"})})}),
        Freshness::Live, report);

    CHECK(nodes.empty());
    CHECK_EQ(report.dropped, 1);
    CHECK_BECAUSE(report.count_of(DefectKind::EmptyIdentity) == 1,
                  "a record with no identity cannot be tracked between polls, focused, or told "
                  "apart from the next one that arrives without an id");
}

void a_broken_dependency_costs_one_edge() {
    CASE("a bad entry in a list does not sink the node that owns it");

    IngestReport report{};
    const std::vector<NodeSpec> nodes = ingest_modules(
        snapshot_with({module_record("m", "loaded", {Value{"real.dep"}, Value{42}, Value{""}})}),
        Freshness::Live, report);

    CHECK_EQ(nodes.size(), 1U);
    CHECK_BECAUSE(nodes[0].depends_on.size() == 1U,
                  "losing one line is better than losing the node it belonged to");
    CHECK_EQ(nodes[0].depends_on[0], std::string{"real.dep"});
    CHECK_EQ(report.count_of(DefectKind::WrongType), 2);
}

void a_repeated_key_is_reported() {
    CASE("the same field twice in one object is malformed");

    IngestReport report{};
    const std::vector<NodeSpec> nodes = ingest_modules(
        snapshot_with({Value::object({
            member("id", Value{"first"}),
            member("id", Value{"second"}),
            member("state", Value{"loaded"}),
        })}),
        Freshness::Live, report);

    CHECK_EQ(nodes.size(), 1U);
    CHECK_BECAUSE(nodes[0].id == std::string{"first"},
                  "taking the first is the only choice that does not depend on iteration "
                  "order");
    CHECK_EQ(report.count_of(DefectKind::DuplicateKey), 1);
}

// ---------------------------------------------------------------------------
// Wholly wrong payloads.
// ---------------------------------------------------------------------------

void a_payload_of_the_wrong_shape_yields_nothing_and_says_why() {
    CASE("garbage in, nothing out, and a reason");

    IngestReport not_object{};
    CHECK(ingest_modules(Value{"a bare string"}, Freshness::Live, not_object).empty());
    CHECK_EQ(not_object.count_of(DefectKind::NotAnObject), 1);

    IngestReport no_modules{};
    CHECK(ingest_modules(Value::object({member("nodes", Value::array({}))}), Freshness::Live,
                         no_modules)
              .empty());
    CHECK_EQ(no_modules.count_of(DefectKind::MissingField), 1);

    IngestReport wrong_type{};
    CHECK(ingest_modules(Value::object({member("modules", Value{"not a list"})}), Freshness::Live,
                         wrong_type)
              .empty());
    CHECK_EQ(wrong_type.count_of(DefectKind::WrongType), 1);

    IngestReport empty_list{};
    CHECK_BECAUSE(ingest_modules(snapshot_with({}), Freshness::Live, empty_list).empty() &&
                      empty_list.clean(),
                  "a node with no modules is a legitimate state, not a defect");
}

// ---------------------------------------------------------------------------
// Kernel telemetry, where a wrong answer costs the machine's GPU.
// ---------------------------------------------------------------------------

void unreadable_kernel_telemetry_never_claims_headroom() {
    CASE("a malformed kernel payload resolves the cautious way");

    IngestReport report{};
    const KernelReading reading = ingest_kernel(Value{"garbage"}, Freshness::Live, true, report);

    CHECK(!reading.pressure_available);
    CHECK(!reading.thermal_available);
    CHECK_BECAUSE(ceiling_for(reading).tier == RenderTier::Normal,
                  "an unparseable payload is not evidence that the machine is idle, and the "
                  "cost of getting this backwards is spending a GPU the machine needed");
    CHECK_EQ(report.count_of(DefectKind::NotAnObject), 1);
}

void an_unknown_verdict_is_not_guessed_at() {
    CASE("a pressure verdict this build does not know leaves no value behind");

    IngestReport report{};
    const KernelReading reading = ingest_kernel(
        Value::object({
            member("pressure_availability", Value::object({member("state", Value{"available"})})),
            member("verdict", Value{"gently-humming"}),
        }),
        Freshness::Live, true, report);

    CHECK(reading.pressure_available);
    CHECK_BECAUSE(!reading.verdict.has_value(),
                  "guessing at an unknown verdict is how a display ends up rendering at full "
                  "cinematic cost on a machine whose scheduler just invented a word for "
                  "'on fire'");
    CHECK_EQ(report.count_of(DefectKind::UnknownEnum), 1);
    CHECK_BECAUSE(ceiling_for(reading).tier == RenderTier::Normal,
                  "the governor already treats an available sensor with no value as no "
                  "evidence at all, so the two layers agree without either one guessing");
}

void an_unavailable_sensor_is_read_as_unavailable() {
    CASE("the availability tag is honoured in both directions");

    IngestReport report{};
    const KernelReading reading = ingest_kernel(
        Value::object({
            member("pressure_availability",
                   Value::object({member("state", Value{"unavailable"}),
                                  member("reason", Value{"no PSI support in this kernel"})})),
            member("verdict", Value{"idle"}),
            member("thermal_availability", Value::object({member("state", Value{"available"})})),
            member("thermal_state", Value{"nominal"}),
        }),
        Freshness::Live, true, report);

    CHECK_BECAUSE(!reading.pressure_available,
                  "a stale verdict left in the payload beside an unavailable flag must not be "
                  "believed; the flag exists precisely to say when the value may be used");
    CHECK(reading.thermal_available);
    CHECK_EQ(ceiling_for(reading).tier, RenderTier::Normal);
    CHECK_BECAUSE(report.defects.empty(),
                  "an unavailable sensor is a fact about the machine, not a defect in the "
                  "payload");
}

void every_thermal_state_round_trips() {
    CASE("the thermal vocabulary matches the daemon's");

    const std::pair<const char*, ThermalState> cases[] = {
        {"unknown", ThermalState::Unknown},     {"nominal", ThermalState::Nominal},
        {"warm", ThermalState::Warm},           {"throttling", ThermalState::Throttling},
        {"critical", ThermalState::Critical},
    };

    for (const auto& [label, expected] : cases) {
        IngestReport report{};
        const KernelReading reading =
            ingest_kernel(kernel_payload({member("thermal_availability", availability("available")),
                                          member("thermal_state", Value{label})}),
                          Freshness::Live, true, report);
        CHECK(reading.thermal.has_value() && *reading.thermal == expected);
        CHECK(report.defects.empty());
    }
}

void every_pressure_verdict_round_trips() {
    CASE("the pressure vocabulary matches the daemon's");

    const std::pair<const char*, PressureVerdict> cases[] = {
        {"idle", PressureVerdict::Idle},
        {"light", PressureVerdict::Light},
        {"contended", PressureVerdict::Contended},
        {"saturated", PressureVerdict::Saturated},
    };

    for (const auto& [label, expected] : cases) {
        IngestReport report{};
        const KernelReading reading = ingest_kernel(
            kernel_payload({member("pressure_availability", availability("available")),
                            member("verdict", Value{label})}),
            Freshness::Live, true, report);
        CHECK(reading.verdict.has_value() && *reading.verdict == expected);
        CHECK(report.defects.empty());
    }
}

void an_available_sensor_with_no_reading_is_reported() {
    CASE("a payload that promises a value and omits it is a defect");

    IngestReport report{};
    const KernelReading reading = ingest_kernel(
        kernel_payload({member("pressure_availability", availability("available"))}),
        Freshness::Live, true, report);

    CHECK(reading.pressure_available);
    CHECK(!reading.verdict.has_value());
    CHECK_EQ(report.count_of(DefectKind::MissingField), 1);
    CHECK_EQ(ceiling_for(reading).tier, RenderTier::Normal);
}

void freshness_travels_with_the_nodes() {
    CASE("age reaches every node the payload produced");

    IngestReport report{};
    const std::vector<NodeSpec> nodes =
        ingest_modules(snapshot_with({module_record("a", "loaded"), module_record("b", "loaded")}),
                       Freshness::Stale, report);

    CHECK_EQ(nodes.size(), 2U);
    for (const NodeSpec& node : nodes) {
        CHECK_BECAUSE(node.freshness == Freshness::Stale,
                      "a node drawn at full strength from a four-minute-old snapshot is the "
                      "display vouching for something it cannot vouch for");
    }
}

}  // namespace

int main() {
    a_well_formed_payload_arrives_intact();
    module_states_are_graded_by_what_an_operator_can_do();
    an_absent_state_is_not_a_healthy_state();
    a_number_where_a_string_belongs_is_not_coerced();
    a_state_from_a_newer_daemon_is_still_drawn();
    fields_this_build_has_never_seen_are_counted_and_ignored();
    a_newer_schema_is_noted_and_still_read();
    a_malformed_entry_does_not_cost_the_payload();
    a_record_with_no_identity_is_unfixable();
    a_broken_dependency_costs_one_edge();
    a_repeated_key_is_reported();
    a_payload_of_the_wrong_shape_yields_nothing_and_says_why();
    unreadable_kernel_telemetry_never_claims_headroom();
    an_unknown_verdict_is_not_guessed_at();
    an_unavailable_sensor_is_read_as_unavailable();
    every_thermal_state_round_trips();
    every_pressure_verdict_round_trips();
    an_available_sensor_with_no_reading_is_reported();
    freshness_travels_with_the_nodes();
    return archie::visual::test::report();
}
