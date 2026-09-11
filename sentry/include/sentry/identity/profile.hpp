// Profiles — open-ended properties on a person, without a hole in the middle.
//
// RFC: docs/RFC-008-SENTRY-WORLD-MODEL.md §Identity
//
// `AttributeClass` in `subject.hpp` is a closed enum, and that is right for the handful of
// things policy actually branches on: a biometric template and a home address need named
// rules. It is wrong for everything else. A profile should hold height, eye colour,
// hobbies, skills, the year someone was met — anything — and an enum cannot enumerate
// "anything".
//
// So traits are open: a key, a value, and where it came from.
//
// # The hole this design has to avoid
//
// An open key/value bag beside a classified enum is a bypass with extra steps. Whoever
// writes `trait{"home_address", "..."}` walks straight around the rule that
// `AttributeClass::HomeAddress` exists to enforce, and nothing notices, because the bag
// takes any string.
//
// The fix is that the key is classified rather than trusted. `classify()` recognises what
// a key *is* — an address, a phone number, a date of birth, a health status — regardless of
// what the caller declared about it, and the stricter of the two classifications wins. A
// caller may say a trait is more sensitive than it looks. A caller may never say it is
// less.
//
// An unrecognised key is `Personal`, not `PublicProfessional`. A default that guessed
// "harmless" would be wrong exactly when someone invents a key nobody anticipated, which is
// the situation the default exists for.
//
// # Where a trait came from is part of the trait
//
// "Rafa is 14" read out of a chat message is not the same kind of fact as "Rafa is 14"
// because Rafa said so. The first is Archie's reading of a sentence — people mistype ages,
// joke, and talk about other people in the third person. Recording the basis is what lets a
// profile show the difference instead of flattening both into a number that looks equally
// certain.

#pragma once

#include <optional>
#include <string>
#include <string_view>
#include <vector>

#include "sentry/identity/subject.hpp"

namespace sentry::identity {

// How exposed a property is.
//
// Three levels rather than a bool, because "eye colour" and "home address" are both
// private in the sense of not being public record, and treating them identically means
// either over-restricting hobbies or under-restricting addresses.
enum class Sensitivity {
    // Public professional record: role, employer, byline, a public account.
    PublicProfessional,
    // About a person's life, but not a lever against them: hobbies, skills, favourite
    // food, the year they were met, eye colour.
    Personal,
    // Usable to locate, impersonate, or discriminate against someone: address, phone
    // number, date of birth, health, precise location, religion, politics.
    Sensitive,
};

[[nodiscard]] std::string_view name(Sensitivity sensitivity) noexcept;

// Whether `a` is at least as restrictive as `b`.
[[nodiscard]] constexpr bool at_least_as_strict(Sensitivity a, Sensitivity b) noexcept {
    return static_cast<int>(a) >= static_cast<int>(b);
}

// What a key is, judged from the key itself.
//
// Substring matching on a normalised key — lowercased, with separators removed — so that
// `home_address`, `homeAddress` and `Home Address` are the same key, and
// `subject_phone_number` is recognised as a phone number.
//
// Deliberately not exhaustive and does not try to be. It catches the categories §40 names
// and the ones that recur; everything else lands on `Personal`, which is the safe side of
// the only mistake this function can make.
//
// It judges the key and cannot see the value, which is a real limit rather than an
// oversight: `lives_in` holds a city as often as a street address, and no amount of key
// matching tells them apart. So this is a floor, not a guarantee — a caller who knows the
// value is more exposed than the key suggests says so through `declared_sensitivity`, and
// the stricter of the two wins.
[[nodiscard]] Sensitivity classify(std::string_view key) noexcept;

// Where a property came from.
enum class TraitBasis {
    // The owner recorded it.
    OwnerDeclared,
    // The person said it about themselves.
    SubjectDeclared,
    // The public professional record.
    PublicRecord,
    // Read out of the owner's own messages, exports, or documents. A reading of a
    // sentence, not a statement of fact — people mistype, joke, and talk about third
    // parties. Carried as a hypothesis.
    InferredFromMessages,
    // A generative model proposed it.
    ModelInferred,
};

[[nodiscard]] std::string_view name(TraitBasis basis) noexcept;

// Whether a basis produces something Sentry asserts, or something it merely suspects.
//
// Both inference bases are hypotheses. This is not a restriction so much as accuracy: a
// profile that shows which fields were read out of a chat and which the person confirmed
// is more useful than one where every field looks equally certain, and the owner is the one
// who benefits from being able to tell.
[[nodiscard]] constexpr bool is_asserted(TraitBasis basis) noexcept {
    return basis == TraitBasis::OwnerDeclared || basis == TraitBasis::SubjectDeclared ||
           basis == TraitBasis::PublicRecord;
}

// One property of one person.
struct Trait {
    std::string key{};
    std::string value{};
    TraitBasis basis = TraitBasis::ModelInferred;
    // What the caller believes about this key, when they have an opinion.
    //
    // Absent by default, meaning `classify(key)` decides alone. A default of `Personal`
    // would look harmless and quietly make every professional key personal — an employer
    // would come out stricter than the public record it came from, and §40's permitted
    // list would refuse itself.
    std::optional<Sensitivity> declared_sensitivity{};
    // When it was learned, and from what. Free text for a human reading the profile.
    std::string source_note{};

    // The classification that actually applies.
    //
    // A caller may declare a trait more sensitive than its key suggests — someone's
    // employer is public record in general and might not be in a particular case. A caller
    // may never declare it less, which is what keeps an open key space from being a way
    // around the closed one.
    [[nodiscard]] Sensitivity effective_sensitivity() const noexcept {
        const auto from_key = classify(key);
        if (!declared_sensitivity) {
            return from_key;
        }
        return at_least_as_strict(*declared_sensitivity, from_key) ? *declared_sensitivity
                                                                   : from_key;
    }

    [[nodiscard]] bool is_hypothesis() const noexcept { return !is_asserted(basis); }
};

// Whether a trait may be recorded against a subject. Returns nothing when it may.
//
// Reuses `AttributeRefusal` rather than inventing a parallel enum, so that
// `identity/override.hpp` already knows how to lift these — an owner overriding a trait
// and an owner overriding a classified attribute are the same act and should not need two
// mechanisms.
[[nodiscard]] std::optional<AttributeRefusal> refuse_trait(const SubjectRecord& subject,
                                                           const Trait& trait) noexcept;

// A person and everything recorded about them.
//
// The traits are a flat list rather than a map: a person can have two employers, three
// phone numbers over time, and a value that changed. Collapsing to one-value-per-key would
// silently drop history that a profile is precisely the place to keep.
struct Profile {
    SubjectRecord subject{};
    std::vector<Trait> traits{};

    // Every trait under this key, newest last.
    [[nodiscard]] std::vector<const Trait*> all(std::string_view key) const;

    // The most recently added trait under this key that Sentry actually asserts.
    //
    // Hypotheses are skipped, so a chat-derived guess never shadows something the person
    // confirmed. When only a guess exists, this returns nothing and the caller has to ask
    // for it explicitly — which is the difference between "Archie does not know" and
    // "Archie suspects", and the two should not read the same.
    [[nodiscard]] const Trait* asserted(std::string_view key) const;

    // The most recent trait under this key whatever its standing.
    [[nodiscard]] const Trait* any(std::string_view key) const;

    // Everything at or above a sensitivity, for a redacted view of the profile.
    [[nodiscard]] std::vector<const Trait*> at_or_above(Sensitivity floor) const;
};

}  // namespace sentry::identity
