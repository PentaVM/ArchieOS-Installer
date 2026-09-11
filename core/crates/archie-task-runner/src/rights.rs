//! Turning a filesystem grant into Landlock access bits.
//!
//! RFC-007 §3.4. This is where the bugs would live, so it is pure arithmetic with no
//! syscalls in sight and tests that can run anywhere.
//!
//! # A ruleset denies everything it handles and was not granted
//!
//! Landlock does not work by listing what to forbid. A ruleset declares the access types
//! it *handles*, and from that moment every one of them is denied everywhere except where
//! a rule explicitly allows it.
//!
//! That inverts the usual mistake. Forgetting to handle an access type does not close a
//! hole — it leaves that type completely unrestricted, silently, while the ruleset looks
//! populated and the process looks confined. So [`handled_access`] takes everything the
//! running kernel knows about, and the grants narrow it back down.
//!
//! # Handling more than the kernel knows is a hard failure
//!
//! `landlock_create_ruleset` rejects a `handled_access_fs` containing a bit the kernel
//! does not support, with `EINVAL`. So the handled set has to be masked to the ABI —
//! which is the same reason the ABI is probed rather than assumed, and the reason a
//! future ABI is treated as no better than the newest known one.

/// Landlock filesystem access rights, as the kernel defines them.
///
/// Written out rather than pulled from a crate: these are ABI constants that will never
/// change, and a wrong value here is a silently weaker sandbox rather than a build error.
pub mod access {
    pub const EXECUTE: u64 = 1 << 0;
    pub const WRITE_FILE: u64 = 1 << 1;
    pub const READ_FILE: u64 = 1 << 2;
    pub const READ_DIR: u64 = 1 << 3;
    pub const REMOVE_DIR: u64 = 1 << 4;
    pub const REMOVE_FILE: u64 = 1 << 5;
    pub const MAKE_CHAR: u64 = 1 << 6;
    pub const MAKE_DIR: u64 = 1 << 7;
    pub const MAKE_REG: u64 = 1 << 8;
    pub const MAKE_SOCK: u64 = 1 << 9;
    pub const MAKE_FIFO: u64 = 1 << 10;
    pub const MAKE_BLOCK: u64 = 1 << 11;
    pub const MAKE_SYM: u64 = 1 << 12;
    /// ABI 2. Linking and renaming across directories.
    pub const REFER: u64 = 1 << 13;
    /// ABI 3. Emptying a file without opening it for writing.
    pub const TRUNCATE: u64 = 1 << 14;
    /// ABI 5. ioctl on device files.
    pub const IOCTL_DEV: u64 = 1 << 15;
}

/// Everything an ABI knows how to handle.
///
/// Cumulative, and capped at the newest version this code understands. A kernel newer
/// than this program may handle more; asking it to handle a bit this program does not
/// know the meaning of is not possible, and pretending otherwise would be the same lie as
/// claiming an old kernel enforces more than it does.
pub fn handled_access(abi: u32) -> u64 {
    use access::*;

    let mut bits = EXECUTE
        | WRITE_FILE
        | READ_FILE
        | READ_DIR
        | REMOVE_DIR
        | REMOVE_FILE
        | MAKE_CHAR
        | MAKE_DIR
        | MAKE_REG
        | MAKE_SOCK
        | MAKE_FIFO
        | MAKE_BLOCK
        | MAKE_SYM;

    // Each of these is rejected with EINVAL by a kernel that does not know it, so the
    // mask is not a nicety — a ruleset asking for TRUNCATE on ABI 2 fails to create at
    // all, and the process would then run unconfined unless the failure is handled.
    if abi >= 2 {
        bits |= REFER;
    }
    if abi >= 3 {
        bits |= TRUNCATE;
    }
    if abi >= 5 {
        bits |= IOCTL_DEV;
    }
    bits
}

