#include "archie/visual/ingest.hpp"

#include <algorithm>
#include <array>
#include <string>

namespace archie::visual {
namespace {

/// Keys this build reads from a module record. Anything else is a field from a newer
/// daemon, which is counted and otherwise ignored.
constexpr std::array<std::string_view, 5> kModuleKeys{"id", "version", "state", "explanation",
                                                      "dependencies"};

constexpr std::array<std::string_view, 5> kObservationKeys{"id", "occurred_at", "position",
                                                           "summary", "details"};

constexpr std::array<std::string_view, 7> kPeerKeys{"node_id",      "hostname", "profile",
                                                    "trust",        "health",   "reachable",
                                                    "capabilities"};

constexpr std::array<std::string_view, 6> kModelKeys{
    "id", "provider", "locality", "availability", "max_privacy_grade", "context_window"};

template <std::size_t N>
bool known_key(const std::array<std::string_view, N>& keys, const std::string& key) {
    return std::find(keys.begin(), keys.end(), key) != keys.end();
}

/// Read a required string. Reports whichever way it is absent and returns nullptr.
const std::string* required_string(const Value& record, std::string_view key,
                                   const std::string& path, IngestReport& report) {
    const Value* field = record.find(key);
    if (field == nullptr) {
        report.note(DefectKind::MissingField, path, "expected a string");
        return nullptr;
    }
    const std::string* text = field->as_string();
    if (text == nullptr) {
        report.note(DefectKind::WrongType, path, "expected a string");
        return nullptr;
    }
    return text;
}

}  // namespace

void IngestReport::note(DefectKind kind, std::string path, std::string detail) {
    defects.push_back(IngestDefect{kind, std::move(path), std::move(detail)});
}

int IngestReport::count_of(DefectKind kind) const noexcept {
    return static_cast<int>(
        std::count_if(defects.begin(), defects.end(),
                      [&](const IngestDefect& defect) { return defect.kind == kind; }));
}

int severity_for_module_state(std::string_view label, bool& recognised) noexcept {
    recognised = true;

    // Deliberately off, or excluded by profile. Not a problem: a node that was never
    // meant to run this module is behaving correctly.
    if (label == "loaded" || label == "disabled" || label == "not-in-profile") {
        return 0;
    }
    // Cannot run on this hardware. Expected on some nodes in a heterogeneous fabric,
    // and not something an operator can fix by acting on the module.
    if (label == "unsatisfied" || label == "wrong-arch") {
        return 1;
    }
    // Blocked by something a person can change: a missing dependency, or policy.
    if (label == "missing-dep" || label == "denied") {
        return 2;
    }
    // The module itself is broken.
    if (label == "invalid") {
        return 3;
    }

    recognised = false;
    // A state from a daemon this build predates. Not claimed to be fine, not claimed to
    // be critical: shown as something to look at, which is exactly what it is.
    return 1;
}

int severity_for_model(std::string_view availability, bool& recognised) noexcept {
    recognised = true;

    // Upstream calls Ready and Cold usable. Cold means registered but not resident,
    // which costs latency on first call and nothing else.
    if (availability == "ready" || availability == "cold") {
        return 0;
    }
    // Administratively off. A model nobody asked to run is behaving correctly, the same
    // as a disabled module.
    if (availability == "disabled") {
        return 0;
    }
    // Temporarily unreachable: a statement about what we cannot see rather than about
    // what is broken, graded like an unreachable peer.
    if (availability == "offline") {
        return 2;
    }

    recognised = false;
    return 1;
}

std::vector<NodeSpec> ingest_models(const Value& snapshot, Freshness freshness,
                                    IngestReport& report) {
    std::vector<NodeSpec> nodes;

    if (snapshot.as_object() == nullptr) {
        report.note(DefectKind::NotAnObject, "", "the snapshot is not an object");
        return nodes;
    }

    const Value* models = snapshot.find("models");
    if (models == nullptr) {
        report.note(DefectKind::MissingField, "models", "no model list in the snapshot");
        return nodes;
    }
    const std::vector<Value>* list = models->as_array();
    if (list == nullptr) {
        report.note(DefectKind::WrongType, "models", "expected an array");
        return nodes;
    }

    nodes.reserve(list->size());
    for (std::size_t index = 0; index < list->size(); ++index) {
        const Value& record = (*list)[index];
        const std::string path = "models[" + std::to_string(index) + "]";

        if (record.as_object() == nullptr) {
            report.note(DefectKind::NotAnObject, path, "expected an object");
            ++report.dropped;
            continue;
        }

        const std::string* id = required_string(record, "id", path + ".id", report);
        if (id == nullptr || id->empty()) {
            if (id != nullptr) {
                report.note(DefectKind::EmptyIdentity, path + ".id", "an empty id");
            }
            ++report.dropped;
            continue;
        }

        NodeSpec spec{};
        spec.id = *id;
        spec.kind = NodeKind::Model;
        spec.label = *id;
        spec.freshness = freshness;

        if (const Value* availability = record.find("availability"); availability != nullptr) {
            if (const std::string* label = availability->as_string(); label != nullptr) {
                bool recognised = true;
                spec.severity = severity_for_model(*label, recognised);
                if (!recognised) {
                    report.note(DefectKind::UnknownEnum, path + ".availability",
                                "unrecognised model availability: " + *label);
                }
            } else {
                report.note(DefectKind::WrongType, path + ".availability", "expected a string");
                spec.severity = 1;
            }
        } else {
            report.note(DefectKind::MissingField, path + ".availability", "expected a string");
            spec.severity = 1;
        }

        // Size is the context window, in thousands of tokens. Using it raw would put
        // every model at the top of the scale once it passed a few thousand, since the
        // bounded mapping saturates; in thousands, a 4k model and a 200k one are still
        // visibly different sizes.
        if (const Value* window = record.find("context_window"); window != nullptr) {
            if (const std::optional<double> value = window->as_number(); value.has_value()) {
                spec.magnitude = static_cast<float>(*value / 1000.0);
            } else {
                report.note(DefectKind::WrongType, path + ".context_window",
                            "expected a number");
            }
        }

        if (const std::vector<Member>* fields = record.as_object(); fields != nullptr) {
            for (const Member& field : *fields) {
                if (!known_key(kModelKeys, field.key)) {
                    ++report.unknown_fields;
                }
            }
        }

        nodes.push_back(std::move(spec));
        ++report.accepted;
    }

    return nodes;
}

int severity_for_peer(std::string_view health, bool reachable, bool& recognised) noexcept {
    recognised = true;

    int graded = 0;
    if (health == "ok") {
        graded = 0;
    } else if (health == "degraded") {
        graded = 1;
    } else if (health == "unreachable") {
        // No recent contact. Not claimed to be broken, not claimed to be fine.
        graded = 2;
    } else if (health == "unhealthy") {
        graded = 3;
    } else {
        recognised = false;
        graded = 1;
    }

    if (!reachable && graded < 2) {
        // The payload said healthy and also said we cannot reach it. Those disagree, and
        // the cautious reading is the one that does not tell an operator a node they
        // cannot talk to is fine.
        graded = 2;
    }
    return graded;
}

std::vector<NodeSpec> ingest_peers(const Value& snapshot, Freshness freshness,
                                   IngestReport& report) {
    std::vector<NodeSpec> nodes;

    if (snapshot.as_object() == nullptr) {
        report.note(DefectKind::NotAnObject, "", "the snapshot is not an object");
        return nodes;
    }

    const Value* peers = snapshot.find("nodes");
    if (peers == nullptr) {
        report.note(DefectKind::MissingField, "nodes", "no peer list in the snapshot");
        return nodes;
    }
    const std::vector<Value>* list = peers->as_array();
    if (list == nullptr) {
        report.note(DefectKind::WrongType, "nodes", "expected an array");
        return nodes;
    }

    nodes.reserve(list->size());
    for (std::size_t index = 0; index < list->size(); ++index) {
        const Value& record = (*list)[index];
        const std::string path = "nodes[" + std::to_string(index) + "]";

        if (record.as_object() == nullptr) {
            report.note(DefectKind::NotAnObject, path, "expected an object");
            ++report.dropped;
            continue;
        }

        const std::string* id = required_string(record, "node_id", path + ".node_id", report);
        if (id == nullptr || id->empty()) {
            if (id != nullptr) {
                report.note(DefectKind::EmptyIdentity, path + ".node_id", "an empty node id");
            }
            ++report.dropped;
            continue;
        }

        NodeSpec spec{};
        spec.id = *id;
        spec.kind = NodeKind::Peer;
        spec.freshness = freshness;
        spec.label = *id;

        if (const Value* hostname = record.find("hostname"); hostname != nullptr) {
            if (const std::string* text = hostname->as_string();
                text != nullptr && !text->empty()) {
                // A hostname is what an operator calls the machine. The node id is what
                // the fabric calls it, and is the fallback rather than the first choice.
                spec.label = *text;
            } else if (hostname->as_string() == nullptr) {
                report.note(DefectKind::WrongType, path + ".hostname", "expected a string");
            }
        }

        // Absent means we were not told, and we do not get to assume the friendly answer.
        bool reachable = false;
        if (const Value* flag = record.find("reachable"); flag != nullptr) {
            if (const std::optional<bool> value = flag->as_boolean(); value.has_value()) {
                reachable = *value;
            } else {
                report.note(DefectKind::WrongType, path + ".reachable", "expected a boolean");
            }
        } else {
            report.note(DefectKind::MissingField, path + ".reachable", "expected a boolean");
        }

        if (const Value* health = record.find("health"); health != nullptr) {
            if (const std::string* label = health->as_string(); label != nullptr) {
                bool recognised = true;
                spec.severity = severity_for_peer(*label, reachable, recognised);
                if (!recognised) {
                    report.note(DefectKind::UnknownEnum, path + ".health",
                                "unrecognised node health: " + *label);
                }
            } else {
                report.note(DefectKind::WrongType, path + ".health", "expected a string");
                spec.severity = reachable ? 1 : 2;
            }
        } else {
            report.note(DefectKind::MissingField, path + ".health", "expected a string");
            spec.severity = reachable ? 1 : 2;
        }

        // Size is what the peer brings to the fabric. A node offering twelve capabilities
        // is a bigger part of it than one offering two, which is a real fact rather than
        // a number chosen to make the scene look varied.
        if (const Value* capabilities = record.find("capabilities"); capabilities != nullptr) {
            if (const std::vector<Value>* declared = capabilities->as_array();
                declared != nullptr) {
                spec.magnitude = static_cast<float>(declared->size());
            } else {
                report.note(DefectKind::WrongType, path + ".capabilities", "expected an array");
            }
        }

        if (const std::vector<Member>* fields = record.as_object(); fields != nullptr) {
            for (const Member& field : *fields) {
                if (field.key == "last_seen") {
                    continue;
                }
                if (!known_key(kPeerKeys, field.key)) {
                    ++report.unknown_fields;
                }
            }
        }

        nodes.push_back(std::move(spec));
        ++report.accepted;
    }

    return nodes;
}

std::vector<NodeSpec> ingest_modules(const Value& snapshot, Freshness freshness,
                                     IngestReport& report) {
    std::vector<NodeSpec> nodes;

    if (snapshot.as_object() == nullptr) {
        report.note(DefectKind::NotAnObject, "", "the snapshot is not an object");
        return nodes;
    }

    const Value* modules = snapshot.find("modules");
    if (modules == nullptr) {
        report.note(DefectKind::MissingField, "modules", "no module list in the snapshot");
        return nodes;
    }

    const std::vector<Value>* list = modules->as_array();
    if (list == nullptr) {
        report.note(DefectKind::WrongType, "modules", "expected an array");
        return nodes;
    }

    nodes.reserve(list->size());
    for (std::size_t index = 0; index < list->size(); ++index) {
        const Value& record = (*list)[index];
        const std::string path = "modules[" + std::to_string(index) + "]";

        if (record.as_object() == nullptr) {
            report.note(DefectKind::NotAnObject, path, "expected an object");
            ++report.dropped;
            continue;
        }

        const std::string* id = required_string(record, "id", path + ".id", report);
        if (id == nullptr || id->empty()) {
            if (id != nullptr) {
                report.note(DefectKind::EmptyIdentity, path + ".id", "an empty id");
            }
            // Unfixable. A record with no identity cannot be tracked between polls,
            // focused, or told apart from the next one that arrives without an id.
            ++report.dropped;
            continue;
        }

        if (record.count("id") > 1) {
            report.note(DefectKind::DuplicateKey, path + ".id", "the key appears more than once");
        }

        NodeSpec spec{};
        spec.id = *id;
        spec.kind = NodeKind::Module;
        spec.label = *id;
        spec.freshness = freshness;

        // Both of these were being read past. `explanation` in particular is documented
        // upstream as always populated, and it is the whole difference between showing
        // that something is wrong and showing what is wrong with it.
        if (const Value* version = record.find("version"); version != nullptr) {
            if (const std::string* text = version->as_string(); text != nullptr) {
                spec.version = *text;
            } else {
                report.note(DefectKind::WrongType, path + ".version", "expected a string");
            }
        }
        if (const Value* explanation = record.find("explanation"); explanation != nullptr) {
            if (const std::string* text = explanation->as_string(); text != nullptr) {
                spec.explanation = *text;
            } else {
                report.note(DefectKind::WrongType, path + ".explanation", "expected a string");
            }
        }

        // State is graded, but its absence does not sink the record: a module we can
        // name and cannot grade is still worth drawing, marked as ungraded.
        const Value* state = record.find("state");
        if (state == nullptr) {
            report.note(DefectKind::MissingField, path + ".state", "expected a string");
            spec.severity = 1;
        } else if (const std::string* label = state->as_string(); label != nullptr) {
            bool recognised = true;
            spec.severity = severity_for_module_state(*label, recognised);
            if (!recognised) {
                report.note(DefectKind::UnknownEnum, path + ".state",
                            "unrecognised module state: " + *label);
            }
        } else {
            report.note(DefectKind::WrongType, path + ".state", "expected a string");
            spec.severity = 1;
        }

        // Dependencies drive the edges, so a malformed entry costs that edge and
        // nothing else. Losing one line is better than losing the node it belonged to.
        if (const Value* dependencies = record.find("dependencies"); dependencies != nullptr) {
            if (const std::vector<Value>* declared = dependencies->as_array();
                declared != nullptr) {
                for (std::size_t d = 0; d < declared->size(); ++d) {
                    const std::string* dependency = (*declared)[d].as_string();
                    if (dependency == nullptr || dependency->empty()) {
                        report.note(DefectKind::WrongType,
                                    path + ".dependencies[" + std::to_string(d) + "]",
                                    "expected a non-empty string");
                        continue;
                    }
                    spec.depends_on.push_back(*dependency);
                }
            } else {
                report.note(DefectKind::WrongType, path + ".dependencies", "expected an array");
            }
        }

        // Magnitude is how much the rest of the node leans on this module. It is a real
        // structural fact rather than a made-up size, and it is why a core module is
        // drawn larger than a leaf one.
        spec.magnitude = static_cast<float>(spec.depends_on.size());

        if (const std::vector<Member>* fields = record.as_object(); fields != nullptr) {
            for (const Member& field : *fields) {
                if (!known_key(kModuleKeys, field.key)) {
                    ++report.unknown_fields;
                }
            }
        }

        nodes.push_back(std::move(spec));
        ++report.accepted;
    }

    return nodes;
}

int severity_for_earthquake(std::string_view label, bool& recognised) noexcept {
    recognised = true;

    // Upstream grades by magnitude: under 2, under 4, under 6, under 8, and above.
    if (label == "trace") {
        return 0;
    }
    if (label == "minor") {
        return 1;
    }
    if (label == "moderate") {
        return 2;
    }
    if (label == "strong") {
        return 3;
    }
    if (label == "major") {
        // Past the top of the palette on purpose. It is drawn in the same colour as a
        // Strong event and outranks one for the render budget, which is what decides
        // whether it is drawn at all on a busy globe.
        return 4;
    }

    recognised = false;
    return 1;
}

std::vector<NodeSpec> ingest_world(const Value& world, Freshness freshness,
                                   IngestReport& report) {
    std::vector<NodeSpec> nodes;

    if (world.as_object() == nullptr) {
        report.note(DefectKind::NotAnObject, "", "the world snapshot is not an object");
        return nodes;
    }

    if (const Value* version = world.find("schema_version"); version != nullptr) {
        if (const std::optional<double> number = version->as_number(); number.has_value()) {
            report.schema_version = number;
            if (*number > kKnownWorldSchema) {
                report.newer_schema = true;
                report.note(DefectKind::NewerSchema, "schema_version",
                            "the daemon speaks a newer world schema than this display");
            }
        } else {
            report.note(DefectKind::WrongType, "schema_version", "expected a number");
        }
    }

    const Value* observations = world.find("observations");
    if (observations == nullptr) {
        report.note(DefectKind::MissingField, "observations", "no observation list");
        return nodes;
    }
    const std::vector<Value>* list = observations->as_array();
    if (list == nullptr) {
        report.note(DefectKind::WrongType, "observations", "expected an array");
        return nodes;
    }

    nodes.reserve(list->size());
    for (std::size_t index = 0; index < list->size(); ++index) {
        const Value& record = (*list)[index];
        const std::string path = "observations[" + std::to_string(index) + "]";

        if (record.as_object() == nullptr) {
            report.note(DefectKind::NotAnObject, path, "expected an object");
            ++report.dropped;
            continue;
        }

        const std::string* id = required_string(record, "id", path + ".id", report);
        if (id == nullptr || id->empty()) {
            if (id != nullptr) {
                report.note(DefectKind::EmptyIdentity, path + ".id", "an empty id");
            }
            ++report.dropped;
            continue;
        }

        // An observation with no place cannot go on a globe, and unlike a missing grade
        // there is no reduced form of it worth drawing: a marker somewhere arbitrary is
        // a claim about where something happened, and inventing one is worse than
        // omitting the record and saying so.
        const Value* position = record.find("position");
        if (position == nullptr) {
            report.note(DefectKind::MissingField, path + ".position", "expected an object");
            ++report.dropped;
            continue;
        }
        const Value* latitude = position->find("latitude");
        const Value* longitude = position->find("longitude");
        const std::optional<double> lat =
            latitude == nullptr ? std::nullopt : latitude->as_number();
        const std::optional<double> lon =
            longitude == nullptr ? std::nullopt : longitude->as_number();
        if (!lat.has_value() || !lon.has_value()) {
            report.note(latitude == nullptr || longitude == nullptr ? DefectKind::MissingField
                                                                   : DefectKind::WrongType,
                        path + ".position", "expected numeric latitude and longitude");
            ++report.dropped;
            continue;
        }

        NodeSpec spec{};
        spec.id = *id;
        spec.kind = NodeKind::Observation;
        spec.freshness = freshness;
        spec.location = GeoPoint{*lat, *lon};

        if (const Value* summary = record.find("summary"); summary != nullptr) {
            if (const std::string* text = summary->as_string(); text != nullptr) {
                spec.label = *text;
            } else {
                report.note(DefectKind::WrongType, path + ".summary", "expected a string");
            }
        }
        if (spec.label.empty()) {
            // A marker with no summary is still worth plotting; it only has to be
            // identifiable when the operator clicks it.
            spec.label = *id;
        }

        // Authority defaults to false, not true. The optimistic reading of silence is
        // exactly what this field exists to refuse: a payload that does not say its
        // source was entitled to speak has not said it was.
        spec.authoritative = false;
        if (const Value* provenance = record.find("provenance"); provenance != nullptr) {
            if (const Value* authoritative = provenance->find("authoritative");
                authoritative != nullptr) {
                if (const std::optional<bool> flag = authoritative->as_boolean();
                    flag.has_value()) {
                    spec.authoritative = *flag;
                } else {
                    report.note(DefectKind::WrongType, path + ".provenance.authoritative",
                                "expected a boolean");
                }
            }
        }

        const Value* details = record.find("details");
        if (details == nullptr) {
            report.note(DefectKind::MissingField, path + ".details", "expected an object");
            spec.severity = 1;
        } else {
            const Value* kind = details->find("kind");
            const std::string* kind_label = kind == nullptr ? nullptr : kind->as_string();

            if (const Value* magnitude = details->find("magnitude"); magnitude != nullptr) {
                if (const std::optional<double> value = magnitude->as_number();
                    value.has_value()) {
                    spec.magnitude = static_cast<float>(*value);
                } else {
                    report.note(DefectKind::WrongType, path + ".details.magnitude",
                                "expected a number");
                }
            }

            if (kind_label != nullptr && *kind_label == "earthquake") {
                if (const Value* severity = details->find("severity"); severity != nullptr) {
                    if (const std::string* grade = severity->as_string(); grade != nullptr) {
                        bool recognised = true;
                        spec.severity = severity_for_earthquake(*grade, recognised);
                        if (!recognised) {
                            report.note(DefectKind::UnknownEnum, path + ".details.severity",
                                        "unrecognised earthquake severity: " + *grade);
                        }
                    } else {
                        report.note(DefectKind::WrongType, path + ".details.severity",
                                    "expected a string");
                        spec.severity = 1;
                    }
                } else {
                    report.note(DefectKind::MissingField, path + ".details.severity",
                                "expected a string");
                    spec.severity = 1;
                }

                // A tsunami warning is a fact about consequence rather than about the
                // event, and it outranks the magnitude grading: a moderate quake with a
                // warning attached is what an operator needs to see first.
                if (const Value* tsunami = details->find("tsunami_warning"); tsunami != nullptr) {
                    if (const std::optional<bool> warned = tsunami->as_boolean();
                        warned.has_value() && *warned) {
                        ++spec.severity;
                    }
                }
            } else {
                // A kind this build has never seen. Placed and graded as something to
                // look at, because a display predating a provider is not a reason to
                // leave a hole in the globe.
                report.note(DefectKind::UnknownEnum, path + ".details.kind",
                            kind_label == nullptr
                                ? "no observation kind"
                                : "unrecognised observation kind: " + *kind_label);
                spec.severity = 1;
            }
        }

        if (const std::vector<Member>* fields = record.as_object(); fields != nullptr) {
            for (const Member& field : *fields) {
                if (field.key == "provenance") {
                    continue;
                }
                if (!known_key(kObservationKeys, field.key)) {
                    ++report.unknown_fields;
                }
            }
        }

        nodes.push_back(std::move(spec));
        ++report.accepted;
    }

    return nodes;
}

KernelReading ingest_kernel(const Value& kernel, Freshness freshness, bool display_present,
                            IngestReport& report) {
    KernelReading reading{};
    reading.display_present = display_present;
    reading.freshness = freshness;

    if (kernel.as_object() == nullptr) {
        report.note(DefectKind::NotAnObject, "", "the kernel snapshot is not an object");
        // Everything stays unavailable, which is the cautious reading and exactly what
        // the budget governor treats as "no evidence of headroom".
        return reading;
    }

    if (const Value* version = kernel.find("schema_version"); version != nullptr) {
        if (const std::optional<double> number = version->as_number(); number.has_value()) {
            report.schema_version = number;
            if (*number > kKnownKernelSchema) {
                report.newer_schema = true;
                report.note(DefectKind::NewerSchema, "schema_version",
                            "the daemon speaks a newer kernel schema than this display");
            }
        } else {
            report.note(DefectKind::WrongType, "schema_version", "expected a number");
        }
    }

    // Availability is an internally tagged enum upstream: {"state": "available"} or
    // {"state": "unavailable", "reason": ...}. Anything else is treated as unavailable,
    // because the whole purpose of the flag is to say when a reading may be believed.
    const auto is_available = [&](const char* key) {
        const Value* field = kernel.find(key);
        if (field == nullptr) {
            report.note(DefectKind::MissingField, key, "expected an availability object");
            return false;
        }
        const Value* state = field->find("state");
        if (state == nullptr) {
            report.note(DefectKind::MissingField, std::string{key} + ".state",
                        "expected a string");
            return false;
        }
        const std::string* text = state->as_string();
        if (text == nullptr) {
            report.note(DefectKind::WrongType, std::string{key} + ".state", "expected a string");
            return false;
        }
        if (*text == "available") {
            return true;
        }
        if (*text != "unavailable") {
            report.note(DefectKind::UnknownEnum, std::string{key} + ".state",
                        "unrecognised availability: " + *text);
        }
        return false;
    };

    if (is_available("pressure_availability")) {
        reading.pressure_available = true;
        if (const Value* verdict = kernel.find("verdict"); verdict != nullptr) {
            if (const std::string* label = verdict->as_string(); label != nullptr) {
                if (*label == "idle") {
                    reading.verdict = PressureVerdict::Idle;
                } else if (*label == "light") {
                    reading.verdict = PressureVerdict::Light;
                } else if (*label == "contended") {
                    reading.verdict = PressureVerdict::Contended;
                } else if (*label == "saturated") {
                    reading.verdict = PressureVerdict::Saturated;
                } else {
                    // Left empty on purpose. An unknown verdict from a newer daemon
                    // must not be guessed at, and the governor already treats an
                    // available sensor with no value as no evidence at all.
                    report.note(DefectKind::UnknownEnum, "verdict",
                                "unrecognised pressure verdict: " + *label);
                }
            } else {
                report.note(DefectKind::WrongType, "verdict", "expected a string");
            }
        } else {
            report.note(DefectKind::MissingField, "verdict",
                        "pressure is available but carries no verdict");
        }
    }

    if (is_available("thermal_availability")) {
        reading.thermal_available = true;
        if (const Value* thermal = kernel.find("thermal_state"); thermal != nullptr) {
            if (const std::string* label = thermal->as_string(); label != nullptr) {
                if (*label == "unknown") {
                    reading.thermal = ThermalState::Unknown;
                } else if (*label == "nominal") {
                    reading.thermal = ThermalState::Nominal;
                } else if (*label == "warm") {
                    reading.thermal = ThermalState::Warm;
                } else if (*label == "throttling") {
                    reading.thermal = ThermalState::Throttling;
                } else if (*label == "critical") {
                    reading.thermal = ThermalState::Critical;
                } else {
                    report.note(DefectKind::UnknownEnum, "thermal_state",
                                "unrecognised thermal state: " + *label);
                }
            } else {
                report.note(DefectKind::WrongType, "thermal_state", "expected a string");
            }
        } else {
            report.note(DefectKind::MissingField, "thermal_state",
                        "thermal is available but carries no state");
        }
    }

    return reading;
}

}  // namespace archie::visual
