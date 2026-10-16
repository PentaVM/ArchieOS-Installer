//! Print driver planning from IPP Get-Printer-Attributes data.
//!
//! This module only plans. It never installs or executes anything:
//! `pacman_command()` returns the invocation as data, and no code path here
//! touches `std::process`. Executing a plan belongs to a separate,
//! policy-gated, Linux-only installation path that does not exist yet.
use super::ipp::PrinterSnapshot;

/// Hard bound on the number of packages in any plan.
const MAX_PACKAGES: usize = 64;

/// Arch package name charset: starts alphanumeric, then lowercase
/// alphanumerics plus `@`, `.`, `_`, `+`, `-` (pacman(8) naming rules).
fn valid_package(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphanumeric())
        && name.bytes().all(|b| {
            b.is_ascii_lowercase()
                || b.is_ascii_digit()
                || matches!(b, b'@' | b'.' | b'_' | b'+' | b'-')
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverTier {
    /// IPP Everywhere / AirPrint: the printer declared a driverless raster
    /// format (`image/pwg-raster` or `image/urf`), so CUPS needs no driver.
    Driverless,
    /// Legacy hardware: named Arch packages would be required. `reason`
    /// explains each inclusion for an operator; it is not a receipt.
    NeedsPackages {
        packages: Vec<String>,
        reason: String,
    },
    /// The attributes match no known case. `considered` lists the exact
    /// attribute values the decision was based on, for manual review.
    Unknown { considered: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverPlan {
    pub tier: DriverTier,
}
impl DriverPlan {
    /// The planned installation command, as data. This module NEVER executes
    /// it; running it belongs to a future policy-gated, Linux-only path.
    /// Returns None for driverless and unknown printers, and defensively for
    /// any package list that fails validation or exceeds the bound.
    pub fn pacman_command(&self) -> Option<Vec<String>> {
        match &self.tier {
            DriverTier::NeedsPackages { packages, .. }
                if !packages.is_empty()
                    && packages.len() <= MAX_PACKAGES
                    && packages.iter().all(|p| valid_package(p)) =>
            {
                let mut command = vec![
                    "pacman".to_owned(),
                    "-S".to_owned(),
                    "--needed".to_owned(),
                    "--noconfirm".to_owned(),
                ];
                command.extend(packages.iter().cloned());
                Some(command)
            }
            _ => None,
        }
    }
    /// Human-readable rendering of the plan for display to an operator.
    pub fn planned_command_line(&self) -> String {
        match self.pacman_command() {
            Some(command) => command.join(" "),
            None => match &self.tier {
                DriverTier::Driverless => {
                    "driverless printing (IPP Everywhere / AirPrint): no packages required"
                        .to_owned()
                }
                DriverTier::NeedsPackages { .. } => {
                    "package list failed validation: manual review required".to_owned()
                }
                DriverTier::Unknown { considered } => format!(
                    "unknown printer: no driver plan; attributes considered: {considered:?}"
                ),
            },
        }
    }
}

fn push_package(packages: &mut Vec<String>, name: &str) {
    if packages.len() < MAX_PACKAGES && valid_package(name) && !packages.iter().any(|p| p == name) {
        packages.push(name.to_owned());
    }
}

/// Decide the driver tier from Get-Printer-Attributes data. Driverless is
/// always preferred: package-based tiers are only reached when the printer
/// declared neither `image/pwg-raster` nor `image/urf`.
pub fn analyze(attributes: &PrinterSnapshot) -> DriverPlan {
    let formats = &attributes.document_formats;
    if formats
        .iter()
        .any(|f| matches!(f.as_str(), "image/pwg-raster" | "image/urf"))
    {
        return DriverPlan {
            tier: DriverTier::Driverless,
        };
    }
    let make_model = attributes
        .make_model
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();
    let mut considered: Vec<String> = formats.clone();
    if let Some(make_model) = &attributes.make_model {
        considered.push(format!("printer-make-and-model={make_model}"));
    }
    let legacy_pdl = formats.iter().any(|f| {
        matches!(
            f.as_str(),
            "application/postscript"
                | "application/vnd.hp-pcl"
                | "application/vnd.pcl"
                | "application/vnd.pcl-xl"
                | "application/pcl"
        )
    });
    let scanner = make_model.contains("scanner")
        || make_model.contains("mfp")
        || make_model.contains("scanjet");
    let hp = make_model.contains("hp") || make_model.contains("hewlett");
    let mut packages = vec![];
    let mut reasons = vec![];
    if legacy_pdl {
        for package in [
            "cups-filters",
            "gutenprint",
            "foomatic-db",
            "foomatic-db-engine",
        ] {
            push_package(&mut packages, package);
        }
        reasons.push("legacy page description language requires CUPS filters".to_owned());
    }
    if scanner {
        push_package(&mut packages, "sane");
        reasons.push("scanner function requires sane".to_owned());
    }
    if hp {
        // Driverless was already ruled out above; hplip is the HP fallback.
        push_package(&mut packages, "hplip");
        reasons.push(
            "hplip is optional; prefer driverless operation where the printer allows it".to_owned(),
        );
    }
    if packages.is_empty() {
        DriverPlan {
            tier: DriverTier::Unknown { considered },
        }
    } else {
        DriverPlan {
            tier: DriverTier::NeedsPackages {
                packages,
                reason: reasons.join("; "),
            },
        }
    }
}
