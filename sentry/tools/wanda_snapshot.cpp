// `sentry-wanda-snapshot` — draw what Wanda actually decided.
//
// Every edge, number and refusal in the output comes from calling the real functions in
// `wanda.hpp` on the graph below. Nothing is drawn from a table of expected results: if
// the scoring changes this picture changes with it, and if Wanda refuses a pair the
// picture says so rather than quietly omitting them.
//
// The people are simulated and the image says so on its face. That is not decoration — a
// diagram of a social graph that does not state whether it describes real people is the
// most misleading artefact this repository could produce.
//
// # Why the labels are not on the lines
//
// They were, for two drafts. The first ran the strongest hypothesis straight through the
// owner's circle with its confidence printed on top of him, which reads as a fact about
// him. The second fixed that and still landed two refusal labels on the wrong people,
// because a chord's midpoint collapses toward the centre as its endpoints approach
// opposite sides of a ring. Text on a line is unambiguous only when nothing else is near
// the line, and in a social graph something always is. So the lines carry a number and
// the panel carries the words.

#include <cmath>
#include <cstdio>
#include <map>
#include <string>
#include <utility>
#include <vector>

#include "sentry/identity/relationship.hpp"
#include "sentry/identity/wanda.hpp"

using namespace sentry::identity;
using namespace sentry::identity::wanda;
using sentry::policy::ConsentEvidence;
using sentry::policy::ConsentLease;
using sentry::policy::ConsentScope;
using sentry::policy::OperatorPolicy;
using sentry::world::TimePoint;

namespace {

constexpr double kPi = 3.14159265358979;

TimePoint at(std::int64_t ms) { return TimePoint{ms}; }

struct Person {
    std::string id;
    std::string label;
    SubjectKind kind;
    std::vector<ConsentScope> scopes;  // empty means no lease at all
    double angle = 0.0;                // position on the ring, degrees
    bool on_ring = true;               // the owner sits at the centre
    double x = 0.0;
    double y = 0.0;
};

SubjectRecord record_for(const Person& person) {
    SubjectRecord subject;
    subject.subject_id = person.id;
    subject.kind = person.kind;
    subject.display_label = person.label;
    return subject;
}

std::optional<ConsentLease> lease_for(const Person& person) {
    if (person.scopes.empty()) {
        return std::nullopt;
    }
    ConsentLease lease;
    lease.subject_id = person.id;
    lease.scopes = person.scopes;
    lease.valid_from = at(0);
    lease.evidence = person.kind == SubjectKind::Owner ? ConsentEvidence::SelfConsent
                                                       : ConsentEvidence::SubjectSigned;
    return lease;
}

std::string_view kind_label(SubjectKind kind) {
    switch (kind) {
        case SubjectKind::Owner:
            return "owner";
        case SubjectKind::ConsentedPerson:
            return "consented";
        case SubjectKind::PublicFigure:
            return "public figure";
        case SubjectKind::AnonymousTrack:
            return "anonymous track";
    }
    return "unknown";
}

// Colour carries kind only. Confidence is carried by the number and the stroke width,
// never by hue alone: a graph that encodes certainty in colour is unreadable to a good
// fraction of the people who would need to read it.
std::string_view kind_colour(SubjectKind kind) {
    switch (kind) {
        case SubjectKind::Owner:
            return "#4dd6c1";
        case SubjectKind::ConsentedPerson:
            return "#7aa7d8";
        case SubjectKind::PublicFigure:
            return "#c8b06a";
        case SubjectKind::AnonymousTrack:
            return "#5c6570";
    }
    return "#5c6570";
}

}  // namespace

