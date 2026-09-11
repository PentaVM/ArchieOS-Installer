//! Who is allowed onto a fabric, and who is not.
//!
//! The property under test is the one an ordinary person is relying on without knowing
//! it: two people who each install ArchieOS on the same network must end up with two
//! fabrics, not one. Nothing they do afterwards should be able to merge them by
//! accident, and neither of them has to remember to turn anything off.

use archie_fabric::{Enrolment, Membership, MembershipError, NodeIdentity, OwnerIdentity};

fn owner_and_node() -> (OwnerIdentity, NodeIdentity) {
    (OwnerIdentity::generate(), NodeIdentity::generate())
}

/// The happy path, so the refusals below mean something.
#[test]
fn a_node_its_owner_admitted_is_accepted() {
    let (owner, node) = owner_and_node();
    let membership = owner.admit(&node.public_key_hex(), 1_000);

    let enrolment = Enrolment::new(&node.public_key_hex(), membership.clone())
        .expect("a node accepts the membership issued for it");
    assert!(enrolment.is_enrolled());
    assert_eq!(
        enrolment.admits(&node.public_key_hex(), Some(&membership)),
        Ok(())
    );
}

/// The whole point. Two installs, two owner keys, two fabrics.
///
/// This is what a guest install produces: the installer generates a fresh owner key, and
/// that alone is the isolation. No allowlist to maintain, no network setting to get
/// right, nothing to forget.
#[test]
fn two_households_on_one_network_do_not_merge() {
    let (nelson, nelson_node) = owner_and_node();
    let (guest, guest_node) = owner_and_node();

    let nelsons_fabric = Enrolment::new(
        &nelson_node.public_key_hex(),
        nelson.admit(&nelson_node.public_key_hex(), 1_000),
    )
    .expect("enrol");
    let guests_fabric = Enrolment::new(
        &guest_node.public_key_hex(),
        guest.admit(&guest_node.public_key_hex(), 1_000),
    )
    .expect("enrol");

    let guest_membership = guest.admit(&guest_node.public_key_hex(), 1_000);
    let nelson_membership = nelson.admit(&nelson_node.public_key_hex(), 1_000);

    // Each refuses the other, in both directions, without either having configured
    // anything about the other's existence.
    assert!(matches!(
        nelsons_fabric.admits(&guest_node.public_key_hex(), Some(&guest_membership)),
        Err(MembershipError::ForeignFabric { .. })
    ));
    assert!(matches!(
        guests_fabric.admits(&nelson_node.public_key_hex(), Some(&nelson_membership)),
        Err(MembershipError::ForeignFabric { .. })
    ));
}

/// A valid signature proves who sent something. It does not prove they were invited,
/// and that distinction is the entire reason this file exists.
#[test]
fn a_genuine_stranger_is_still_a_stranger() {
    let (owner, node) = owner_and_node();
    let fabric = Enrolment::new(
        &node.public_key_hex(),
        owner.admit(&node.public_key_hex(), 1_000),
    )
    .expect("enrol");

    // A real node with a real keypair, announcing honestly. Everything about it is
    // authentic. It was simply never admitted.
    let stranger = NodeIdentity::generate();
    assert!(matches!(
        fabric.admits(&stranger.public_key_hex(), None),
        Err(MembershipError::Unenrolled)
    ));
}

/// A node cannot present someone else's certificate as its own.
#[test]
fn a_membership_cannot_be_lifted_onto_another_node() {
    let (owner, admitted) = owner_and_node();
    let fabric = Enrolment::new(
        &admitted.public_key_hex(),
        owner.admit(&admitted.public_key_hex(), 1_000),
    )
    .expect("enrol");

    // Same fabric, genuine certificate, wrong holder. The certificate names the key it
    // admits, so replaying it from a different node fails on the binding rather than on
    // the signature.
    let interloper = NodeIdentity::generate();
    let borrowed = owner.admit(&admitted.public_key_hex(), 1_000);
    assert!(matches!(
        fabric.admits(&interloper.public_key_hex(), Some(&borrowed)),
        Err(MembershipError::NotForSender { .. })
    ));
}

