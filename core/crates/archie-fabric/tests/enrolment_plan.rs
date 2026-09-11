//! What the installer decides, checked without a disk or a terminal.
//!
//! The wizard's prompts and file writing cannot be tested and are not where the mistakes
//! live. The mistake that matters is an install that quietly ends up somewhere it should
//! not be, and that decision is pure.

use archie_fabric::{
    fingerprint_of_owner_key, plan_enrolment, Enrolment, FabricChoice, MembershipError,
    NodeIdentity, OwnerIdentity, PlanError,
};

/// An owner and the hex an operator would copy off the machine holding it.
///
/// It goes through the on-disk form deliberately. `OwnerIdentity` has no accessor for
/// its private half — a type that will hand you the private key is a type that will hand
/// it to a log line — so the only way to obtain one is the way the installer does, by
/// reading the file.
fn owner_on_disk(tag: &str) -> (OwnerIdentity, String) {
    let dir = std::env::temp_dir().join(format!(
        "archie-plan-{tag}-{}-{}",
        std::process::id(),
        archie_types::Timestamp::now().as_millis()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("owner.key");
    let owner = OwnerIdentity::load_or_create(&path).expect("create owner key");
    let hex = std::fs::read_to_string(&path)
        .expect("read owner key")
        .trim()
        .to_string();
    (owner, hex)
}

/// The default, and what somebody gets by pressing Enter through the wizard.
#[test]
fn an_ordinary_install_starts_its_own_fabric() {
    let node = NodeIdentity::generate();
    let plan = plan_enrolment(&node, &FabricChoice::default(), 1_000).expect("plan");

    assert!(plan.started_new_fabric);
    assert!(
        plan.owner.is_some(),
        "an install that made the fabric keeps the key that can admit things to it"
    );
    assert!(plan.enrolment.is_enrolled());
    assert_eq!(
        plan.enrolment.fabric_fingerprint(),
        Some(plan.fabric_fingerprint())
    );
}

/// The requirement, at the layer where it is decided.
///
/// Two people run the installer. Neither answers any question about isolation, because
/// there is no question about isolation to answer.
#[test]
fn two_ordinary_installs_are_two_fabrics() {
    let first =
        plan_enrolment(&NodeIdentity::generate(), &FabricChoice::StartNew, 1).expect("plan");
    let second =
        plan_enrolment(&NodeIdentity::generate(), &FabricChoice::StartNew, 1).expect("plan");

    assert_ne!(first.fabric_fingerprint(), second.fabric_fingerprint());

    // And each refuses the other, which is the property the fingerprints stand in for.
    assert!(matches!(
        first
            .enrolment
            .admits(&second.membership.node_public_key, Some(&second.membership)),
        Err(MembershipError::ForeignFabric { .. })
    ));
    assert!(matches!(
        second
            .enrolment
            .admits(&first.membership.node_public_key, Some(&first.membership)),
        Err(MembershipError::ForeignFabric { .. })
    ));
}

/// Joining takes a deliberate action, and the action needs something only somebody with
/// the owning machine can produce.
#[test]
fn joining_an_existing_fabric_puts_the_node_in_that_fabric() {
    let (owner, owner_hex) = owner_on_disk("join");

    let joining_node = NodeIdentity::generate();
    let plan = plan_enrolment(
        &joining_node,
        &FabricChoice::JoinWithOwnerKey {
            owner_private_key_hex: owner_hex,
        },
        2_000,
    )
    .expect("plan");

    assert!(!plan.started_new_fabric);
    assert_eq!(plan.fabric_fingerprint(), owner.fingerprint());

    // A node already in that fabric admits the newcomer, which is the point of joining.
    let existing = NodeIdentity::generate();
    let existing_enrolment = Enrolment::new(
        &existing.public_key_hex(),
        owner.admit(&existing.public_key_hex(), 1),
    )
    .expect("enrol");
    assert_eq!(
        existing_enrolment.admits(&joining_node.public_key_hex(), Some(&plan.membership)),
        Ok(())
    );
}

/// Somebody lent an owner key to add one machine has not been handed the fabric.
#[test]
fn joining_does_not_leave_the_owner_key_on_the_joining_machine() {
    let (_owner, owner_hex) = owner_on_disk("lent");
    let plan = plan_enrolment(
        &NodeIdentity::generate(),
        &FabricChoice::JoinWithOwnerKey {
            owner_private_key_hex: owner_hex,
        },
        1,
    )
    .expect("plan");

    assert!(
        plan.owner.is_none(),
        "writing it to their disk would quietly hand them the ability to admit anyone \
         else to a fabric that is not theirs"
    );
}

/// A mistyped or truncated key is refused with something a person can act on, rather
/// than producing an install that silently belongs nowhere.
#[test]
fn an_unusable_owner_key_is_refused_plainly() {
    let node = NodeIdentity::generate();

    assert!(matches!(
        plan_enrolment(
            &node,
            &FabricChoice::JoinWithOwnerKey {
                owner_private_key_hex: "not hex at all".into()
            },
            1
        ),
        Err(PlanError::UnusableOwnerKey(_))
    ));

    assert!(
        matches!(
            plan_enrolment(
                &node,
                &FabricChoice::JoinWithOwnerKey {
                    owner_private_key_hex: "aabbcc".into()
                },
                1
            ),
            Err(PlanError::UnusableOwnerKey(_))
        ),
        "a truncated key is the likely typo, and it must not silently produce a fabric \
         nobody meant"
    );
}

/// The person is told which fabric they are about to join before they commit to it.
#[test]
fn a_key_can_be_identified_before_it_is_used() {
    let (owner, owner_hex) = owner_on_disk("identify");
    assert_eq!(
        fingerprint_of_owner_key(&owner_hex).expect("fingerprint"),
        owner.fingerprint()
    );
    assert!(fingerprint_of_owner_key("rubbish").is_err());
}

/// Whitespace off the end of a copied key is not a different key.
#[test]
fn a_key_pasted_with_a_newline_still_works() {
    let (owner, owner_hex) = owner_on_disk("whitespace");
    let pasted = format!("  {owner_hex}\n");

    let plan = plan_enrolment(
        &NodeIdentity::generate(),
        &FabricChoice::JoinWithOwnerKey {
            owner_private_key_hex: pasted,
        },
        1,
    )
    .expect("a key copied out of a terminal brings its newline with it");
    assert_eq!(plan.fabric_fingerprint(), owner.fingerprint());
}

/// The summary says which of the two things is happening, in words.
#[test]
fn the_plan_can_explain_itself_before_anything_is_written() {
    let fresh =
        plan_enrolment(&NodeIdentity::generate(), &FabricChoice::StartNew, 1).expect("plan");
    let summary = fresh.summary();
    assert!(
        summary.contains("own fabric") && summary.contains("will not talk"),
        "an installer that silently decides which fabric a machine joins is an installer \
         whose most important decision is invisible: {summary}"
    );
    assert!(summary.contains(&fresh.fabric_fingerprint()));

    let (_owner, owner_hex) = owner_on_disk("summary");
    let joining = plan_enrolment(
        &NodeIdentity::generate(),
        &FabricChoice::JoinWithOwnerKey {
            owner_private_key_hex: owner_hex,
        },
        1,
    )
    .expect("plan");
    assert!(joining.summary().contains("join the existing fabric"));
    assert!(joining.summary().contains(&joining.fabric_fingerprint()));
}

/// The certificate the plan produces is the one the node will actually hold.
#[test]
fn the_plan_admits_the_node_it_was_asked_about() {
    let node = NodeIdentity::generate();
    let plan = plan_enrolment(&node, &FabricChoice::StartNew, 1).expect("plan");

    assert_eq!(plan.membership.node_public_key, node.public_key_hex());
    assert_eq!(
        plan.enrolment
            .admits(&node.public_key_hex(), Some(&plan.membership)),
        Ok(())
    );

    // And not somebody else's.
    let other = NodeIdentity::generate();
    assert!(matches!(
        plan.enrolment
            .admits(&other.public_key_hex(), Some(&plan.membership)),
        Err(MembershipError::NotForSender { .. })
    ));
}
