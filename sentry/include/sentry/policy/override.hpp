// Owner override — the operator's authority over their own node, made explicit.
//
// RFC: docs/RFC-008-SENTRY-WORLD-MODEL.md §Consent
//
// A refusal the owner cannot argue with is a system that has decided it knows better than
// the person who runs it. That is the wrong shape for a machine that lives in someone's
// house. So every refusal in Sentry that is a *permission* question can be lifted by the
// owner, deliberately and on the record.
//
// The record is the whole point, and it is not a formality. An override that leaves no
// trace is indistinguishable from a bug: six months later nobody can tell whether Archie
// resolved an identity because a lease existed or because someone waved it through, and
// the difference is exactly what an operator would need to answer for.
//
// # Three kinds of refusal, and only one of them is about permission
//
// **Permission.** No lease, scope not granted, policy forbids it on this node. These are
// the owner's to lift. `LiftedOnTheRecord`.
//
// **Fact.** A self-edge, an edge to an anonymous track, a sensor that is not authorized.
// These are not refusals of permission — there is nothing to permit. An anonymous track
// has no identity to attach a name to; overriding does not conjure one, it just writes
// something false. The honest answer is that the override does not apply, and the fix is
// to supply the missing thing. `NotAPermissionQuestion`.
//
// **Epistemic integrity.** A generative model asserting into world state. This is the one
// refusal that stands against the owner too, and it stands *for* them: if Archie's guesses
// can be stamped as verified, the owner permanently loses the ability to tell what Archie
// actually knows from what it made up. That is not a restriction on the operator's
// authority — it is the thing that keeps the operator's authority worth having.
// `WouldCorruptWhatArchieKnows`.
//
// The owner can still act on a guess. They read it, they agree, and they declare it
// themselves — at which point the claim rests on a person's judgement rather than a
// model's confidence, which is exactly the distinction worth preserving.

#pragma once

#include <string>
#include <string_view>

#include "sentry/policy/consent.hpp"

namespace sentry::policy {

// The owner exercising their authority, deliberately.
//
// `justification` is required and checked. Not bureaucracy: an override with no stated
// reason is one nobody — including the owner, later — can evaluate, and the most common
// reader of this field is the person who wrote it trying to remember why.
struct OwnerOverride {
    std::string owner_id{};
    std::string justification{};
    TimePoint at{};

    // Whether this override is well-formed enough to act on.
    [[nodiscard]] bool is_stated() const noexcept {
        return !owner_id.empty() && !justification.empty();
    }
};

// What an override does to a particular refusal.
enum class OverrideVerdict {
    // The owner's authority covers this. Proceed, and write a record naming them.
    LiftedOnTheRecord,
    // Not a permission question. Overriding would not make the operation possible, only
    // untrue — the missing thing has to be supplied instead.
    NotAPermissionQuestion,
    // Stands against the owner too, and stands for them: lifting it would destroy the
    // owner's ability to tell what Archie knows from what it guessed.
    WouldCorruptWhatArchieKnows,
    // The override itself is incomplete — no owner, or no stated reason.
    OverrideNotStated,
};

[[nodiscard]] std::string_view name(OverrideVerdict verdict) noexcept;
[[nodiscard]] std::string_view describe(OverrideVerdict verdict) noexcept;

// Whether the owner can lift a consent refusal.
//
// Almost all of them: every refusal here is about whether permission exists, and
// permission on this node is the owner's to give. `SensorNotAuthorized` is the exception,
// and it is a factual one — the camera is not reachable or not approved, and saying "do it
// anyway" does not produce a frame.
[[nodiscard]] OverrideVerdict can_override(ConsentRefusal refusal,
                                           const OwnerOverride& override_) noexcept;

// A lease the owner creates on their own authority.
//
// This is not a loophole around consent; it is what `ConsentEvidence::OperatorAttested`
// has meant since the beginning — the operator states that they obtained permission out of
// band, and puts their name to it. The lease it produces is a normal lease and is checked
// like any other, which means the operator's own policy still has to accept attested
// evidence for it to work.
[[nodiscard]] ConsentLease attested_by_owner(const OwnerOverride& override_,
                                             std::string subject_id,
                                             std::vector<ConsentScope> scopes) noexcept;

// A policy with every dial turned on.
//
// The owner's node, the owner's rules. Provided as a named function rather than left for
// each caller to assemble, so that "the operator has enabled everything" is one recognised
// configuration instead of six scattered booleans that might disagree.
[[nodiscard]] OperatorPolicy unrestricted_policy() noexcept;

}  // namespace sentry::policy
