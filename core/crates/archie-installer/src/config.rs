//! The `archie.toml` the install writes.
//!
//! Generated with its comments intact. `/etc/archie/archie.toml` is a file a person edits
//! later, on a machine they may have installed a year earlier, and a bare table of values
//! tells them what is set without telling them what any of it means. The comments are the
//! difference between a file somebody edits and a file somebody replaces.
//!
//! The text is generated rather than serialised because `toml::to_string` cannot keep
//! comments. That trades a guarantee for readability, so the guarantee is bought back in
//! tests: every generated file is parsed by the real `ArchieConfig` loader, which is the
//! same code the daemon runs at boot.

use crate::answers::Answers;
use archie_types::NodeProfile;

/// Write the configuration for an installed node.
///
/// `fabric_fingerprint` is included as a comment only. It is not configuration — the
/// fabric a node belongs to is decided by the certificate in `identity/membership.json`
/// and cannot be changed by editing a text file. It is written here because somebody
/// looking at two machines needs a way to see, without running anything, whether they
/// are in the same fabric.
pub fn render(answers: &Answers, fabric_fingerprint: &str, map_data: Option<&str>) -> String {
    let profile = answers.profile().as_str();
    let cortex = if answers.enable_ai_module() {
        "enabled at install time"
    } else {
        "declined at install time"
    };

    format!(
        "\
# ArchieOS node configuration, written by the installer for {hostname}.
#
# This node belongs to fabric {fabric_fingerprint}.
# That is not set here: it comes from the certificate in
# /var/lib/archie/identity/membership.json, and editing this file cannot change it.
# It is written down so two machines can be compared without running anything.

[node]
# Chosen at install time, not inferred from hardware: a workstation with a large GPU is
# still a workstation. A dev-vm is never trusted to hold the only copy of anything.
profile = \"{profile}\"
extra_capabilities = []

[fabric]
transport = \"udp\"
bind = \"0.0.0.0:7654\"
# Peers found by multicast on the local network. Packets from machines in other fabrics
# arrive and are refused; belonging to no fabric is not belonging to every fabric.
seeds = []
multicast_group = \"239.255.42.1\"
announce_interval_ms = 5000
# At least twice the announce interval, or one lost packet evicts a healthy peer.
peer_timeout_ms = 20000
peer_forget_ms = 3600000

[scheduler]
slots_per_node = 4
reserved_active_slots = 1

[policy]
# Set this true only on a node wired to something that can move, heat or energise. It
# gates the operations that have physical consequences.
safety_boundary_present = false
max_lease_lifetime_ms = 3600000

[world]
# Where the vector map was downloaded, when one was. Absent means the globe uses only
# the imagery compiled into the display, which needs no network and is not a fault.
{world_line}
[modules]
# The Analytical Cortex was {cortex}; it is a systemd unit, so `systemctl status
# archie-cortex.socket` is the answer to whether it is running now.
disabled = []
grant_permissions = []
",
        hostname = answers.hostname(),
        world_line = match map_data {
            Some(path) => format!(
                "map_data = \"{path}\"
"
            ),
            None => String::new(),
        },
    )
}

/// The profiles offered by the wizard, in the order they are shown.
///
/// `Workstation` is first because it is what somebody installing this on the machine in
/// front of them is doing, and a menu whose first entry is right for most people is a
/// menu most people answer correctly.
pub const OFFERED_PROFILES: [NodeProfile; 5] = [
    NodeProfile::Workstation,
    NodeProfile::Server,
    NodeProfile::BrainNode,
    NodeProfile::EdgeGateway,
    NodeProfile::DevVm,
];

/// One line describing what a profile is for, shown beside it in the menu.
pub fn describe(profile: NodeProfile) -> &'static str {
    match profile {
        NodeProfile::Workstation => {
            "this machine, with a screen. Runs the observatory and can also do work."
        }
        NodeProfile::Server => "headless. Owns the task graph and coordinates the others.",
        NodeProfile::BrainNode => "a box with accelerators in it, for inference.",
        NodeProfile::EdgeGateway => "small and always on, bridging to devices and radios.",
        NodeProfile::DevVm => "throwaway. Never trusted to hold the only copy of anything.",
    }
}
