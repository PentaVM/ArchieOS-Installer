//! Asking the kernel what Landlock it actually has.
//!
//! RFC-007 §3.4. One syscall, and it is the syscall that stops `SandboxStrength` from
//! being a promise nobody checked.
//!
//! # Why probe rather than parse a version
//!
//! A kernel's release number does not answer this. Landlock can be compiled out, and it
//! can be disabled at boot with `lsm=` — a 6.8 kernel with `CONFIG_SECURITY_LANDLOCK=n`
//! looks exactly like a 5.10 one to anything that needs it. The distribution's version
//! string would say otherwise, confidently.
//!
//! So the answer comes from `landlock_create_ruleset(NULL, 0,
//! LANDLOCK_CREATE_RULESET_VERSION)`, which is defined to return the supported ABI
//! version and to create nothing. It is the kernel's own answer about itself.
//!
//! # Every failure means "no Landlock", and they are not the same
//!
//! `ENOSYS` is a kernel too old to have the syscall. `EOPNOTSUPP` is a kernel that has it
//! and has Landlock disabled. Both mean the same thing for the decision — this node
//! cannot confine a filesystem — and they mean very different things to whoever has to
//! fix it, so the reason is carried alongside the verdict rather than collapsed into
//! `None`.

/// The result of asking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Probe {
    /// Landlock is available at this ABI version.
    Available(u32),
    /// It is not, and this is why.
    Unavailable(Unavailable),
}

/// Why Landlock could not be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unavailable {
    /// The kernel does not have the syscall. Pre-5.13, or built without it.
    NoSuchSyscall,
    /// The kernel has the syscall and Landlock is disabled — most often `lsm=` at boot
    /// leaving it out of the active stack.
    DisabledInKernel,
    /// This host is not Linux. Not a fault; there is simply no Landlock to have.
    NotLinux,
    /// The syscall failed in a way that is none of the above.
    Unexpected(i32),
}

impl Probe {
    /// The ABI version, if there is one.
    pub fn abi(&self) -> Option<u32> {
        match self {
            Self::Available(abi) => Some(*abi),
            Self::Unavailable(_) => None,
        }
    }

    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available(_))
    }

    /// A sentence for an operator who expected confinement and did not get it.
    pub fn explain(&self) -> String {
        match self {
            Self::Available(abi) => format!("Landlock ABI {abi}"),
            Self::Unavailable(Unavailable::NoSuchSyscall) => {
                "this kernel has no landlock_create_ruleset syscall; Landlock needs 5.13 \
                 or later, built in"
                    .into()
            }
            Self::Unavailable(Unavailable::DisabledInKernel) => {
                "this kernel has Landlock compiled in but not active; check the `lsm=` \
                 boot parameter, which must include `landlock`"
                    .into()
            }
            Self::Unavailable(Unavailable::NotLinux) => {
                "not Linux; there is no Landlock here to have".into()
            }
            Self::Unavailable(Unavailable::Unexpected(errno)) => {
                format!("landlock_create_ruleset failed unexpectedly with errno {errno}")
            }
        }
    }
}

/// Interpret the syscall's return value.
///
/// Split out from the syscall so the mapping is testable on any host — the part that can
/// be got wrong is which errno means what, not how to call a number.
pub fn interpret(result: i64, errno: i32) -> Probe {
    if result >= 0 {
        // A zero ABI would mean "supported, version zero", which does not exist. Treating
        // it as available would have the caller build a ruleset against a version the
        // kernel never claimed.
        if result == 0 {
            return Probe::Unavailable(Unavailable::Unexpected(0));
        }
        return Probe::Available(result as u32);
    }

    Probe::Unavailable(match errno {
        // ENOSYS: no such syscall.
        38 => Unavailable::NoSuchSyscall,
        // EOPNOTSUPP: the syscall exists, Landlock does not.
        95 => Unavailable::DisabledInKernel,
        other => Unavailable::Unexpected(other),
    })
}