/// What one grant permits, in access bits.
///
/// `directory` matters: the read and write rights for a directory are different bits from
/// the ones for a file, and granting a directory only `READ_FILE` produces a path a
/// worker can open files under but not list — which fails in a way that looks like a
/// missing file rather than a missing permission.
pub fn rights_for(read: bool, write: bool, execute: bool, directory: bool, abi: u32) -> u64 {
    use access::*;
    let mut bits = 0;

    if read {
        bits |= READ_FILE;
        if directory {
            bits |= READ_DIR;
        }
    }

    if write {
        bits |= WRITE_FILE;
        if directory {
            // Writing *in* a directory means creating and removing entries. Without
            // these a worker granted write access to its output directory can modify
            // files that already exist and cannot create one, which is almost never what
            // anybody meant.
            bits |= MAKE_REG | MAKE_DIR | REMOVE_FILE | REMOVE_DIR | MAKE_SYM;
        }
        if abi >= 3 {
            // Truncation follows write. On a kernel that handles it, a writable path that
            // did not grant it produces a worker that can rewrite a file but not shorten
            // it — a confusing partial failure rather than a security property.
            bits |= TRUNCATE;
        }
    }

    if execute {
        bits |= EXECUTE;
    }

    bits
}

/// Whether these rights would produce a rule that grants nothing.
///
/// Landlock accepts such a rule and it does nothing useful, so it is worth catching:
/// a grant that permits nothing is a configuration mistake, and the resulting worker
/// fails on first access for a reason invisible in the manifest.
pub fn grants_nothing(rights: u64) -> bool {
    rights == 0
}

/// Mask a grant's rights to what the ruleset actually handles.
///
/// A rule may not allow an access the ruleset does not handle — `landlock_add_rule`
/// returns `EINVAL`. Masking rather than failing is right here: the caller already asked
/// for what it wanted, and the ABI has already been reported honestly, so quietly
/// dropping a bit the kernel cannot enforce is the same outcome the caveat in the plan
/// already describes.
pub fn mask_to_handled(rights: u64, abi: u32) -> u64 {
    rights & handled_access(abi)
}

#[cfg(test)]
mod tests {
    use super::access::*;
    use super::*;

    #[test]
    fn an_older_abi_handles_a_strict_subset() {
        let one = handled_access(1);
        let two = handled_access(2);
        let three = handled_access(3);
        let five = handled_access(5);

        assert_eq!(one & REFER, 0, "REFER is ABI 2");
        assert_ne!(two & REFER, 0);
        assert_eq!(two & TRUNCATE, 0, "TRUNCATE is ABI 3");
        assert_ne!(three & TRUNCATE, 0);
        assert_eq!(three & IOCTL_DEV, 0, "IOCTL_DEV is ABI 5");
        assert_ne!(five & IOCTL_DEV, 0);

        // Strictly cumulative: every bit an older ABI handles, a newer one handles too.
        assert_eq!(one & two, one);
        assert_eq!(two & three, two);
        assert_eq!(three & five, three);
    }

    #[test]
    fn a_future_abi_is_not_assumed_to_handle_more() {
        // Asking a kernel to handle a bit this program does not know the meaning of is
        // not possible, and pretending otherwise is the same lie as claiming an old
        // kernel enforces more than it does.
        assert_eq!(handled_access(99), handled_access(5));
    }

    #[test]
    fn the_handled_set_is_everything_because_omission_opens_a_hole() {
        // Landlock denies what it handles and was not granted. Forgetting to handle an
        // access type leaves it *unrestricted*, silently, while the ruleset looks
        // populated and the process looks confined.
        let handled = handled_access(1);
        for (name, bit) in [
            ("EXECUTE", EXECUTE),
            ("WRITE_FILE", WRITE_FILE),
            ("READ_FILE", READ_FILE),
            ("READ_DIR", READ_DIR),
            ("REMOVE_DIR", REMOVE_DIR),
            ("REMOVE_FILE", REMOVE_FILE),
            ("MAKE_CHAR", MAKE_CHAR),
            ("MAKE_DIR", MAKE_DIR),
            ("MAKE_REG", MAKE_REG),
            ("MAKE_SOCK", MAKE_SOCK),
            ("MAKE_FIFO", MAKE_FIFO),
            ("MAKE_BLOCK", MAKE_BLOCK),
            ("MAKE_SYM", MAKE_SYM),
        ] {
            assert_ne!(
                handled & bit,
                0,
                "{name} must be handled or it is unrestricted"
            );
        }
    }

