//! Applying a Landlock ruleset, irreversibly, to this process.
//!
//! RFC-007 §3.4. The last thing that happens before `exec`.
//!
//! # Three details the kernel is unforgiving about
//!
//! Every one of these fails in a way that looks like something else, which is why they
//! are called out here rather than left to be discovered:
//!
//! 1. **`landlock_ruleset_attr` changes size between ABIs.** It is 8 bytes through ABI 3
//!    and 16 from ABI 4, when `handled_access_net` was added. The size is passed to the
//!    syscall, and a kernel handed a larger struct than it knows returns `E2BIG`.
//!
//! 2. **`landlock_path_beneath_attr` is packed.** `__u64` then `__s32` with no padding:
//!    twelve bytes, not sixteen. A naturally-aligned struct would put the file descriptor
//!    where the kernel expects nothing and pass a garbage `parent_fd`.
//!
//! 3. **`PR_SET_NO_NEW_PRIVS` must be set first.** `landlock_restrict_self` returns
//!    `EPERM` without it. The requirement exists so a confined process cannot regain
//!    privilege through a setuid binary — which is exactly the escape a sandbox is for.
//!
//! # Failure here is never a downgrade
//!
//! If a ruleset cannot be created or applied, this program does not `exec` the worker. A
//! task whose manifest required confinement, running unconfined because a syscall
//! returned an error nobody checked, is the outcome the whole design exists to prevent.

use crate::rights;

/// The size of `struct landlock_ruleset_attr` for a given ABI.
///
/// A pure function because it is testable, and because getting it wrong produces `E2BIG`
/// from a syscall that would otherwise look correct.
pub const fn ruleset_attr_size(abi: u32) -> usize {
    if abi >= 4 {
        // handled_access_fs + handled_access_net
        16
    } else {
        // handled_access_fs alone
        8
    }
}

/// Why a sandbox could not be applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyError {
    /// The kernel refused to create the ruleset.
    CreateFailed { errno: i32 },
    /// A granted path could not be opened. The path in a manifest may simply not exist on
    /// this node, which is a configuration problem rather than a kernel one.
    PathUnopenable { path: String, errno: i32 },
    /// The kernel refused a rule.
    RuleRejected { path: String, errno: i32 },
    /// `PR_SET_NO_NEW_PRIVS` failed; without it `restrict_self` cannot succeed.
    NoNewPrivsFailed { errno: i32 },
    /// The kernel refused to apply the finished ruleset.
    RestrictFailed { errno: i32 },
    /// Landlock is not available at all on this node.
    Unavailable,
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CreateFailed { errno } => write!(
                f,
                "landlock_create_ruleset failed with errno {errno}; E2BIG (7) means the \
                 attribute struct was larger than this kernel's ABI expects"
            ),
            Self::PathUnopenable { path, errno } => write!(
                f,
                "cannot open granted path {path} (errno {errno}); the manifest names a \
                 path that does not exist on this node"
            ),
            Self::RuleRejected { path, errno } => write!(
                f,
                "the kernel rejected the rule for {path} (errno {errno}); EINVAL (22) \
                 means the rule allowed an access the ruleset does not handle"
            ),
            Self::NoNewPrivsFailed { errno } => write!(
                f,
                "PR_SET_NO_NEW_PRIVS failed with errno {errno}; Landlock cannot be \
                 applied without it, because a confined process must not be able to \
                 regain privilege through a setuid binary"
            ),
            Self::RestrictFailed { errno } => write!(
                f,
                "landlock_restrict_self failed with errno {errno}; EPERM (1) usually \
                 means PR_SET_NO_NEW_PRIVS was not set"
            ),
            Self::Unavailable => f.write_str("this kernel has no usable Landlock"),
        }
    }
}

impl std::error::Error for ApplyError {}

/// One path to confine, resolved to the bits it will be granted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub path: String,
    pub allowed_access: u64,
}

/// Turn a task's grants into rules for a given ABI.
///
/// Pure, so the whole translation can be checked without a kernel. Grants that would
/// permit nothing are dropped rather than sent: the kernel accepts such a rule and it
/// does nothing, so sending it would only make a mistake harder to see.
pub fn rules_for(grants: &[archie_execution::spec::FilesystemGrant], abi: u32) -> Vec<Rule> {
    grants
        .iter()
        .filter_map(|grant| {
            let path = grant.path.to_str()?;
            // A grant is a directory unless it plainly names a file. Erring towards
            // directory is right: the directory rights are a superset for the read case,
            // and a rule that under-grants produces a worker failing on a path its
            // manifest said it could use.
            let directory = !path.contains('.') || grant.path.is_dir();
            let bits = rights::rights_for(grant.read, grant.write, grant.execute, directory, abi);
            let bits = rights::mask_to_handled(bits, abi);
            if rights::grants_nothing(bits) {
                return None;
            }
            Some(Rule {
                path: path.to_string(),
                allowed_access: bits,
            })
        })
        .collect()
}

