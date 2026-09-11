//! The ArchieOS installer.
//!
//! An installer is mostly a wizard, and a wizard is untestable: it prompts, it reads a
//! terminal, it needs a disk. But almost none of what an installer gets wrong is in the
//! prompting. It is in the decisions — which disk may be erased, which fabric the machine
//! ends up in, what the partition on an NVMe drive is called, whether the live medium
//! turns up in a list of targets.
//!
//! Those are pure, and they are here. What is left in `main.rs` is asking and doing:
//! roughly two hundred lines that need a real machine, wrapped around a library that
//! does not.
//!
//! The fabric decision lives one crate over, in `archie_fabric::enrolment_plan`, because
//! it is the same decision `archie enrol` makes later and there should be one of it.

pub mod answers;
pub mod config;
pub mod disk;
pub mod mapdata;
pub mod plan;
pub mod probe;

pub use answers::{validate_hostname, validate_username, Answers, InvalidAnswer};
pub use disk::{
    assess, partition_path, survey, BlockDevice, Survey, Unsuitable, MINIMUM_TARGET_BYTES,
};
pub use mapdata::{assess as assess_map, MapChoice, MapProblem, MapRegion};
pub use plan::{build, Action, IdentityFiles, InstallPlan, Step, TARGET_ROOT};
pub use probe::{parse, ProbeError, LSBLK_ARGV};

pub mod onboarding;

pub mod map_cache;
