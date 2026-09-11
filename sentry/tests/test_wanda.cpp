// Wanda — the rules that keep a social-inference mode from becoming an authority.
//
// The failure this whole module guards against has a shape: two people are seen together
// often enough, a number crosses a threshold, and a system starts holding a record of a
// friendship nobody declared. Every case here is one step on that path, blocked.

#include "harness.hpp"

#include "sentry/identity/wanda.hpp"

using namespace sentry::identity;
using namespace sentry::identity::wanda;
using sentry::policy::ConsentEvidence;
using sentry::policy::ConsentLease;
using sentry::policy::ConsentScope;
using sentry::policy::OperatorPolicy;
using sentry::world::EpistemicState;
using sentry::world::TimePoint;

namespace {

TimePoint at(std::int64_t ms) { return TimePoint{ms}; }

constexpr std::int64_t kDay = 24LL * 60LL * 60LL * 1000LL;

SubjectRecord person(std::string id, SubjectKind kind = SubjectKind::ConsentedPerson) {
    SubjectRecord subject;
    subject.subject_id = std::move(id);
    subject.kind = kind;
    subject.display_label = subject.subject_id;
    return subject;
}

ConsentLease lease(std::string id, std::vector<ConsentScope> scopes) {
    ConsentLease granted;
    granted.subject_id = std::move(id);
    granted.scopes = std::move(scopes);
    granted.valid_from = at(0);
    granted.evidence = ConsentEvidence::SubjectSigned;
    return granted;
}

ConsentLease identifiable(std::string id) {
    return lease(std::move(id), {ConsentScope::CameraIdentity});
}

WandaPolicy enabled() {
    WandaPolicy policy;
    policy.social_inference_enabled = true;
    return policy;
}

CoPresence seen(int encounters, int places, int days) {
    CoPresence evidence;
    evidence.encounters = encounters;
    evidence.distinct_places = places;
    evidence.distinct_days = days;
    return evidence;
}

// The ordinary case: two people the owner knows, both identifiable, seen about.
std::optional<SocialHypothesis> ordinary(const CoPresence& evidence,
                                         const WandaPolicy& policy = enabled()) {
    return propose(policy, person("nelson", SubjectKind::Owner), person("rafa"),
                   identifiable("nelson"), identifiable("rafa"), evidence, true, true,
                   at(1000));
}

void deterministic_and_never_certain() {
    CASE("the same evidence always produces the same number");
    // Determinism is the property that lets this sit above the models. It is asserted on
    // exact values rather than on a tolerance, because a score that drifts between runs
    // would escalate on one node and not on another from identical evidence.
    CHECK(score(seen(3, 3, 3)).permille() == score(seen(3, 3, 3)).permille());
    CHECK(score(seen(3, 3, 3)).permille() == 30 * 3 + 90 * 3 + 80 * 3);
    CHECK(score(seen(0, 0, 0)).permille() == 0);

    CASE("certainty is not available to Wanda");
    // A social-inference mode that could reach 1.0 would be making a claim about the
    // world rather than summarising evidence.
    CHECK(score(seen(1000, 1000, 1000)).permille() == Confidence::kCeilingPermille);
    CHECK(score(seen(1000, 1000, 1000)).value() < 1.0);

    CASE("more evidence never lowers the score");
    // A scoring function that could fall as evidence accumulated would be impossible to
    // explain to the person being asked about.
    int previous = 0;
    for (int n = 0; n <= 12; ++n) {
        const int now = score(seen(n, n, n)).permille();
        CHECK(now >= previous);
        previous = now;
    }

    CASE("variety counts for more than volume");
    // Sixty sightings in one lobby on one day is a shared commute. Three sightings in
    // three places on three days is an acquaintance, and the score has to know that.
    CHECK(score(seen(3, 3, 3)) > score(seen(20, 1, 1)));

    CASE("repeated sightings stop counting once they stop saying anything new");
    // The same fact observed again is not a second fact. Without this, volume alone
    // reaches any threshold given a long enough corridor.
    CHECK(score(seen(4, 1, 1)) == score(seen(400, 1, 1)));
    CHECK(score(seen(3, 1, 1)) < score(seen(4, 1, 1)));

    CASE("negative counts are treated as absent, not as evidence against");
    CHECK(score(seen(-5, -5, -5)).permille() == 0);
    CHECK(score(seen(3, -1, 0)).permille() == 30 * 3);
}

void what_wanda_may_not_touch() {
    const CoPresence evidence = seen(5, 4, 4);

    CASE("an anonymous track never acquires a relationship");
    // An edge to a track is how a track stops being anonymous, which is the back door
    // around every consent rule in this subsystem.
    CHECK(refuse(enabled(), person("nelson", SubjectKind::Owner),
                 person("track-7", SubjectKind::AnonymousTrack), identifiable("nelson"),
                 identifiable("track-7"), evidence, true, true, at(1000)) ==
          WandaRefusal::EndpointNotResolvable);

    CASE("being in the public record is not consent to be socially mapped");
    CHECK(refuse(enabled(), person("nelson", SubjectKind::Owner),
                 person("reporter", SubjectKind::PublicFigure), identifiable("nelson"),
                 identifiable("reporter"), evidence, true, true, at(1000)) ==
          WandaRefusal::EndpointNotResolvable);

    CASE("someone who never consented does not become a node by being seen");
    CHECK(refuse(enabled(), person("nelson", SubjectKind::Owner), person("stranger"),
                 identifiable("nelson"), std::nullopt, evidence, true, true, at(1000)) ==
          WandaRefusal::EndpointHasNoIdentityConsent);

    CASE("a lease that does not cover identity does not cover being identified");
    // Consenting to have your location read is not consenting to be resolved from a
    // camera. The scopes are deliberately not ordered so one cannot imply another.
    CHECK(refuse(enabled(), person("nelson", SubjectKind::Owner), person("rafa"),
                 identifiable("nelson"), lease("rafa", {ConsentScope::GpsLocation}),
                 evidence, true, true, at(1000)) ==
          WandaRefusal::EndpointHasNoIdentityConsent);

    CASE("a revoked lease stops Wanda immediately");
    ConsentLease withdrawn = identifiable("rafa");
    withdrawn.revocation = sentry::policy::RevocationState::Revoked;
    CHECK(refuse(enabled(), person("nelson", SubjectKind::Owner), person("rafa"),
                 identifiable("nelson"), withdrawn, evidence, true, true, at(1000)) ==
          WandaRefusal::EndpointHasNoIdentityConsent);

    CASE("Wanda does not map relationships between strangers");
    // The rule that keeps this from being a surveillance graph. Wanda may notice that
    // somebody the owner knows keeps appearing with a third person. It may not build a
    // web between two people whose only connection to the operator is the same city.
    CHECK(refuse(enabled(), person("passer-by-a"), person("passer-by-b"),
                 identifiable("passer-by-a"), identifiable("passer-by-b"), evidence, false,
                 false, at(1000)) == WandaRefusal::NeitherEndpointKnownToOwner);
    // One endpoint being known is enough — that is the lead worth having.
    CHECK(!refuse(enabled(), person("rafa"), person("ana"), identifiable("rafa"),
                  identifiable("ana"), evidence, true, false, at(1000))
               .has_value());

    CASE("a node with social inference switched off infers nothing");
    // Checked before consent, so an operator sees the switch they actually need rather
    // than going to look for a missing lease.
    WandaPolicy off;
    CHECK(refuse(off, person("nelson", SubjectKind::Owner), person("rafa"),
                 identifiable("nelson"), identifiable("rafa"), evidence, true, true,
                 at(1000)) == WandaRefusal::ForbiddenByOperatorPolicy);
    CHECK(!ordinary(evidence, off).has_value());

    CASE("nothing observed is not weak evidence");
    CHECK(refuse(enabled(), person("nelson", SubjectKind::Owner), person("rafa"),
                 identifiable("nelson"), identifiable("rafa"), seen(0, 0, 0), true, true,
                 at(1000)) == WandaRefusal::NoEvidence);

    CASE("nobody has a relationship with themselves");
    CHECK(refuse(enabled(), person("rafa"), person("rafa"), identifiable("rafa"),
                 identifiable("rafa"), evidence, true, true, at(1000)) ==
          WandaRefusal::SelfHypothesis);
}

void a_proposal_is_a_question() {
    CASE("what Wanda produces is a hypothesis, and says so");
    const auto proposed = ordinary(seen(4, 3, 3));
    CHECK(proposed.has_value());
    CHECK(proposed->state == EpistemicState::Inferred);
    CHECK(proposed->from_id == "nelson");
    CHECK(proposed->to_id == "rafa");

    CASE("a hypothesis carries what would disprove it");
    // A proposal nobody can disprove is not a hypothesis, and the person reading it
    // deserves to know what to ask.
    CHECK(!proposed->falsifier.empty());

    CASE("Wanda proposes the weakest tie that fits");
    // Noticing two people together is evidence they know each other, not evidence of
    // how. Proposing `Family` from a corridor asserts something about a life nobody
    // described.
    CHECK(proposed->kind == RelationKind::Friend);

    CASE("evidence about people ages out");
    // Two colleagues who shared an office in 2024 may not have met since.
    CHECK(is_live(*proposed, at(1000)));
    CHECK(is_live(*proposed, at(1000 + 29 * kDay)));
    CHECK(!is_live(*proposed, at(1000 + 31 * kDay)));
}

void the_only_way_out_of_hypothesis_standing() {
    const auto proposed = ordinary(seen(4, 3, 3));
    CHECK(proposed.has_value());

    CASE("a participant can turn a hypothesis into an edge");
    const auto promoted = promote(*proposed, "rafa", at(2000));
    CHECK(promoted.has_value());

    CASE("and the resulting edge says a person declared it, not that a score crossed a line");
    // The edge exists afterwards because somebody said so. Wanda's part survives as a
    // note in the margin and nothing more.
    CHECK(promoted->basis == EdgeBasis::SubjectDeclared);
    CHECK(promoted->declarer_id == "rafa");

    CASE("co-presence is never the basis of an asserted edge");
    // Named and refused in `relationship.hpp` for every kind. Wanda must not be the
    // component that quietly reintroduces it.
    CHECK(promoted->basis != EdgeBasis::CoPresenceInference);
    CHECK(promoted->basis != EdgeBasis::ModelHypothesis);

    CASE("and the promoted edge passes the graph's own rule for personal ties");
    const auto refusal =
        refuse_edge(*promoted, person("nelson", SubjectKind::Owner), person("rafa"), "rafa");
    CHECK(!refusal.has_value());

    CASE("somebody it is not about cannot confirm it");
    // An operator declaring a friendship between two other people is describing lives
    // that did not ask.
    CHECK(!promote(*proposed, "someone-else", at(2000)).has_value());

    CASE("an expired hypothesis cannot be promoted");
    CHECK(!promote(*proposed, "rafa", at(1000 + 31 * kDay)).has_value());
}

void models_advise_and_do_not_decide() {
    const WandaPolicy policy = enabled();
    OperatorPolicy allows_export;
    allows_export.cloud_export_enabled = true;
    OperatorPolicy no_export;

    auto with = [&](int permille) {
        SocialHypothesis hypothesis;
        hypothesis.from_id = "nelson";
        hypothesis.to_id = "rafa";
        hypothesis.confidence = Confidence::from_permille(permille);
        hypothesis.expires_at = at(100 * kDay);
        return hypothesis;
    };

    CASE("when Wanda has decided, no model is asked");
    // A settled question sent to a generative model comes back with a confident opinion
    // it had no need of. That is the mechanism by which hallucinations enter a system
    // that was working.
    CHECK(escalate(policy, allows_export, with(100), identifiable("nelson"),
                   identifiable("rafa"), at(1000))
              .tier == ModelTier::NotNeeded);
    CHECK(escalate(policy, allows_export, with(900), identifiable("nelson"),
                   identifiable("rafa"), at(1000))
              .tier == ModelTier::NotNeeded);

    CASE("only genuine ambiguity reaches a model");
    const auto ambiguous =
        escalate(policy, allows_export,
                 with(500), lease("nelson", {ConsentScope::CloudProcessing}),
                 lease("rafa", {ConsentScope::CloudProcessing}), at(1000));
    CHECK(ambiguous.tier == ModelTier::FrontierPermitted);
    CHECK(!ambiguous.reason.empty());

    CASE("a node that exports nothing keeps the question at home");
    CHECK(escalate(policy, no_export, with(500),
                   lease("nelson", {ConsentScope::CloudProcessing}),
                   lease("rafa", {ConsentScope::CloudProcessing}), at(1000))
              .tier == ModelTier::LocalOnly);

    CASE("asking about two people exports both, so both must have agreed");
    // The case that a per-subject check would miss: Nelson consenting to cloud
    // processing does not authorise sending a question that is also about Rafa.
    CHECK(escalate(policy, allows_export, with(500),
                   lease("nelson", {ConsentScope::CloudProcessing}), identifiable("rafa"),
                   at(1000))
              .tier == ModelTier::LocalOnly);

    CASE("an expired hypothesis is not worth a model's time");
    CHECK(escalate(policy, allows_export, with(500),
                   lease("nelson", {ConsentScope::CloudProcessing}),
                   lease("rafa", {ConsentScope::CloudProcessing}), at(200 * kDay))
              .tier == ModelTier::NotNeeded);

    CASE("a model may move the number inside the band and never outside it");
    // The whole point of putting the analytic above the models: a generative system that
    // could drive a social claim to near-certainty has become the authority.
    const SocialHypothesis before = with(500);
    CHECK(absorb(policy, before, Confidence::from_permille(950)).confidence.permille() ==
          policy.ambiguous_to_permille);
    CHECK(absorb(policy, before, Confidence::from_permille(0)).confidence.permille() ==
          policy.ambiguous_from_permille);
    CHECK(absorb(policy, before, Confidence::from_permille(620)).confidence.permille() == 620);

    CASE("and whatever a model claims about itself, the result is still inferred");
    SocialHypothesis dressed_up = with(500);
    dressed_up.state = EpistemicState::Verified;
    CHECK(absorb(policy, dressed_up, Confidence::from_permille(600)).state ==
          EpistemicState::Inferred);
}

}  // namespace

int main() {
    deterministic_and_never_certain();
    what_wanda_may_not_touch();
    a_proposal_is_a_question();
    the_only_way_out_of_hypothesis_standing();
    models_advise_and_do_not_decide();
    return sentry::test::report();
}
