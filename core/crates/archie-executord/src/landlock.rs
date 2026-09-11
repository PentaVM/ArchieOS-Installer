//! Landlock: what the kernel will actually enforce, and never more than that.
//!
//! RFC-007 §7, step 7 and §3.4. This runs in `archie-task-runner`, not in `archied` and
//! not in the executor — Landlock restrictions are irreversible for a process tree, so
//! whatever applies them must be the process about to *become* the worker.
//!
//! # The ABI is a negotiation, and losing it honestly is the point
//!
//! Landlock has shipped in versions. ABI 1 (5.13) restricts filesystem access. ABI 2
//! (5.19) adds refer, ABI 3 (6.2) truncate, ABI 4 (6.7) TCP bind and connect. A node
//! running an older kernel supports a strict prefix of what a newer one does.
//!
//! The failure mode this module exists to prevent is not "the sandbox is weaker on old
//! kernels". That is unavoidable. It is **claiming otherwise**: reporting network
//! restriction on a kernel with ABI 2, so a task whose manifest requires it is admitted
//! and runs with unrestricted sockets. A weaker sandbox is a known risk. A sandbox that
//! lies about its strength is an unknown one, and the manifest requirement was written
//! precisely so someone would not have to find out by accident.
//!
//! So [`Enforcement::for_abi`] reports what the ABI genuinely provides, and
//! [`plan`] refuses rather than downgrading when a requirement cannot be met.

use archie_execution::spec::{FilesystemGrant, NetworkGrant, RequiredEnforcement};

/// The Landlock ABI a kernel reports.
///
/// `None` means Landlock is unavailable — not compiled in, or disabled at boot. Which of
/// those it is matters to an operator, but not to the decision made here.
pub type Abi = Option<u32>;

/// The oldest ABI worth using. Below this there is no filesystem restriction at all, so
/// there is nothing to apply.
pub const MINIMUM_USEFUL_ABI: u32 = 1;

/// The ABI at which network restriction becomes available.
pub const NETWORK_ABI: u32 = 4;

/// What a given ABI can actually enforce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Enforcement {
    pub filesystem: bool,
    /// Restricting the creation of hard links and renames across directories. ABI 2.
    pub refer: bool,
    /// Restricting file truncation. ABI 3. Its absence is worth knowing: without it a
    /// worker with read-only access to a file can still empty it.
    pub truncate: bool,
    /// Restricting TCP bind and connect. ABI 4.
    pub network: bool,
}

impl Enforcement {
    /// What this ABI provides.
    ///
    /// Deliberately cumulative and deliberately conservative: an unknown future ABI is
    /// treated as providing what the newest one we know about provides, never more. A
    /// kernel newer than this code may enforce more, and assuming it does would be the
    /// same lie in the other direction.
    pub fn for_abi(abi: Abi) -> Self {
        let Some(abi) = abi else {
            return Self::default();
        };
        if abi < MINIMUM_USEFUL_ABI {
            return Self::default();
        }
        Self {
            filesystem: true,
            refer: abi >= 2,
            truncate: abi >= 3,
            network: abi >= NETWORK_ABI,
        }
    }

    pub fn enforces_anything(&self) -> bool {
        self.filesystem || self.refer || self.truncate || self.network
    }
}

/// Why a sandbox could not be applied as specified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxRefusal {
    /// The task requires filesystem confinement and this kernel has no Landlock.
    FilesystemUnavailable { abi: Abi },
    /// The task requires network restriction and this ABI cannot provide it.
    ///
    /// Reported separately from the filesystem case because it is the one most likely to
    /// pass unnoticed: a kernel with ABI 1-3 confines the filesystem perfectly well and
    /// silently ignores every network rule.
    NetworkUnavailable { abi: Abi, needed: u32 },
    /// A grant names a path that cannot be confined.
    UnusableGrant(String),
}

impl std::fmt::Display for SandboxRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FilesystemUnavailable { abi } => write!(
                f,
                "this task requires filesystem confinement and this kernel reports \
                 Landlock ABI {abi:?}; it will not be run here unconfined"
            ),
            Self::NetworkUnavailable { abi, needed } => write!(
                f,
                "this task requires network restriction, which needs Landlock ABI \
                 {needed}; this kernel reports {abi:?}, where network rules are accepted \
                 and silently ignored"
            ),
            Self::UnusableGrant(detail) => write!(f, "unusable filesystem grant: {detail}"),
        }
    }
}

impl std::error::Error for SandboxRefusal {}

/// One filesystem rule, as the ruleset will receive it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathRule {
    pub path: String,
    /// Read the file's contents, and list a directory.
    pub read: bool,
    /// Write contents, create and remove entries.
    pub write: bool,
    pub execute: bool,
    /// Whether truncation is permitted. Only meaningful from ABI 3; below it, truncation
    /// is unrestricted whatever this says.
    pub truncate: bool,
}