    #[test]
    fn a_readable_directory_can_be_listed_not_only_opened_under() {
        // Granting a directory only READ_FILE produces a path a worker can open files
        // under but not list, which fails looking like a missing file rather than a
        // missing permission.
        let file = rights_for(true, false, false, false, 3);
        assert_ne!(file & READ_FILE, 0);
        assert_eq!(file & READ_DIR, 0, "a file is not listable");

        let dir = rights_for(true, false, false, true, 3);
        assert_ne!(dir & READ_FILE, 0);
        assert_ne!(dir & READ_DIR, 0);
    }

    #[test]
    fn a_writable_directory_can_have_entries_created_in_it() {
        // Without these, a worker granted write access to its output directory can modify
        // files that already exist and cannot create one — almost never what was meant.
        let dir = rights_for(true, true, false, true, 3);
        for (name, bit) in [
            ("MAKE_REG", MAKE_REG),
            ("MAKE_DIR", MAKE_DIR),
            ("REMOVE_FILE", REMOVE_FILE),
            ("REMOVE_DIR", REMOVE_DIR),
        ] {
            assert_ne!(dir & bit, 0, "a writable directory needs {name}");
        }
    }

    #[test]
    fn truncation_follows_write_and_only_where_the_kernel_handles_it() {
        // On a kernel that handles TRUNCATE, a writable path without it produces a worker
        // that can rewrite a file but not shorten it — a confusing partial failure.
        let writable_new = rights_for(true, true, false, false, 3);
        assert_ne!(writable_new & TRUNCATE, 0);

        let writable_old = rights_for(true, true, false, false, 2);
        assert_eq!(
            writable_old & TRUNCATE,
            0,
            "ABI 2 rejects a rule asking for a bit it does not handle"
        );

        // And a read-only path never gets it, which is the whole point: without it, a
        // worker with read-only access to a file can still empty it.
        let read_only = rights_for(true, false, false, false, 3);
        assert_eq!(read_only & TRUNCATE, 0);
    }

    #[test]
    fn execute_is_off_unless_asked_for() {
        // A data directory that is also executable is a sandbox with a door in it.
        let data = rights_for(true, true, false, true, 3);
        assert_eq!(data & EXECUTE, 0);

        let binaries = rights_for(true, false, true, true, 3);
        assert_ne!(binaries & EXECUTE, 0);
        assert_eq!(binaries & WRITE_FILE, 0, "read-execute is not writable");
    }

    #[test]
    fn a_grant_that_permits_nothing_is_recognisable() {
        assert!(grants_nothing(rights_for(false, false, false, false, 5)));
        assert!(!grants_nothing(rights_for(true, false, false, false, 5)));
    }

    #[test]
    fn rights_are_masked_to_what_the_ruleset_handles() {
        // A rule may not allow an access the ruleset does not handle: add_rule returns
        // EINVAL, and one rejected rule means the path is not granted at all.
        let with_truncate = READ_FILE | TRUNCATE;
        assert_eq!(mask_to_handled(with_truncate, 3), with_truncate);
        assert_eq!(
            mask_to_handled(with_truncate, 2),
            READ_FILE,
            "TRUNCATE must be dropped on a kernel that cannot handle it"
        );

        let with_ioctl = READ_FILE | IOCTL_DEV;
        assert_eq!(mask_to_handled(with_ioctl, 4), READ_FILE);
        assert_eq!(mask_to_handled(with_ioctl, 5), with_ioctl);
    }

    #[test]
    fn every_right_a_grant_produces_is_one_the_ruleset_handles() {
        // The invariant that keeps add_rule from failing: whatever `rights_for` computes
        // for an ABI must survive masking at that same ABI unchanged.
        for abi in 1..=5 {
            for (read, write, execute, directory) in [
                (true, false, false, false),
                (true, false, false, true),
                (true, true, false, true),
                (true, false, true, true),
                (true, true, true, true),
            ] {
                let rights = rights_for(read, write, execute, directory, abi);
                assert_eq!(
                    mask_to_handled(rights, abi),
                    rights,
                    "abi {abi} produced a right it does not handle"
                );
            }
        }
    }
}