/// Apply the ruleset to this process, irreversibly.
///
/// After this returns `Ok`, nothing this process or any of its children does can widen
/// the restrictions. That is the point, and it is why this runs in the program that is
/// about to `exec` a worker rather than in the supervisor that has other tasks to start.
#[cfg(target_os = "linux")]
pub fn apply(abi: u32, rules: &[Rule]) -> Result<(), ApplyError> {
    use std::os::unix::io::AsRawFd;

    const SYS_LANDLOCK_CREATE_RULESET: libc::c_long = 444;
    const SYS_LANDLOCK_ADD_RULE: libc::c_long = 445;
    const SYS_LANDLOCK_RESTRICT_SELF: libc::c_long = 446;
    const LANDLOCK_RULE_PATH_BENEATH: libc::c_ulong = 1;

    /// `struct landlock_ruleset_attr`. The net field exists from ABI 4; below that the
    /// size passed to the syscall stops short of it, so it is simply not read.
    #[repr(C)]
    struct RulesetAttr {
        handled_access_fs: u64,
        handled_access_net: u64,
    }

    /// `struct landlock_path_beneath_attr`, which the kernel declares packed.
    ///
    /// Twelve bytes, not sixteen. Without `packed` the descriptor lands four bytes past
    /// where the kernel reads it, and the syscall receives whatever happened to be in the
    /// padding — a garbage fd, usually rejected, occasionally not.
    #[repr(C, packed)]
    struct PathBeneathAttr {
        allowed_access: u64,
        parent_fd: i32,
    }

    let attr = RulesetAttr {
        handled_access_fs: rights::handled_access(abi),
        handled_access_net: 0,
    };

    // SAFETY: the pointer is to a live local, and the size is the one this ABI expects.
    let ruleset_fd = unsafe {
        libc::syscall(
            SYS_LANDLOCK_CREATE_RULESET,
            &attr as *const RulesetAttr,
            ruleset_attr_size(abi),
            0usize,
        )
    };
    if ruleset_fd < 0 {
        return Err(ApplyError::CreateFailed {
            errno: last_errno(),
        });
    }
    // Owned so the descriptor is closed on every path out of this function, including
    // the error ones. A leaked ruleset fd would keep an unapplied ruleset alive for the
    // life of the worker.
    let ruleset = unsafe { OwnedFd::from_raw(ruleset_fd as i32) };

    for rule in rules {
        let dir = match std::fs::File::open(&rule.path) {
            Ok(dir) => dir,
            Err(e) => {
                return Err(ApplyError::PathUnopenable {
                    path: rule.path.clone(),
                    errno: e.raw_os_error().unwrap_or(0),
                })
            }
        };

        let beneath = PathBeneathAttr {
            allowed_access: rule.allowed_access,
            parent_fd: dir.as_raw_fd(),
        };

        // SAFETY: both pointers are to live locals, and the file stays open across the
        // call because `dir` is still in scope.
        let result = unsafe {
            libc::syscall(
                SYS_LANDLOCK_ADD_RULE,
                ruleset.0,
                LANDLOCK_RULE_PATH_BENEATH,
                &beneath as *const PathBeneathAttr,
                0usize,
            )
        };
        if result != 0 {
            return Err(ApplyError::RuleRejected {
                path: rule.path.clone(),
                errno: last_errno(),
            });
        }
    }

    // Must come before restrict_self, which returns EPERM without it. The requirement
    // exists so a confined process cannot regain privilege through a setuid binary,
    // which is precisely the escape a sandbox is meant to close.
    // SAFETY: prctl with a constant option and no pointer arguments.
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
        return Err(ApplyError::NoNewPrivsFailed {
            errno: last_errno(),
        });
    }

    // SAFETY: the descriptor is a ruleset this function created.
    if unsafe { libc::syscall(SYS_LANDLOCK_RESTRICT_SELF, ruleset.0, 0usize) } != 0 {
        return Err(ApplyError::RestrictFailed {
            errno: last_errno(),
        });
    }

    Ok(())
}

/// A file descriptor that closes itself.
#[cfg(target_os = "linux")]
struct OwnedFd(libc::c_int);

#[cfg(target_os = "linux")]
impl OwnedFd {
    /// # Safety
    /// `fd` must be a valid, owned descriptor.
    unsafe fn from_raw(fd: libc::c_int) -> Self {
        Self(fd)
    }
}

#[cfg(target_os = "linux")]
impl Drop for OwnedFd {
    fn drop(&mut self) {
        // SAFETY: this type owns the descriptor and is dropped once.
        unsafe { libc::close(self.0) };
    }
}

