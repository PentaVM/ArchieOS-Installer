//! Names, checked at the prompt rather than three minutes into the install.

use archie_installer::{validate_hostname, validate_username, Answers, InvalidAnswer};
use archie_types::NodeProfile;

#[test]
fn ordinary_names_are_accepted() {
    for name in ["archie", "archie-01", "workshop", "a", "n1"] {
        assert_eq!(validate_hostname(name), Ok(()), "{name}");
    }
    for name in ["nelson", "penta", "n", "_svc", "a-b_c9"] {
        assert_eq!(validate_username(name), Ok(()), "{name}");
    }
}

/// The rules are the kernel's and `useradd`'s, not ones invented here. Somebody who types
/// a name with a space in it should be told at the prompt, where retyping costs nothing.
#[test]
fn a_hostname_the_system_would_reject_is_rejected_here() {
    assert_eq!(validate_hostname(""), Err(InvalidAnswer::EmptyHostname));
    assert_eq!(
        validate_hostname("my machine"),
        Err(InvalidAnswer::HostnameCharacter { found: ' ' })
    );
    assert_eq!(
        validate_hostname("archie.local"),
        Err(InvalidAnswer::HostnameCharacter { found: '.' })
    );
    assert_eq!(
        validate_hostname("-archie"),
        Err(InvalidAnswer::HostnameEdgeHyphen)
    );
    assert_eq!(
        validate_hostname("archie-"),
        Err(InvalidAnswer::HostnameEdgeHyphen)
    );
    assert_eq!(
        validate_hostname(&"a".repeat(64)),
        Err(InvalidAnswer::HostnameTooLong { length: 64 })
    );
    assert_eq!(validate_hostname(&"a".repeat(63)), Ok(()));
}

#[test]
fn a_username_useradd_would_reject_is_rejected_here() {
    assert_eq!(validate_username(""), Err(InvalidAnswer::EmptyUsername));
    assert_eq!(
        validate_username("Nelson"),
        Err(InvalidAnswer::UsernameStart { found: 'N' })
    );
    assert_eq!(
        validate_username("1nelson"),
        Err(InvalidAnswer::UsernameStart { found: '1' })
    );
    assert_eq!(
        validate_username("nelson chess"),
        Err(InvalidAnswer::UsernameCharacter { found: ' ' })
    );
    assert_eq!(
        validate_username(&"n".repeat(33)),
        Err(InvalidAnswer::UsernameTooLong { length: 33 })
    );
}

/// Not a naming collision but a privilege one.
///
/// `archie` is the account the daemon runs under. Letting somebody claim it at the
/// install prompt would give a human login the identity of the control plane.
#[test]
fn the_accounts_the_system_already_owns_cannot_be_claimed() {
    for name in [
        "root",
        "archie",
        "archie-cortex",
        "archie-display",
        "nobody",
    ] {
        assert_eq!(
            validate_username(name),
            Err(InvalidAnswer::UsernameReserved { name: name.into() }),
            "{name}"
        );
    }
}

/// Every message says what to do instead, because it is read by somebody standing at a
/// machine with nothing installed on it and no other computer to look it up on.
#[test]
fn every_refusal_explains_itself() {
    let problems = [
        validate_hostname("").unwrap_err(),
        validate_hostname("a b").unwrap_err(),
        validate_hostname("-x").unwrap_err(),
        validate_hostname(&"a".repeat(64)).unwrap_err(),
        validate_username("").unwrap_err(),
        validate_username("X").unwrap_err(),
        validate_username("a b").unwrap_err(),
        validate_username(&"n".repeat(33)).unwrap_err(),
        validate_username("root").unwrap_err(),
    ];
    for problem in problems {
        let text = problem.to_string();
        assert!(text.len() > 25, "too terse to act on: {text}");
        // Printed mid-sentence, indented under the prompt, so it must not open like a
        // new sentence. A quoted name is fine; a capital letter is not.
        assert!(
            text.chars().next().is_some_and(|c| !c.is_uppercase()),
            "reads as a new sentence where it is printed: {text}"
        );
    }
}

/// An `Answers` that exists has already been checked, so nothing downstream re-asks.
#[test]
fn answers_cannot_be_constructed_around_a_name_that_will_not_work() {
    assert!(Answers::new("my machine", "nelson", NodeProfile::Server, true).is_err());
    assert!(Answers::new("archie", "Nelson", NodeProfile::Server, true).is_err());

    let good = Answers::new("archie", "nelson", NodeProfile::Server, false).expect("valid");
    assert_eq!(good.hostname(), "archie");
    assert_eq!(good.username(), "nelson");
    assert_eq!(good.profile(), NodeProfile::Server);
    assert!(!good.enable_ai_module());
}
