// Owner override for the identity layer.
//
// RFC: docs/RFC-008-SENTRY-WORLD-MODEL.md §Identity
//
// The defaults in `subject.hpp` and `relationship.hpp` are what the *automated* pipeline
// may do on its own: a detector, a fusion pass, a scheduled import. They are strict
// because that path runs unattended, at volume, and nobody reviews each result.
//
// This header is the other path. The owner is looking at one specific case, has a reason,
// and says yes. That is a different situation from a pipeline doing the same thing ten
// thousand times, and it gets a different answer.
//
// So: `refuse_edge` stays as it is and the pipeline keeps calling it. When the owner
// disagrees with a particular refusal, `can_override` says whether their authority reaches
// it, and the operation proceeds with a record naming them.
//
// # The one thing an override cannot do
//
// It cannot make a model's guess into something Archie verified. Not because the owner
// lacks the authority — because exercising it there would destroy the thing the authority
// is for. An owner who cannot distinguish what Archie observed from what Archie inferred
// has lost the ability to supervise it at all.
//
// There is no refusal in that case either. `promote` takes the hypothesis and re-attributes
// it to the owner, which is the honest version of agreeing with it: the claim now rests on
// a person's judgement, and says so.

#pragma once

#include "sentry/identity/relationship.hpp"
#include "sentry/identity/subject.hpp"
#include "sentry/policy/override.hpp"

namespace sentry::identity {

using sentry::policy::OverrideVerdict;
using sentry::policy::OwnerOverride;

// Whether the owner's authority reaches an edge refusal.
//
// Most of them, including the two that are strictest by default. A personal tie read out
// of the public record and a relationship noticed from co-presence are both refused to the
// pipeline and both available to the owner — the difference being that the owner is
// looking at one case and putting their name to it, rather than a job doing it at volume
// with nobody reading the output.
[[nodiscard]] OverrideVerdict can_override(EdgeRefusal refusal,
                                           const OwnerOverride& override_) noexcept;

// Whether the owner's authority reaches an attribute refusal.
[[nodiscard]] OverrideVerdict can_override(AttributeRefusal refusal,
                                           const OwnerOverride& override_) noexcept;

// Turn a model's hypothesis into the owner's own declaration.
//
// This is how someone agrees with a lead without corrupting the record of where it came
// from. The returned edge is `OwnerDeclared`, and its `source_note` carries both the
// original basis and the owner's reason — so a later reader can see that Archie suggested
// it and a person accepted it, which is more informative than either fact alone.
//
// The result still goes through `refuse_edge` like anything else. Promotion changes who is
// making the claim; it does not exempt the claim from the rules about what may be claimed.
[[nodiscard]] RelationEdge promote(const RelationEdge& hypothesis,
                                   const OwnerOverride& override_);

}  // namespace sentry::identity