#[cfg(target_os = "linux")]
fn last_errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// Apply the ruleset to this process, irreversibly.
#[cfg(not(target_os = "linux"))]
pub fn apply(_abi: u32, _rules: &[Rule]) -> Result<(), ApplyError> {
    Err(ApplyError::Unavailable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use archie_execution::spec::FilesystemGrant;

    #[test]
    fn the_attribute_struct_grows_at_abi_four() {
        // A kernel handed a larger struct than its ABI knows returns E2BIG, from a
        // syscall that would otherwise look perfectly correct.
        assert_eq!(ruleset_attr_size(1), 8);
        assert_eq!(ruleset_attr_size(3), 8);
        assert_eq!(
            ruleset_attr_size(4),
            16,
            "handled_access_net was added at ABI 4"
        );
        assert_eq!(ruleset_attr_size(5), 16);
    }

    #[test]
    fn the_packed_layout_is_twelve_bytes_not_sixteen() {
        // Without `packed` the file descriptor lands four bytes past where the kernel
        // reads it, and the syscall receives whatever was in the padding.
        #[repr(C, packed)]
        struct PathBeneathAttr {
            allowed_access: u64,
            parent_fd: i32,
        }
        assert_eq!(std::mem::size_of::<PathBeneathAttr>(), 12);

        #[repr(C)]
        struct Unpacked {
            allowed_access: u64,
            parent_fd: i32,
        }
        assert_eq!(
            std::mem::size_of::<Unpacked>(),
            16,
            "this is the size the kernel would reject, and the reason packed is required"
        );
    }

    #[test]
    fn grants_become_rules_with_masked_bits() {
        let grants = vec![
            FilesystemGrant::read_only("/usr/lib/archie"),
            FilesystemGrant::read_write("/var/lib/archie/work"),
        ];

        let rules = rules_for(&grants, 3);
        assert_eq!(rules.len(), 2);

        let read_only = &rules[0];
        assert_ne!(read_only.allowed_access & rights::access::READ_FILE, 0);
        assert_eq!(read_only.allowed_access & rights::access::WRITE_FILE, 0);
        assert_eq!(
            read_only.allowed_access & rights::access::TRUNCATE,
            0,
            "a read-only path that could be truncated is most of the damage"
        );

        let writable = &rules[1];
        assert_ne!(writable.allowed_access & rights::access::WRITE_FILE, 0);
        assert_ne!(writable.allowed_access & rights::access::MAKE_REG, 0);
    }

    #[test]
    fn every_rule_only_allows_what_its_abi_handles() {
        // A rule allowing an access the ruleset does not handle is rejected with EINVAL,
        // and one rejected rule means the path is not granted at all.
        let grants = vec![FilesystemGrant::read_write("/var/lib/archie/work")];
        for abi in 1..=5 {
            for rule in rules_for(&grants, abi) {
                assert_eq!(
                    rule.allowed_access & !rights::handled_access(abi),
                    0,
                    "abi {abi} produced a rule bit it does not handle"
                );
            }
        }
    }

    #[test]
    fn a_grant_that_permits_nothing_is_dropped_rather_than_sent() {
        // The kernel accepts such a rule and it does nothing, so sending it would only
        // make the mistake harder to see.
        let grants = vec![FilesystemGrant {
            path: std::path::PathBuf::from("/data"),
            read: false,
            write: false,
            execute: false,
        }];
        assert!(rules_for(&grants, 5).is_empty());
    }

    #[test]
    fn errors_name_the_errno_that_explains_them() {
        // Each of these is a different fix, and the numbers are the ones an operator will
        // actually see in a log.
        let create = ApplyError::CreateFailed { errno: 7 };
        assert!(create.to_string().contains("E2BIG"));

        let rule = ApplyError::RuleRejected {
            path: "/x".into(),
            errno: 22,
        };
        assert!(rule.to_string().contains("EINVAL"));
        assert!(rule.to_string().contains("does not handle"));

        let restrict = ApplyError::RestrictFailed { errno: 1 };
        assert!(restrict.to_string().contains("PR_SET_NO_NEW_PRIVS"));

        let nnp = ApplyError::NoNewPrivsFailed { errno: 1 };
        assert!(nnp.to_string().contains("setuid"));

        let missing = ApplyError::PathUnopenable {
            path: "/nope".into(),
            errno: 2,
        };
        assert!(missing.to_string().contains("does not exist on this node"));
    }

    #[test]
    #[cfg(not(target_os = "linux"))]
    fn applying_off_linux_reports_unavailable_rather_than_succeeding() {
        // Returning Ok here would tell the caller a worker was confined on a host with no
        // Landlock at all.
        assert_eq!(apply(4, &[]), Err(ApplyError::Unavailable));
    }
}