/// The owner's signature is what makes a certificate mean anything.
#[test]
fn a_forged_certificate_is_refused() {
    let (owner, node) = owner_and_node();
    let fabric = Enrolment::new(
        &node.public_key_hex(),
        owner.admit(&node.public_key_hex(), 1_000),
    )
    .expect("enrol");

    let mut forged = owner.admit(&node.public_key_hex(), 1_000);
    // Someone claiming a later issue date, hoping the signature is not checked against
    // the fields it covers.
    forged.issued_at_ms = 9_999_999;
    assert_eq!(
        fabric.admits(&node.public_key_hex(), Some(&forged)),
        Err(MembershipError::BadSignature)
    );

    let mut mangled = owner.admit(&node.public_key_hex(), 1_000);
    mangled.signature = "00".repeat(64);
    assert_eq!(
        fabric.admits(&node.public_key_hex(), Some(&mangled)),
        Err(MembershipError::BadSignature)
    );
}

/// An impostor cannot mint memberships for a fabric it does not own, even knowing the
/// owner's public key: the certificate is checked against the signature, not the claim.
#[test]
fn claiming_someone_elses_fabric_does_not_join_it() {
    let (nelson, nelson_node) = owner_and_node();
    let fabric = Enrolment::new(
        &nelson_node.public_key_hex(),
        nelson.admit(&nelson_node.public_key_hex(), 1_000),
    )
    .expect("enrol");

    let impostor_owner = OwnerIdentity::generate();
    let impostor_node = NodeIdentity::generate();

    // Signed by the impostor, but relabelled to name Nelson's fabric.
    let mut counterfeit = impostor_owner.admit(&impostor_node.public_key_hex(), 1_000);
    counterfeit.owner_public_key = nelson.public_key_hex();

    assert_eq!(
        fabric.admits(&impostor_node.public_key_hex(), Some(&counterfeit)),
        Err(MembershipError::BadSignature),
        "the fabric field is a claim; the signature is the evidence, and swapping the \
         first without the second must not pass"
    );
}

/// The state a machine is in before anyone has decided what it belongs to.
#[test]
fn a_node_that_belongs_to_nothing_talks_to_nobody() {
    let nothing = Enrolment::none();
    assert!(!nothing.is_enrolled());
    assert!(nothing.membership().is_none());
    assert_eq!(nothing.fabric_fingerprint(), None);

    let (owner, node) = owner_and_node();
    let perfectly_good = owner.admit(&node.public_key_hex(), 1_000);
    assert_eq!(
        nothing.admits(&node.public_key_hex(), Some(&perfectly_good)),
        Err(MembershipError::Unenrolled),
        "belonging to no fabric is not the same as belonging to every fabric, and the \
         second is what an unenrolled node would be if it admitted anything valid"
    );
}

/// A node refuses to adopt a certificate that admits somebody else, rather than holding
/// it and failing confusingly on every packet afterwards.
#[test]
fn enrolment_refuses_a_membership_for_a_different_node() {
    let (owner, node) = owner_and_node();
    let other = NodeIdentity::generate();

    assert!(matches!(
        Enrolment::new(
            &node.public_key_hex(),
            owner.admit(&other.public_key_hex(), 1_000)
        ),
        Err(MembershipError::NotForSender { .. })
    ));
}

