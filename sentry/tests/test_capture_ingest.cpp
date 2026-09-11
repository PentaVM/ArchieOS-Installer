// A stream of captured events becomes counts and contacts, and nothing else.
//
// The gate is tested in test_capture; this is about what survives it into the graph — a
// summary, never a store of what happened — and that the suppressions are counted so the
// owner can audit them.

#include "harness.hpp"

#include "sentry/capture/ingest.hpp"

using namespace sentry::capture;
using sentry::identity::EdgeBasis;

namespace {

std::map<std::string, DeviceEnrollment, std::less<>> phone_enrolled() {
    DeviceEnrollment d;
    d.device_id = "phone";
    d.label = "Nelson's phone";
    d.enrolled_at = TimePoint{0};
    d.active = true;
    std::map<std::string, DeviceEnrollment, std::less<>> m;
    m.emplace("phone", d);
    return m;
}

CapturePolicy policy() {
    CapturePolicy p;
    p.owner_id = "nelson";
    p.record_when_presumed = true;
    p.record_when_unknown = false;
    return p;
}

const char* kStream =
    R"({"device_id":"phone","source":"web_search","actor":"confirmed_owner","author":"owner","observed_at":"1000"})"
    "\n"
    R"({"device_id":"phone","source":"web_search","actor":"confirmed_owner","author":"owner","observed_at":"2000"})"
    "\n"
    R"({"device_id":"phone","source":"email","actor":"confirmed_owner","author":"owner","observed_at":"3000"})"
    "\n"
    // Mum picks up the phone.
    R"({"device_id":"phone","source":"web_search","actor":"confirmed_other","author":"owner","observed_at":"4000"})"
    "\n"
    // A message from Rafa: owner is at the device, but Rafa wrote it.
    R"({"device_id":"phone","source":"notification","actor":"confirmed_owner","author":"counterparty","counterparty":"Rafa","observed_at":"5000"})"
    "\n"
    // The owner replies to Rafa.
    R"({"device_id":"phone","source":"notification","actor":"confirmed_owner","author":"owner","counterparty":"Rafa","observed_at":"6000"})"
    "\n"
    // An event from a phone the owner never enrolled.
    R"({"device_id":"unknown-phone","source":"email","actor":"confirmed_owner","author":"owner","observed_at":"7000"})"
    "\n";

void a_stream_becomes_counts_not_content() {
    CASE("what the owner did, as numbers");

    const auto summary = ingest_stream(policy(), phone_enrolled(), "nelson", kStream);

    CHECK_EQ(summary.events_seen, std::size_t{7});
    CHECK_EQ(summary.recorded, std::size_t{4});  // 2 searches, 1 email, 1 owner-sent message

    std::size_t searches = 0;
    for (const auto& tally : summary.by_source) {
        if (tally.source == CaptureSource::WebSearch) {
            searches = tally.recorded;
        }
    }
    CHECK_EQ(searches, std::size_t{2});
}

void every_suppression_is_counted_so_the_owner_can_audit_it() {
    CASE("the gate is visible, not taken on faith");

    const auto summary = ingest_stream(policy(), phone_enrolled(), "nelson", kStream);

    CHECK_BECAUSE(summary.suppressed.not_the_owner == 1,
                  "the one event while mum held the phone is counted as suppressed, so the "
                  "owner can see the gate fired");
    CHECK_EQ(summary.suppressed.counterparty_content, std::size_t{1});  // Rafa's message
    CHECK_EQ(summary.suppressed.device_not_enrolled, std::size_t{1});   // unknown-phone
    CHECK_EQ(summary.suppressed.total(), std::size_t{3});
}

void a_contact_becomes_a_hypothesis_not_an_assertion() {
    CASE("that the owner knows Rafa is a guess to confirm");

    const auto summary = ingest_stream(policy(), phone_enrolled(), "nelson", kStream);

    // Rafa appeared twice — a received message and a sent reply — and both count toward the
    // contact even though the received one's content was suppressed.
    CHECK_EQ(summary.contact_counts.at("Rafa"), std::size_t{2});

    CHECK_EQ(summary.contact_hypotheses.size(), std::size_t{1});
    if (!summary.contact_hypotheses.empty()) {
        const auto& edge = summary.contact_hypotheses[0];
        CHECK_EQ(edge.from_id, std::string{"nelson"});
        CHECK_EQ(edge.to_id, std::string{"Rafa"});
        CHECK_BECAUSE(edge.basis == EdgeBasis::ModelHypothesis,
                      "a name in a notification is evidence the owner knows someone, never "
                      "an asserted relationship");
    }
}

void a_contact_on_an_un_enrolled_device_is_not_observed() {
    CASE("a contact only counts from a device that is actually the owner's");

    const char* stream =
        R"({"device_id":"unknown-phone","source":"notification","actor":"confirmed_owner","author":"counterparty","counterparty":"Stranger","observed_at":"1000"})"
        "\n";
    const auto summary = ingest_stream(policy(), phone_enrolled(), "nelson", stream);

    CHECK_BECAUSE(summary.contact_counts.empty(),
                  "an un-enrolled device cannot introduce a contact any more than it can "
                  "record activity");
    CHECK_EQ(summary.suppressed.device_not_enrolled, std::size_t{1});
}

void a_guest_using_the_phone_introduces_no_contact() {
    CASE("who mum messages is not the owner's contact");

    const char* stream =
        R"({"device_id":"phone","source":"notification","actor":"confirmed_other","author":"counterparty","counterparty":"MumsFriend","observed_at":"1000"})"
        "\n";
    const auto summary = ingest_stream(policy(), phone_enrolled(), "nelson", stream);

    CHECK_BECAUSE(summary.contact_counts.empty(),
                  "a contact seen while a guest held the device is the guest's, not the "
                  "owner's, and is not recorded at all");
    CHECK_EQ(summary.suppressed.not_the_owner, std::size_t{1});
}

void a_malformed_line_is_counted_not_fatal() {
    CASE("a bad line costs a count, not the stream");

    const char* stream =
        R"({"device_id":"phone","source":"web_search","actor":"confirmed_owner","author":"owner","observed_at":"1000"})"
        "\n"
        "this is not json\n"
        R"({"source":"email"})"  // no device
        "\n"
        R"({"device_id":"phone","source":"not_a_real_source","actor":"confirmed_owner"})"
        "\n";
    const auto summary = ingest_stream(policy(), phone_enrolled(), "nelson", stream);

    CHECK_EQ(summary.recorded, std::size_t{1});
    CHECK_EQ(summary.unparsed, std::size_t{3});
}

void an_empty_stream_is_empty() {
    CASE("nothing in, nothing out");

    const auto summary = ingest_stream(policy(), phone_enrolled(), "nelson", "");
    CHECK_EQ(summary.events_seen, std::size_t{0});
    CHECK_EQ(summary.recorded, std::size_t{0});
    CHECK(summary.contact_hypotheses.empty());
}

void parse_defaults_are_conservative() {
    CASE("an event that omits actor and author reads as unknown and system");

    const auto e = parse_event(R"({"device_id":"phone","source":"app_usage"})");
    CHECK(e.has_value());
    if (e) {
        CHECK_EQ(e->actor, ActorAttribution::Unknown);
        CHECK_EQ(e->author, ContentAuthor::System);
    }
}

void capture_is_bounded_before_a_noisy_sensor_can_fill_memory() {
    CASE("large ambient streams have fixed event and identity ceilings");

    std::string stream;
    const std::string prefix =
        R"({"device_id":"phone","source":"notification","actor":"confirmed_owner","author":"owner","counterparty":")";
    for (std::size_t i = 0; i < kMaxUniqueContacts + 100; ++i) {
        stream += prefix + "contact-" + std::to_string(i) + R"(","observed_at":"1000"})" + "\n";
    }
    const auto summary = ingest_stream(policy(), phone_enrolled(), "nelson", stream);
    CHECK_EQ(summary.contact_counts.size(), kMaxUniqueContacts);
    CHECK_EQ(summary.suppressed.contact_capacity, std::size_t{100});

    std::string too_many;
    const std::string event =
        R"({"device_id":"phone","source":"app_usage","actor":"confirmed_owner","author":"system"})"
        "\n";
    for (std::size_t i = 0; i < kMaxCaptureEvents + 10; ++i) {
        too_many += event;
    }
    const auto bounded = ingest_stream(policy(), phone_enrolled(), "nelson", too_many);
    CHECK(bounded.input_truncated);
    CHECK(bounded.events_seen + bounded.unparsed <= kMaxCaptureEvents);
}

void oversized_identifiers_and_lines_are_not_allocated_into_the_summary() {
    CASE("wire bounds agree with the memory gate");
    const std::string huge(kMaxCaptureIdentifierBytes + 1, 'x');
    const std::string line =
        R"({"device_id":"phone","source":"notification","actor":"confirmed_owner","counterparty":")" +
        huge + R"("})";
    CHECK(!parse_event(line).has_value());
    CHECK(!parse_event(std::string(kMaxCaptureLineBytes + 1, 'x')).has_value());
}

}  // namespace

int main() {
    a_stream_becomes_counts_not_content();
    every_suppression_is_counted_so_the_owner_can_audit_it();
    a_contact_becomes_a_hypothesis_not_an_assertion();
    a_contact_on_an_un_enrolled_device_is_not_observed();
    a_guest_using_the_phone_introduces_no_contact();
    a_malformed_line_is_counted_not_fatal();
    an_empty_stream_is_empty();
    parse_defaults_are_conservative();
    capture_is_bounded_before_a_noisy_sensor_can_fill_memory();
    oversized_identifiers_and_lines_are_not_allocated_into_the_summary();

    return sentry::test::report();
}
