// Open-ended profile properties, and the hole they would open if the key were trusted.
//
// A profile has to hold height, eye colour, hobbies, the year someone was met — anything —
// so the key space is open. An open key space beside a closed enum is a bypass with extra
// steps unless the key is classified rather than believed, and most of these cases are
// about that.

#include "harness.hpp"

#include "sentry/identity/override.hpp"
#include "sentry/identity/profile.hpp"

using namespace sentry::identity;
using sentry::policy::OverrideVerdict;
using sentry::policy::OwnerOverride;
using sentry::world::TimePoint;

namespace {

SubjectRecord person(std::string id, SubjectKind kind) {
    SubjectRecord s;
    s.subject_id = std::move(id);
    s.kind = kind;
    s.display_label = s.subject_id;
    return s;
}

Trait trait(std::string key, std::string value, TraitBasis basis) {
    Trait t;
    t.key = std::move(key);
    t.value = std::move(value);
    t.basis = basis;
    return t;
}

RelationEdge edge(std::string from, std::string to, RelationKind kind, EdgeBasis basis) {
    RelationEdge e;
    e.from_id = std::move(from);
    e.to_id = std::move(to);
    e.kind = kind;
    e.basis = basis;
    return e;
}

// ---------------------------------------------------------------- open keys

void a_profile_holds_anything() {
    CASE("the key space is genuinely open");

    // The point of the whole file: an enum cannot enumerate "anything", and a profile has
    // to hold whatever the owner finds worth recording.
    for (const auto* key : {"height_cm", "eye_colour", "hobbies", "skills", "favourite_band",
                            "met_in_year", "how_we_met", "shoe_size", "coffee_order"}) {
        const Trait t = trait(key, "value", TraitBasis::OwnerDeclared);
        CHECK_BECAUSE(t.effective_sensitivity() == Sensitivity::Personal,
                      "an unanticipated key lands on Personal, which is the safe side of "
                      "the only mistake this classification can make");
    }
}

void the_key_is_classified_not_believed() {
    CASE("an open key space is not a way around the closed one");

    // The bypass this prevents: writing trait{"home_address", ...} and walking straight
    // around AttributeClass::HomeAddress, with nothing noticing because the bag takes any
    // string.
    Trait sneaky = trait("home_address", "Calle Falsa 123", TraitBasis::OwnerDeclared);
    sneaky.declared_sensitivity = Sensitivity::PublicProfessional;

    CHECK_BECAUSE(sneaky.effective_sensitivity() == Sensitivity::Sensitive,
                  "a caller may declare a trait more sensitive than its key suggests, "
                  "never less");
}

void naming_conventions_do_not_defeat_the_classifier() {
    CASE("the key is normalised before it is matched");

    // A key space anyone can add to needs matching that does not depend on them having
    // guessed the house style.
    for (const auto* key : {"home_address", "homeAddress", "Home Address", "home-address",
                            "subject_home_address_line_1", "HOMEADDRESS"}) {
        CHECK_EQ(classify(key), Sensitivity::Sensitive);
    }
    for (const auto* key : {"phone_number", "mobileNumber", "whatsapp_number",
                            "date_of_birth", "birthPlace", "gps_coordinates",
                            "health_status", "national_id"}) {
        CHECK_EQ(classify(key), Sensitivity::Sensitive);
    }
}

void a_short_word_is_not_found_inside_a_longer_one() {
    CASE("age is inside last_message, and race is inside bracelet");

    // Found by running the chat ingester: `last_message` and `messages_exchanged` both came
    // back Sensitive, because a substring test finds "age" in "message". A profile where
    // the number of messages someone sent is classified as sensitive personal data is not
    // being careful, it is being useless — the owner learns to pass --all for everything,
    // and the redaction stops meaning anything.
    for (const auto* key : {"last_message", "messages_exchanged", "message_count",
                            "bracelet_colour", "incoming_calls", "package_tracking",
                            "average_reply_time", "language", "storage_notes", "manager"}) {
        CHECK_BECAUSE(classify(key) != Sensitivity::Sensitive,
                      "a short needle found inside an unrelated word makes the whole "
                      "classification noise");
    }

    // While the words themselves still classify.
    for (const auto* key : {"age", "date_of_birth", "bank_account", "annual_income",
                            "health", "religion"}) {
        CHECK_EQ(classify(key), Sensitivity::Sensitive);
    }
}

void an_overlapping_key_is_treated_as_the_stricter_one() {
    CASE("work address is an address");

    // The categories overlap, and the overlap is exactly where a wrong guess does damage.
    CHECK_EQ(classify("work_address"), Sensitivity::Sensitive);
    CHECK_EQ(classify("office_phone"), Sensitivity::Sensitive);

    // While the genuinely professional keys stay professional.
    for (const auto* key : {"job_title", "employer", "affiliation", "byline", "public_account"}) {
        CHECK_EQ(classify(key), Sensitivity::PublicProfessional);
    }
}

// ---------------------------------------------------------------- §40 on open keys

void a_public_figure_keeps_only_their_professional_record() {
    CASE("§40 reaches the open key space too");

    const auto reporter = person("reporter", SubjectKind::PublicFigure);

    CHECK_EQ(refuse_trait(reporter, trait("home_address", "x", TraitBasis::PublicRecord)),
             std::optional{AttributeRefusal::PrivateDataOnPublicFigure});

    // And personal-but-harmless is refused too. Being in the news does not put someone's
    // hobbies and eye colour into the public record either.
    CHECK_BECAUSE(refuse_trait(reporter, trait("hobbies", "climbing", TraitBasis::PublicRecord)) ==
                      AttributeRefusal::PrivateDataOnPublicFigure,
                  "the professional record is what is public about a public figure, and "
                  "that is the whole of it");

    // Their professional record is fine.
    CHECK(!refuse_trait(reporter, trait("employer", "El Pais", TraitBasis::PublicRecord)));
    CHECK(!refuse_trait(reporter, trait("byline", "...", TraitBasis::PublicRecord)));
}

void a_consented_person_may_hold_anything() {
    CASE("a friend's profile is not restricted");

    // This is the ordinary case the whole feature exists for: the owner recording what
    // they know about someone in their life.
    const auto rafa = person("rafa", SubjectKind::ConsentedPerson);
    for (const auto* key : {"age", "birthplace", "lives_in", "hobbies", "skills",
                            "eye_colour", "height_cm", "met_in_year"}) {
        CHECK(!refuse_trait(rafa, trait(key, "value", TraitBasis::OwnerDeclared)));
    }
}

void the_owner_overrides_a_public_figure_refusal() {
    CASE("even §40 yields to the owner, on the record");

    const auto reporter = person("reporter", SubjectKind::PublicFigure);
    const auto refusal = refuse_trait(reporter, trait("lives_in", "Madrid", TraitBasis::OwnerDeclared));
    CHECK(refusal.has_value());

    const OwnerOverride owner{"nelson", "researching this person deliberately", TimePoint{1}};
    if (refusal) {
        CHECK_EQ(can_override(*refusal, owner), OverrideVerdict::LiftedOnTheRecord);
    }
}

void a_track_holds_no_properties() {
    CASE("nothing may be hung on an anonymous track");

    const auto track = person("track-4", SubjectKind::AnonymousTrack);
    CHECK_EQ(refuse_trait(track, trait("eye_colour", "brown", TraitBasis::OwnerDeclared)),
             std::optional{AttributeRefusal::AnonymousTrackCannotCarryIdentity});
}

// ---------------------------------------------------------------- standing

void a_chat_reading_is_not_a_confirmed_fact() {
    CASE("where a property came from is part of the property");

    // "Rafa is 14" read out of a message is Archie's reading of a sentence. People
    // mistype, joke, and talk about third parties. Recording the basis is what lets a
    // profile show the difference rather than flattening both into a number that looks
    // equally certain.
    CHECK(trait("age", "14", TraitBasis::InferredFromMessages).is_hypothesis());
    CHECK(trait("age", "14", TraitBasis::ModelInferred).is_hypothesis());
    CHECK(!trait("age", "14", TraitBasis::SubjectDeclared).is_hypothesis());
    CHECK(!trait("age", "14", TraitBasis::OwnerDeclared).is_hypothesis());
}

void a_guess_never_shadows_something_confirmed() {
    CASE("asserted() skips hypotheses; any() does not");

    Profile rafa;
    rafa.subject = person("rafa", SubjectKind::ConsentedPerson);
    rafa.traits.push_back(trait("age", "14", TraitBasis::SubjectDeclared));
    rafa.traits.push_back(trait("age", "15", TraitBasis::InferredFromMessages));

    const auto* confirmed = rafa.asserted("age");
    CHECK(confirmed != nullptr);
    if (confirmed) {
        CHECK_BECAUSE(confirmed->value == "14",
                      "a chat-derived guess must not shadow what the person said "
                      "themselves, however recent the guess is");
    }

    const auto* latest = rafa.any("age");
    CHECK(latest != nullptr);
    if (latest) {
        CHECK_EQ(latest->value, std::string{"15"});
    }
}

void not_knowing_and_suspecting_do_not_read_the_same() {
    CASE("a profile with only a guess reports no asserted value");

    Profile rafa;
    rafa.subject = person("rafa", SubjectKind::ConsentedPerson);
    rafa.traits.push_back(trait("lives_in", "Madrid", TraitBasis::InferredFromMessages));

    CHECK_BECAUSE(rafa.asserted("lives_in") == nullptr,
                  "the caller has to ask for the guess explicitly; 'Archie does not know' "
                  "and 'Archie suspects' should not return the same thing");
    CHECK(rafa.any("lives_in") != nullptr);
}

void history_is_kept_rather_than_overwritten() {
    CASE("a flat list, because people change employers");

    Profile rafa;
    rafa.subject = person("rafa", SubjectKind::ConsentedPerson);
    rafa.traits.push_back(trait("employer", "first", TraitBasis::PublicRecord));
    rafa.traits.push_back(trait("employer", "second", TraitBasis::PublicRecord));

    CHECK_BECAUSE(rafa.all("employer").size() == 2,
                  "collapsing to one value per key would silently drop history that a "
                  "profile is precisely the place to keep");
    CHECK_EQ(rafa.asserted("employer")->value, std::string{"second"});
}

void a_redacted_view_is_one_call() {
    CASE("everything at or above a sensitivity");

    Profile rafa;
    rafa.subject = person("rafa", SubjectKind::ConsentedPerson);
    rafa.traits.push_back(trait("employer", "x", TraitBasis::PublicRecord));
    rafa.traits.push_back(trait("hobbies", "climbing", TraitBasis::OwnerDeclared));
    rafa.traits.push_back(trait("home_address", "y", TraitBasis::OwnerDeclared));

    CHECK_EQ(rafa.at_or_above(Sensitivity::Sensitive).size(), std::size_t{1});
    CHECK_EQ(rafa.at_or_above(Sensitivity::Personal).size(), std::size_t{2});
    CHECK_EQ(rafa.at_or_above(Sensitivity::PublicProfessional).size(), std::size_t{3});
}

// ---------------------------------------------------------------- mutuals

void friends_in_common_are_computed_from_asserted_edges() {
    CASE("amigos en comun");

    RelationGraph g{"nelson"};
    g.assert_edge(edge("nelson", "rafa", RelationKind::Friend, EdgeBasis::OwnerDeclared));
    g.assert_edge(edge("nelson", "ana", RelationKind::Friend, EdgeBasis::OwnerDeclared));
    g.assert_edge(edge("rafa", "ana", RelationKind::Friend, EdgeBasis::SubjectDeclared));
    g.assert_edge(edge("rafa", "beto", RelationKind::Friend, EdgeBasis::SubjectDeclared));

    // Nelson and Rafa both know Ana.
    const std::vector<std::string> expected{"ana"};
    CHECK_EQ(g.mutual("nelson", "rafa"), expected);

    // And neither counts as their own mutual acquaintance, despite the direct edge that
    // puts each in the other's neighbour list.
    const auto shared = g.mutual("nelson", "rafa");
    CHECK_BECAUSE(std::ranges::find(shared, "nelson") == shared.end() &&
                      std::ranges::find(shared, "rafa") == shared.end(),
                  "a direct edge is a relationship, not a mutual one; without this every "
                  "connected pair reports each other");
}

void a_guessed_edge_does_not_create_a_mutual_friend() {
    CASE("mutuals come from asserted edges only");

    RelationGraph g{"nelson"};
    g.assert_edge(edge("nelson", "ana", RelationKind::Friend, EdgeBasis::OwnerDeclared));
    g.hypothesise_edge(edge("rafa", "ana", RelationKind::Friend, EdgeBasis::ModelHypothesis));

    CHECK_BECAUSE(g.mutual("nelson", "rafa").empty(),
                  "a shared acquaintance that exists because a model guessed at one of the "
                  "two edges is not a shared acquaintance");
}

void the_graph_has_no_depth_limit() {
    CASE("infinite in the sense that matters: nothing caps the distance");

    RelationGraph g{"p0"};
    for (int i = 0; i < 500; ++i) {
        g.assert_edge(edge("p" + std::to_string(i), "p" + std::to_string(i + 1),
                           RelationKind::Friend, EdgeBasis::OwnerDeclared));
    }

    CHECK_EQ(g.degree("p500"), std::optional{std::size_t{500}});
    CHECK(g.path_to_owner("p500")->size() == 501);
    CHECK(g.orphans().empty());
}

}  // namespace

int main() {
    a_profile_holds_anything();
    the_key_is_classified_not_believed();
    naming_conventions_do_not_defeat_the_classifier();
    a_short_word_is_not_found_inside_a_longer_one();
    an_overlapping_key_is_treated_as_the_stricter_one();

    a_public_figure_keeps_only_their_professional_record();
    a_consented_person_may_hold_anything();
    the_owner_overrides_a_public_figure_refusal();
    a_track_holds_no_properties();

    a_chat_reading_is_not_a_confirmed_fact();
    a_guess_never_shadows_something_confirmed();
    not_knowing_and_suspecting_do_not_read_the_same();
    history_is_kept_rather_than_overwritten();
    a_redacted_view_is_one_call();

    friends_in_common_are_computed_from_asserted_edges();
    a_guessed_edge_does_not_create_a_mutual_friend();
    the_graph_has_no_depth_limit();

    return sentry::test::report();
}
