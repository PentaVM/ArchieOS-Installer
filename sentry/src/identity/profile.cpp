#include "sentry/identity/profile.hpp"

#include <algorithm>
#include <array>
#include <cctype>

namespace sentry::identity {

std::string_view name(Sensitivity sensitivity) noexcept {
    switch (sensitivity) {
        case Sensitivity::PublicProfessional: return "public_professional";
        case Sensitivity::Personal:           return "personal";
        case Sensitivity::Sensitive:          return "sensitive";
    }
    return "unknown";
}

std::string_view name(TraitBasis basis) noexcept {
    switch (basis) {
        case TraitBasis::OwnerDeclared:        return "owner_declared";
        case TraitBasis::SubjectDeclared:      return "subject_declared";
        case TraitBasis::PublicRecord:         return "public_record";
        case TraitBasis::InferredFromMessages: return "inferred_from_messages";
        case TraitBasis::ModelInferred:        return "model_inferred";
    }
    return "unknown";
}

namespace {

// Lowercase; every run of non-alphanumeric characters becomes one space.
//
// So `home_address`, `homeAddress`, `Home Address` and `home-address` all normalise to the
// same words. A key space that anyone can add to needs matching that does not depend on
// them having guessed the house style.
std::string normalise(std::string_view key) {
    std::string out;
    out.reserve(key.size());
    bool pending_space = false;
    for (const unsigned char c : key) {
        if (std::isalnum(c) != 0) {
            if (pending_space && !out.empty()) {
                out.push_back(' ');
            }
            pending_space = false;
            out.push_back(static_cast<char>(std::tolower(c)));
        } else {
            pending_space = true;
        }
    }
    return out;
}

// Whole-word match.
//
// Short words have to be matched this way or they find themselves inside longer ones that
// mean something else entirely: `age` is in `last_message` and `messages_exchanged`, `race`
// is in `bracelet`, `income` is in `incoming_calls`. A profile where the number of messages
// someone sent is classified as sensitive personal data is not being careful, it is being
// useless, and the owner learns to pass `--all` for everything.
bool has_word(std::string_view words, std::initializer_list<std::string_view> needles) {
    std::size_t at = 0;
    while (at <= words.size()) {
        const auto space = words.find(' ', at);
        const auto word = words.substr(at, space == std::string_view::npos ? std::string_view::npos
                                                                           : space - at);
        for (const auto needle : needles) {
            if (word == needle) {
                return true;
            }
        }
        if (space == std::string_view::npos) {
            break;
        }
        at = space + 1;
    }
    return false;
}

// Substring match against the key with its spaces removed.
//
// Only for fragments long and specific enough that finding them inside another word means
// the other word is about the same thing: `homeaddress` inside `subjecthomeaddressline1`.
bool has_fragment(std::string_view squashed, std::initializer_list<std::string_view> needles) {
    return std::ranges::any_of(needles, [squashed](std::string_view needle) {
        return squashed.find(needle) != std::string_view::npos;
    });
}

}  // namespace

Sensitivity classify(std::string_view key) noexcept {
    const std::string words = normalise(key);
    std::string squashed;
    squashed.reserve(words.size());
    for (const char c : words) {
        if (c != ' ') {
            squashed.push_back(c);
        }
    }

    // Sensitive first, and checked before anything else. A key that looks both professional
    // and sensitive — `work_address`, `office_phone` — is sensitive: the categories overlap,
    // and the overlap is exactly where a wrong guess does damage.
    if (has_word(words, {"age", "dob", "ssn", "gps", "race", "bank", "income", "salary",
                         "sex", "gender", "religion", "politics", "political", "health",
                         "diagnosis", "medication", "disability", "ethnicity", "nationality",
                         "immigration", "passport", "imei", "token", "secret", "password",
                         "birthday", "birthplace", "residence", "hometown"})) {
        return Sensitivity::Sensitive;
    }
    if (has_fragment(squashed, {"address", "phone", "mobilenumber", "telephone", "whatsapp",
                                "email", "postcode", "zipcode", "coordinate", "latitude",
                                "longitude", "geolocation", "livelocation", "livesin",
                                "dateofbirth", "birthdate", "placeofbirth", "nationalid",
                                "taxid", "driverlicence", "driverlicense", "medicalrecord",
                                "healthstatus", "creditcard", "biometric", "faceprint",
                                "fingerprint", "gaitsignature", "deviceid", "macaddress"})) {
        return Sensitivity::Sensitive;
    }

    // Public professional record. Narrow on purpose: these are the keys where §40 says
    // collection is fine, and a category that grew loosely would start quietly admitting
    // things that are not public at all.
    if (has_word(words, {"role", "title", "employer", "company", "organisation",
                         "organization", "affiliation", "byline", "publication", "publisher",
                         "outlet", "handle", "website", "quote"}) ||
        has_fragment(squashed, {"jobtitle", "publicaccount", "profileurl",
                                "publicstatement"})) {
        return Sensitivity::PublicProfessional;
    }

    // Everything else. Height, eye colour, hobbies, skills, the year someone was met, a
    // favourite band — and every key nobody has thought of yet, which is the case this
    // default exists for. `Personal` is the safe side of the only mistake possible here.
    return Sensitivity::Personal;
}

std::optional<AttributeRefusal> refuse_trait(const SubjectRecord& subject,
                                             const Trait& trait) noexcept {
    // Same rule as `refuse_attribute`: a track is a body a sensor saw, and the first
    // property hung on it is the moment it stops being anonymous.
    if (subject.kind == SubjectKind::AnonymousTrack) {
        return AttributeRefusal::AnonymousTrackCannotCarryIdentity;
    }

    // §40 applied to the open key space. Without this the enum's protection is decorative:
    // anyone can write `trait{"home_address", ...}` and walk around
    // `AttributeClass::HomeAddress` entirely.
    //
    // Both `Sensitive` and `Personal` are refused on a public figure. Being in the news
    // does not put someone's hobbies and eye colour into the public record either — the
    // professional record is what is public about them, and that is the whole of it.
    if (subject.kind == SubjectKind::PublicFigure &&
        trait.effective_sensitivity() != Sensitivity::PublicProfessional) {
        return AttributeRefusal::PrivateDataOnPublicFigure;
    }

    return std::nullopt;
}

std::vector<const Trait*> Profile::all(std::string_view key) const {
    std::vector<const Trait*> found;
    for (const auto& trait : traits) {
        if (trait.key == key) {
            found.push_back(&trait);
        }
    }
    return found;
}

const Trait* Profile::asserted(std::string_view key) const {
    const Trait* newest = nullptr;
    for (const auto& trait : traits) {
        if (trait.key == key && is_asserted(trait.basis)) {
            newest = &trait;
        }
    }
    return newest;
}

const Trait* Profile::any(std::string_view key) const {
    const Trait* newest = nullptr;
    for (const auto& trait : traits) {
        if (trait.key == key) {
            newest = &trait;
        }
    }
    return newest;
}

std::vector<const Trait*> Profile::at_or_above(Sensitivity floor) const {
    std::vector<const Trait*> found;
    for (const auto& trait : traits) {
        if (at_least_as_strict(trait.effective_sensitivity(), floor)) {
            found.push_back(&trait);
        }
    }
    return found;
}

}  // namespace sentry::identity