/// Ask this kernel.
#[cfg(target_os = "linux")]
pub fn probe() -> Probe {
    // x86_64 syscall number for landlock_create_ruleset. Taken from the kernel's own
    // table rather than a crate, because this is the only syscall this program makes
    // before it has decided anything, and a wrong number here would silently probe
    // something else entirely.
    const SYS_LANDLOCK_CREATE_RULESET: libc::c_long = 444;
    // LANDLOCK_CREATE_RULESET_VERSION: ask for the version rather than create a ruleset.
    const VERSION_FLAG: libc::c_ulong = 1;

    // SAFETY: the version query is defined to take a null attribute pointer and a zero
    // size, and to create nothing. It is the one call in this file that touches the
    // kernel, and it is a pure question.
    let result = unsafe {
        libc::syscall(
            SYS_LANDLOCK_CREATE_RULESET,
            std::ptr::null::<libc::c_void>(),
            0usize,
            VERSION_FLAG,
        )
    };
    let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
    interpret(result, errno)
}

/// Ask this kernel.
#[cfg(not(target_os = "linux"))]
pub fn probe() -> Probe {
    Probe::Unavailable(Unavailable::NotLinux)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_positive_return_is_the_abi_version() {
        assert_eq!(interpret(1, 0), Probe::Available(1));
        assert_eq!(interpret(4, 0), Probe::Available(4));
        assert_eq!(interpret(4, 0).abi(), Some(4));
        assert!(interpret(4, 0).is_available());
    }

    #[test]
    fn a_zero_abi_is_not_treated_as_available() {
        // "Supported, version zero" does not exist. Accepting it would have the caller
        // build a ruleset against a version the kernel never claimed.
        assert_eq!(
            interpret(0, 0),
            Probe::Unavailable(Unavailable::Unexpected(0))
        );
        assert!(!interpret(0, 0).is_available());
    }

    #[test]
    fn a_missing_syscall_and_a_disabled_lsm_are_different_problems() {
        // Both mean this node cannot confine a filesystem. They mean very different
        // things to whoever has to fix it: one needs a newer kernel, the other needs a
        // boot parameter.
        let old_kernel = interpret(-1, 38);
        assert_eq!(old_kernel, Probe::Unavailable(Unavailable::NoSuchSyscall));
        assert!(old_kernel.explain().contains("5.13"));

        let disabled = interpret(-1, 95);
        assert_eq!(disabled, Probe::Unavailable(Unavailable::DisabledInKernel));
        assert!(disabled.explain().contains("lsm="));

        assert_ne!(old_kernel, disabled);
        assert!(!old_kernel.is_available() && !disabled.is_available());
    }

    #[test]
    fn an_unrecognised_errno_is_kept_rather_than_guessed_at() {
        // Mapping an unknown failure onto a known one would send an operator to check a
        // boot parameter that was never the problem.
        assert_eq!(
            interpret(-1, 13),
            Probe::Unavailable(Unavailable::Unexpected(13))
        );
        assert!(interpret(-1, 13).explain().contains("errno 13"));
    }

    #[test]
    fn nothing_available_ever_reports_an_abi() {
        for unavailable in [
            Unavailable::NoSuchSyscall,
            Unavailable::DisabledInKernel,
            Unavailable::NotLinux,
            Unavailable::Unexpected(99),
        ] {
            let probe = Probe::Unavailable(unavailable);
            assert_eq!(probe.abi(), None);
            assert!(!probe.explain().is_empty());
        }
    }

    #[test]
    fn probing_this_host_answers_without_panicking() {
        // On Linux this is the real syscall; everywhere else it is NotLinux. Either way
        // it must answer, because the caller's next decision is whether a task may run.
        let probe = probe();
        match &probe {
            Probe::Available(abi) => assert!(*abi >= 1),
            Probe::Unavailable(_) => assert!(!probe.explain().is_empty()),
        }

        #[cfg(not(target_os = "linux"))]
        assert_eq!(probe, Probe::Unavailable(Unavailable::NotLinux));
    }
}