/// Two nodes admitted by the same owner are on the same fabric, which is the other half
/// of the isolation property: separating strangers must not also separate a household.
#[test]
fn two_nodes_of_one_household_admit_each_other() {
    let owner = OwnerIdentity::generate();
    let laptop = NodeIdentity::generate();
    let server = NodeIdentity::generate();

    let laptop_fabric = Enrolment::new(
        &laptop.public_key_hex(),
        owner.admit(&laptop.public_key_hex(), 1_000),
    )
    .expect("enrol");
    let server_membership = owner.admit(&server.public_key_hex(), 2_000);

    assert_eq!(
        laptop_fabric.admits(&server.public_key_hex(), Some(&server_membership)),
        Ok(())
    );
    assert_eq!(
        laptop_fabric.fabric_fingerprint(),
        Some(owner.fingerprint()),
        "and both can show the operator the same fabric fingerprint, so two installs \
         meant to be separate can be seen to be separate"
    );
}

/// The certificate survives the wire.
#[test]
fn a_membership_round_trips_through_json() {
    let (owner, node) = owner_and_node();
    let membership = owner.admit(&node.public_key_hex(), 1_234);

    let encoded = serde_json::to_string(&membership).expect("encode");
    let decoded: Membership = serde_json::from_str(&encoded).expect("decode");
    assert_eq!(decoded, membership);
    assert_eq!(
        decoded.verify_for(&owner.public_key_hex(), &node.public_key_hex()),
        Ok(())
    );
}

/// Two owner keys generated moments apart are different fabrics. Obvious, and the whole
/// scheme rests on it, so it is checked rather than assumed.
#[test]
fn every_generated_owner_is_a_new_fabric() {
    let first = OwnerIdentity::generate();
    let second = OwnerIdentity::generate();
    assert_ne!(first.public_key_hex(), second.public_key_hex());
    assert_ne!(first.fingerprint(), second.fingerprint());
    assert_eq!(first.fingerprint().len(), 16);
}

/// An owner key can be put on a disk without ever being handed out.
///
/// The installer needs to persist the particular key a plan was built around. The only
/// other way to get one onto a disk is `load_or_create`, which on an absent path invents
/// a different key — so an installer would generate a fabric, plan an install around it,
/// and then write a second, unrelated fabric to the machine. The alternative, an accessor
/// for the private half, is the thing this type exists to refuse.
#[test]
fn an_owner_key_can_be_saved_and_is_the_same_fabric_when_read_back() {
    let dir = std::env::temp_dir().join(format!(
        "archie-owner-save-{}-{}",
        std::process::id(),
        archie_types::Timestamp::now().as_millis()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("owner.key");

    let owner = OwnerIdentity::generate();
    owner.save(&path).expect("save");

    let reloaded = OwnerIdentity::load(&path).expect("load");
    assert_eq!(reloaded.public_key_hex(), owner.public_key_hex());
    assert_eq!(reloaded.fingerprint(), owner.fingerprint());

    // And it is the same fabric in the sense that matters: a certificate issued by one is
    // accepted by a node enrolled through the other.
    let node = NodeIdentity::generate();
    let enrolment = Enrolment::new(
        &node.public_key_hex(),
        owner.admit(&node.public_key_hex(), 1),
    )
    .expect("enrol");
    assert_eq!(
        enrolment.admits(
            &node.public_key_hex(),
            Some(&reloaded.admit(&node.public_key_hex(), 2))
        ),
        Ok(())
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Replacing an owner key silently is destroying a fabric: every certificate it ever
/// issued stops verifying, and the machines holding them stop being members of anything.
#[test]
fn saving_over_an_existing_owner_key_is_refused() {
    let dir = std::env::temp_dir().join(format!(
        "archie-owner-clobber-{}-{}",
        std::process::id(),
        archie_types::Timestamp::now().as_millis()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("owner.key");

    let first = OwnerIdentity::generate();
    first.save(&path).expect("save");
    assert!(
        OwnerIdentity::generate().save(&path).is_err(),
        "the second save would orphan every node the first one admitted"
    );

    assert_eq!(
        OwnerIdentity::load(&path).expect("load").public_key_hex(),
        first.public_key_hex(),
        "and the refusal leaves the original in place"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