int main() {
    const double cx = 380.0;
    const double cy = 400.0;
    const double ring = 200.0;
    const auto rx = [&](double degrees) { return cx + ring * std::cos(degrees * kPi / 180.0); };
    const auto ry = [&](double degrees) { return cy - ring * std::sin(degrees * kPi / 180.0); };

    // --- the simulated node -------------------------------------------------------
    //
    // Nelson is the owner and consents for himself. Everyone else carries the lease the
    // scenario gives them, including the two who carry none — they are here so the
    // picture shows what Wanda does about them, which is nothing.
    //
    // Laid out on a ring so a declared edge runs centre-to-rim and can never pass through
    // a third person.
    std::vector<Person> people = {
        {"nelson", "Nelson · Penta", SubjectKind::Owner,
         {ConsentScope::CameraIdentity, ConsentScope::BiometricIdentity}, 0.0, false, cx, cy},
        {"rafa", "Rafa", SubjectKind::ConsentedPerson, {ConsentScope::CameraIdentity}, 126.0,
         true, 0, 0},
        {"gabriel", "Gabriel", SubjectKind::ConsentedPerson, {ConsentScope::CameraIdentity}, 54.0,
         true, 0, 0},
        {"ana", "Ana", SubjectKind::ConsentedPerson,
         {ConsentScope::CameraIdentity, ConsentScope::CloudProcessing}, 342.0, true, 0, 0},
        {"track-7", "track-7", SubjectKind::AnonymousTrack, {}, 270.0, true, 0, 0},
        {"vecino", "Neighbour", SubjectKind::ConsentedPerson, {ConsentScope::GpsLocation}, 198.0,
         true, 0, 0},
    };
    for (Person& person : people) {
        if (person.on_ring) {
            person.x = rx(person.angle);
            person.y = ry(person.angle);
        }
    }

    std::map<std::string, Person> by_id;
    for (const Person& person : people) {
        by_id[person.id] = person;
    }

    // Ties the owner has declared. These are assertions, and they are what makes each
    // person reachable — the justification for holding their record at all.
    RelationGraph graph{"nelson"};
    const auto declare = [&](const char* to, RelationKind kind) {
        RelationEdge edge;
        edge.from_id = "nelson";
        edge.to_id = to;
        edge.kind = kind;
        edge.basis = EdgeBasis::OwnerDeclared;
        edge.declarer_id = "nelson";
        edge.source_note = "declared by the owner";
        graph.assert_edge(edge);
    };
    declare("rafa", RelationKind::Friend);
    declare("ana", RelationKind::Colleague);
    declare("vecino", RelationKind::AuthorizedGuest);

    // What the sensors and ingest observed, as counts. Gabriel is deliberately not
    // declared: he is the third person who keeps turning up, which is the only situation
    // Wanda exists for.
    struct Observed {
        std::string a;
        std::string b;
        CoPresence evidence;
    };
    const std::vector<Observed> observed = {
        {"rafa", "gabriel", {6, 4, 5}},     // varied: different places, different days
        {"ana", "gabriel", {9, 1, 1}},      // a shared commute: volume without variety
        {"nelson", "ana", {3, 2, 2}},       // moderate
        {"vecino", "gabriel", {5, 3, 3}},   // the neighbour never consented to be identified
        {"gabriel", "track-7", {8, 5, 5}},  // an anonymous track
    };

    WandaPolicy policy;
    policy.social_inference_enabled = true;
    OperatorPolicy operator_policy;
    operator_policy.cloud_export_enabled = true;
    const TimePoint now = at(1757000000000LL);

    struct Drawn {
        int index = 0;
        std::string a;
        std::string b;
        CoPresence evidence;
        int permille = 0;
        std::string tier;
        std::string refusal;
    };
    std::vector<Drawn> results;

    int index = 0;
    for (const Observed& pair : observed) {
        const Person& a = by_id[pair.a];
        const Person& b = by_id[pair.b];
        const bool a_known = graph.degree(a.id).has_value();
        const bool b_known = graph.degree(b.id).has_value();

        Drawn drawn;
        drawn.index = ++index;
        drawn.a = a.id;
        drawn.b = b.id;
        drawn.evidence = pair.evidence;

        const auto refusal = refuse(policy, record_for(a), record_for(b), lease_for(a),
                                    lease_for(b), pair.evidence, a_known, b_known, now);
        if (refusal.has_value()) {
            drawn.refusal = std::string(describe(*refusal));
            results.push_back(drawn);
            continue;
        }
        const auto hypothesis = propose(policy, record_for(a), record_for(b), lease_for(a),
                                        lease_for(b), pair.evidence, a_known, b_known, now);
        if (hypothesis.has_value()) {
            drawn.permille = hypothesis->confidence.permille();
            drawn.tier = std::string(name(escalate(policy, operator_policy, *hypothesis,
                                                   lease_for(a), lease_for(b), now)
                                              .tier));
        }
        results.push_back(drawn);
    }

    // --- draw ---------------------------------------------------------------------
    std::string svg;
    char buffer[1200];
    const auto emit = [&](const char* format, auto... args) {
        std::snprintf(buffer, sizeof(buffer), format, args...);
        svg += buffer;
    };

    svg +=
        R"(<svg xmlns="http://www.w3.org/2000/svg" width="1240" height="700" viewBox="0 0 1240 700">)";
    svg += R"(<rect width="1240" height="700" fill="#0a0f16"/>)";
    svg +=
        R"(<text x="34" y="44" fill="#e6edf5" font-family="monospace" font-size="18" letter-spacing="2">WANDA &#183; ANALYTICAL SOCIAL NETWORK</text>)";
    svg +=
        R"(<text x="34" y="68" fill="#f0a04b" font-family="monospace" font-size="12">SIMULATED PEOPLE &#183; no real person is described here</text>)";
    svg +=
        R"(<text x="34" y="88" fill="#7d8896" font-family="monospace" font-size="11">deterministic &#183; nothing sampled, nothing generated &#183; every number is computed</text>)";

    // The apex of an inferred edge, placed by angle. A chord's midpoint collapses toward
    // the centre as its endpoints approach opposite sides, so angle is what stays
    // meaningful however far apart two people are.
    const auto apex_of = [&](const Person& a, const Person& b) {
        double angle = 0.0;
        double radius = 0.0;
        if (a.on_ring && b.on_ring) {
            double separation = b.angle - a.angle;
            while (separation > 180.0) {
                separation -= 360.0;
            }
            while (separation < -180.0) {
                separation += 360.0;
            }
            angle = a.angle + separation / 2.0;
            radius = ring * (1.15 + 0.28 * std::fabs(separation) / 180.0);
        } else {
            const Person& rim = a.on_ring ? a : b;
            angle = rim.angle + 17.0;
            radius = ring * 0.58;
        }
        const double radians = angle * kPi / 180.0;
        return std::pair<double, double>{cx + radius * std::cos(radians),
                                         cy - radius * std::sin(radians)};
    };

    // Asserted edges: straight, solid, centre to rim.
    for (const RelationEdge& edge : graph.asserted()) {
        const Person& a = by_id[edge.from_id];
        const Person& b = by_id[edge.to_id];
        emit(R"(<line x1="%.0f" y1="%.0f" x2="%.0f" y2="%.0f" stroke="#416180" stroke-width="2"/>)",
             a.x, a.y, b.x, b.y);
        emit(R"(<text x="%.0f" y="%.0f" fill="#8fa6bb" font-family="monospace" font-size="10" text-anchor="middle">%s</text>)",
             a.x + (b.x - a.x) * 0.60, a.y + (b.y - a.y) * 0.60 - 7,
             std::string(name(edge.kind)).c_str());
    }

    // Inferred edges: arcs bowing outward, carrying a number and nothing else.
    for (const Drawn& drawn : results) {
        const Person& a = by_id[drawn.a];
        const Person& b = by_id[drawn.b];
        const std::pair<double, double> apex = apex_of(a, b);
        const double control_x = 2.0 * apex.first - (a.x + b.x) / 2.0;
        const double control_y = 2.0 * apex.second - (a.y + b.y) / 2.0;
        const bool refused = !drawn.refusal.empty();
        const double width =
            refused ? 1.4 : 1.0 + 3.0 * (static_cast<double>(drawn.permille) / 950.0);
        emit(R"(<path d="M %.0f %.0f Q %.0f %.0f %.0f %.0f" fill="none" stroke="%s" stroke-width="%.2f" stroke-dasharray="%s"/>)",
             a.x, a.y, control_x, control_y, b.x, b.y, refused ? "#6b3a48" : "#d9a441", width,
             refused ? "2 6" : "7 5");
        emit(R"(<circle cx="%.0f" cy="%.0f" r="11" fill="#0a0f16" stroke="%s" stroke-width="1.4"/>)",
             apex.first, apex.second, refused ? "#a8465c" : "#d9a441");
        emit(R"(<text x="%.0f" y="%.0f" fill="%s" font-family="monospace" font-size="11" text-anchor="middle">%d</text>)",
             apex.first, apex.second + 4, refused ? "#e08098" : "#f0c368", drawn.index);
    }

    for (const Person& person : people) {
        const double radius = person.kind == SubjectKind::Owner ? 27.0 : 21.0;
        emit(R"(<circle cx="%.0f" cy="%.0f" r="%.0f" fill="#0f1720" stroke="%s" stroke-width="2"/>)",
             person.x, person.y, radius, std::string(kind_colour(person.kind)).c_str());
        emit(R"(<text x="%.0f" y="%.0f" fill="#e6edf5" font-family="monospace" font-size="12" text-anchor="middle">%s</text>)",
             person.x, person.y + radius + 18, person.label.c_str());
        emit(R"(<text x="%.0f" y="%.0f" fill="#7d8896" font-family="monospace" font-size="9" text-anchor="middle">%s</text>)",
             person.x, person.y + radius + 31, std::string(kind_label(person.kind)).c_str());
        const auto degree = graph.degree(person.id);
        const std::string reach =
            degree.has_value() ? "degree " + std::to_string(*degree) : "no path to owner";
        emit(R"(<text x="%.0f" y="%.0f" fill="#5d6a78" font-family="monospace" font-size="9" text-anchor="middle">%s</text>)",
             person.x, person.y + radius + 43, reach.c_str());
    }

    // --- the panel: the words, where nothing can land on top of them ----------------
    const double px = 700.0;
    emit(R"(<line x1="%.0f" y1="118" x2="%.0f" y2="668" stroke="#1d2730" stroke-width="1"/>)", px,
         px);
    emit(R"(<text x="%.0f" y="150" fill="#e6edf5" font-family="monospace" font-size="12" letter-spacing="1">WHAT WANDA DECIDED</text>)",
         px + 26);

    double row = 188.0;
    for (const Drawn& drawn : results) {
        const bool refused = !drawn.refusal.empty();
        emit(R"(<circle cx="%.0f" cy="%.0f" r="11" fill="#0a0f16" stroke="%s" stroke-width="1.4"/>)",
             px + 36, row - 4, refused ? "#a8465c" : "#d9a441");
        emit(R"(<text x="%.0f" y="%.0f" fill="%s" font-family="monospace" font-size="11" text-anchor="middle">%d</text>)",
             px + 36, row, refused ? "#e08098" : "#f0c368", drawn.index);
        emit(R"(<text x="%.0f" y="%.0f" fill="#dbe4ee" font-family="monospace" font-size="12">%s &#8212; %s</text>)",
             px + 58, row, by_id[drawn.a].label.c_str(), by_id[drawn.b].label.c_str());

        if (refused) {
            emit(R"(<text x="%.0f" y="%.0f" fill="#c2566e" font-family="monospace" font-size="10">refused: %s</text>)",
                 px + 58, row + 16, drawn.refusal.c_str());
        } else {
            emit(R"(<text x="%.0f" y="%.0f" fill="#e8c27a" font-family="monospace" font-size="12" text-anchor="end">%.2f</text>)",
                 px + 500, row, static_cast<double>(drawn.permille) / 1000.0);
            emit(R"(<text x="%.0f" y="%.0f" fill="#7d8896" font-family="monospace" font-size="10">%d encounters &#183; %d places &#183; %d days &#8594; model: %s</text>)",
                 px + 58, row + 16, drawn.evidence.encounters, drawn.evidence.distinct_places,
                 drawn.evidence.distinct_days, drawn.tier.c_str());
        }
        row += 54.0;
    }

    row += 14.0;
    emit(R"(<line x1="%.0f" y1="%.0f" x2="1206" y2="%.0f" stroke="#1d2730" stroke-width="1"/>)",
         px + 26, row - 20, row - 20);
    const char* notes[] = {
        "solid = declared by a person, and asserted",
        "dashed = a Wanda hypothesis; never an asserted edge",
        "dotted = refused before any evidence was weighed",
        "confidence caps at 0.95: certainty is not available here",
    };
    for (const char* note : notes) {
        emit(R"(<text x="%.0f" y="%.0f" fill="#6c7783" font-family="monospace" font-size="10">%s</text>)",
             px + 26, row, note);
        row += 17.0;
    }

    svg += "</svg>";
    std::printf("%s", svg.c_str());

    // The same decisions in words, on stderr, so the numbers can be checked without
    // opening an image.
    std::fprintf(stderr, "Wanda decisions (all computed, none tabulated):\n");
    for (const Drawn& drawn : results) {
        if (!drawn.refusal.empty()) {
            std::fprintf(stderr, "  %-9s %-9s  REFUSED  %s\n", drawn.a.c_str(), drawn.b.c_str(),
                         drawn.refusal.c_str());
        } else {
            std::fprintf(stderr, "  %-9s %-9s  hypothesis %.3f  model: %s\n", drawn.a.c_str(),
                         drawn.b.c_str(), static_cast<double>(drawn.permille) / 1000.0,
                         drawn.tier.c_str());
        }
    }
    for (const std::string& orphan : graph.orphans()) {
        std::fprintf(stderr, "  orphan (no reason to hold a record): %s\n", orphan.c_str());
    }
    return 0;
}