/// A sandbox ready to be applied, and an honest account of what it will and will not do.
#[derive(Debug, Clone, PartialEq)]
pub struct SandboxPlan {
    pub abi: Abi,
    pub enforcement: Enforcement,
    pub rules: Vec<PathRule>,
    /// Whether outbound TCP will actually be blocked. False on a kernel below ABI 4 even
    /// when the spec asked for no network — which is exactly why a task that *requires*
    /// it is refused rather than given this plan.
    pub network_blocked: bool,
    /// Things this plan does not enforce, in words. Attached so the fact travels with the
    /// plan into logs and audit rather than living only in whoever read the ABI.
    pub caveats: Vec<String>,
}

impl SandboxPlan {
    /// Whether this plan is worth applying at all.
    pub fn enforces_anything(&self) -> bool {
        self.enforcement.enforces_anything() && !self.rules.is_empty()
    }
}

/// Build a sandbox plan, or refuse.
///
/// Refusal is the load-bearing behaviour. A task whose manifest requires confinement runs
/// with it or does not run here — the scheduler can place it on a node with a newer
/// kernel, and that is a far better outcome than an untrusted worker running unconfined
/// because nobody checked a version number.
pub fn plan(
    abi: Abi,
    grants: &[FilesystemGrant],
    network: NetworkGrant,
    required: &RequiredEnforcement,
) -> Result<SandboxPlan, SandboxRefusal> {
    let enforcement = Enforcement::for_abi(abi);

    if required.filesystem_sandbox && !enforcement.filesystem {
        return Err(SandboxRefusal::FilesystemUnavailable { abi });
    }

    // A task that requires confinement and asked for no network needs the ABI that can
    // actually deny sockets. Below it, the rules are accepted and ignored — which is the
    // most dangerous possible combination, because everything looks configured.
    let wants_network_denied = network == NetworkGrant::None;
    if required.filesystem_sandbox && wants_network_denied && !enforcement.network {
        return Err(SandboxRefusal::NetworkUnavailable {
            abi,
            needed: NETWORK_ABI,
        });
    }

    let mut rules = Vec::with_capacity(grants.len());
    for grant in grants {
        let path = grant
            .path
            .to_str()
            .ok_or_else(|| SandboxRefusal::UnusableGrant(grant.path.display().to_string()))?;
        if !grant.grants_anything() {
            return Err(SandboxRefusal::UnusableGrant(format!(
                "{path} permits nothing"
            )));
        }
        rules.push(PathRule {
            path: path.to_string(),
            read: grant.read,
            write: grant.write,
            execute: grant.execute,
            // Truncation follows write access: a grant that permits writing a file has
            // no reason to forbid emptying it, and a read-only grant has every reason to.
            truncate: grant.write,
        });
    }

    let mut caveats = Vec::new();
    if !enforcement.truncate {
        caveats.push(
            "kernel is below Landlock ABI 3: a worker with read-only access to a file can \
             still truncate it"
                .to_string(),
        );
    }
    if !enforcement.refer {
        caveats.push(
            "kernel is below Landlock ABI 2: hard links and cross-directory renames are \
             unrestricted"
                .to_string(),
        );
    }
    if !enforcement.network {
        caveats.push(format!(
            "kernel is below Landlock ABI {NETWORK_ABI}: network access is not restricted \
             by this sandbox, whatever the task's grant says"
        ));
    }

    Ok(SandboxPlan {
        abi,
        enforcement,
        rules,
        network_blocked: enforcement.network && wants_network_denied,
        caveats,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grants() -> Vec<FilesystemGrant> {
        vec![
            FilesystemGrant::read_only("/usr/lib/archie"),
            FilesystemGrant::read_write("/var/lib/archie/work"),
        ]
    }

    #[test]
    fn an_older_abi_provides_a_strict_prefix_of_a_newer_one() {
        assert_eq!(Enforcement::for_abi(None), Enforcement::default());
        assert!(!Enforcement::for_abi(None).enforces_anything());

        let one = Enforcement::for_abi(Some(1));
        assert!(one.filesystem && !one.refer && !one.truncate && !one.network);

        let three = Enforcement::for_abi(Some(3));
        assert!(three.filesystem && three.refer && three.truncate && !three.network);

        let four = Enforcement::for_abi(Some(4));
        assert!(four.network);
    }

    #[test]
    fn an_unknown_future_abi_is_not_assumed_to_do_more() {
        // A kernel newer than this code may enforce more. Assuming it does is the same
        // lie as assuming an old one does — just in the other direction.
        assert_eq!(
            Enforcement::for_abi(Some(99)),
            Enforcement::for_abi(Some(4))
        );
    }

    #[test]
    fn a_task_requiring_confinement_is_refused_on_a_kernel_without_landlock() {
        // The whole point. Running an untrusted worker unconfined because nobody checked
        // a version number is the outcome this prevents.
        let err = plan(
            None,
            &grants(),
            NetworkGrant::None,
            &RequiredEnforcement::untrusted(),
        )
        .unwrap_err();

        assert_eq!(err, SandboxRefusal::FilesystemUnavailable { abi: None });
        assert!(err.to_string().contains("will not be run here unconfined"));
    }

    #[test]
    fn network_restriction_is_refused_separately_because_it_fails_quietly() {
        // A kernel with ABI 1-3 confines the filesystem perfectly well and silently
        // ignores every network rule. Everything looks configured, which is the most
        // dangerous combination available.
        for abi in [1, 2, 3] {
            let err = plan(
                Some(abi),
                &grants(),
                NetworkGrant::None,
                &RequiredEnforcement::untrusted(),
            )
            .unwrap_err();
            assert_eq!(
                err,
                SandboxRefusal::NetworkUnavailable {
                    abi: Some(abi),
                    needed: NETWORK_ABI
                }
            );
            assert!(err.to_string().contains("silently ignored"));
        }
    }

    #[test]
    fn a_kernel_that_can_do_it_all_produces_a_plan_with_no_caveats() {
        let plan = plan(
            Some(4),
            &grants(),
            NetworkGrant::None,
            &RequiredEnforcement::untrusted(),
        )
        .unwrap();

        assert!(plan.caveats.is_empty());
        assert!(plan.network_blocked);
        assert!(plan.enforces_anything());
        assert_eq!(plan.rules.len(), 2);
    }

    #[test]
    fn a_task_that_requires_nothing_still_gets_what_the_kernel_can_give() {
        // Confinement is not withheld from a trusted worker just because it did not
        // demand it; requirements govern refusal, not application.
        let plan = plan(
            Some(1),
            &grants(),
            NetworkGrant::None,
            &RequiredEnforcement::none(),
        )
        .unwrap();

        assert!(plan.enforcement.filesystem);
        assert_eq!(plan.rules.len(), 2);
        assert!(
            !plan.network_blocked,
            "ABI 1 cannot deny sockets, and the plan says so rather than implying it did"
        );
    }

    #[test]
    fn every_shortfall_is_written_into_the_plan_not_left_in_the_reader() {
        // The caveats travel with the plan into logs and audit, so the fact does not
        // live only in whoever happened to read the ABI number.
        let plan = plan(
            Some(1),
            &grants(),
            NetworkGrant::None,
            &RequiredEnforcement::none(),
        )
        .unwrap();

        assert_eq!(plan.caveats.len(), 3);
        assert!(plan.caveats.iter().any(|c| c.contains("truncate it")));
        assert!(plan.caveats.iter().any(|c| c.contains("hard links")));
        assert!(plan.caveats.iter().any(|c| c.contains("network access")));
    }

    #[test]
    fn a_read_only_grant_forbids_truncation_and_a_writable_one_does_not() {
        // Without this, a worker with read-only access to a file could still empty it —
        // which is most of the damage without any of the write permission.
        let plan = plan(
            Some(3),
            &grants(),
            NetworkGrant::Outbound,
            &RequiredEnforcement::none(),
        )
        .unwrap();

        let read_only = plan
            .rules
            .iter()
            .find(|r| r.path.contains("usr/lib"))
            .unwrap();
        assert!(read_only.read && !read_only.write);
        assert!(
            !read_only.truncate,
            "a read-only grant has every reason to forbid emptying the file"
        );

        let writable = plan.rules.iter().find(|r| r.path.contains("work")).unwrap();
        assert!(writable.write && writable.truncate);
    }

    #[test]
    fn an_empty_grant_is_refused_rather_than_applied_as_a_very_strict_one() {
        let empty = vec![FilesystemGrant {
            path: std::path::PathBuf::from("/data"),
            read: false,
            write: false,
            execute: false,
        }];
        let err = plan(
            Some(4),
            &empty,
            NetworkGrant::Outbound,
            &RequiredEnforcement::none(),
        )
        .unwrap_err();
        assert!(matches!(err, SandboxRefusal::UnusableGrant(_)));
    }

    #[test]
    fn a_task_permitted_outbound_network_is_not_refused_for_lacking_abi_four() {
        // It never asked for sockets to be denied, so the ABI that denies them is not
        // required. Refusing here would strand tasks on perfectly adequate nodes.
        let plan = plan(
            Some(1),
            &grants(),
            NetworkGrant::Outbound,
            &RequiredEnforcement::untrusted(),
        )
        .unwrap();
        assert!(!plan.network_blocked);
    }

    #[test]
    fn a_plan_with_no_rules_enforces_nothing_and_says_so() {
        let plan = plan(
            Some(4),
            &[],
            NetworkGrant::Outbound,
            &RequiredEnforcement::none(),
        )
        .unwrap();
        assert!(
            !plan.enforces_anything(),
            "a ruleset with no paths confines nothing, and applying it would be theatre"
        );
    }
}
