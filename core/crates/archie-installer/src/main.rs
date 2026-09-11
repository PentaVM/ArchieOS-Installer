//! `archie-install` — the wizard.
//!
//! This file is the part that cannot be tested: it prompts, it reads a terminal, it runs
//! `sgdisk`. Everything it decides comes from `archie_installer`'s library half or from
//! `archie_fabric::enrolment_plan`, which is why this is short.
//!
//! The shape is deliberate. Ask everything, decide everything, show the whole plan, get
//! one confirmation, then act. An installer that interleaves questions with `mkfs` is one
//! where the answer to the fourth question arrives after the disk is already gone.

#[cfg(unix)]
fn main() -> std::process::ExitCode {
    match wizard::run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("\nInstallation stopped: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// The installer partitions disks and writes to `/`. It exists to run from the live ISO,
/// and on anything else the honest thing is to say so rather than to half-work.
#[cfg(not(unix))]
fn main() -> std::process::ExitCode {
    eprintln!(
        "archie-install runs on the ArchieOS live image. The decisions it makes are in \
         the archie-installer library and are testable anywhere; this binary is the part \
         that needs a real disk."
    );
    std::process::ExitCode::FAILURE
}

#[cfg(unix)]
mod wizard {
    use archie_core::config::ArchieConfig;
    use archie_fabric::{fingerprint_of_owner_key, plan_enrolment, FabricChoice, NodeIdentity};
    use archie_installer::mapdata;
    use archie_installer::plan::{Action, IdentityFiles};
    use archie_installer::{
        config, disk, plan, probe, Answers, BlockDevice, InstallPlan, MINIMUM_TARGET_BYTES,
    };
    use archie_types::NodeProfile;
    use std::io::{BufRead, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::process::{Command, Stdio};

    /// Where keys live between being generated and being installed.
    ///
    /// A tmpfs. The live medium is read-only and this never reaches it; on a machine that
    /// aborts halfway the keys are in RAM and are gone at the next power cycle.
    const STAGING: &str = "/run/archie-install";

    /// Room the installed system itself needs before anything optional is added.
    /// Deliberately generous: refusing a map that would have fitted is an annoyance,
    /// and filling the disk of a machine somebody just installed is not.
    const SYSTEM_RESERVE: u64 = 12_000_000_000;

    type Fallible = Result<(), Box<dyn std::error::Error>>;

    pub fn run() -> Fallible {
        require_root()?;
        let argv: Vec<String> = std::env::args().collect();
        let profile = match argv.as_slice() {
            [_] => None,
            [_, flag, path] if flag == "--profile" => {
                let file = std::fs::File::open(path)?;
                let mut bytes = Vec::new();
                std::io::Read::read_to_end(&mut std::io::Read::take(file, 65537), &mut bytes)?;
                Some(archie_installer::onboarding::Profile::parse(&bytes)?)
            }
            _ => return Err("usage: archie-install [--profile <wizard.json>]".into()),
        };

        println!("\nArchieOS installer\n==================\n");

        if !Path::new("/run/archiso/airootfs/etc/os-release").is_file() {
            return Err("the live source filesystem is missing; no disk was changed".into());
        }
        if !Path::new("/sys/firmware/efi").is_dir() {
            return Err("restart the installation USB in UEFI mode before installing".into());
        }
        for program in [
            "sgdisk",
            "partprobe",
            "mkfs.fat",
            "mkfs.ext4",
            "mount",
            "mkdir",
            "rsync",
            "sh",
            "genfstab",
            "arch-chroot",
            "chpasswd",
            "useradd",
            "systemctl",
            "bootctl",
            "mkinitcpio",
            "umount",
        ] {
            let available = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                .any(|dir| {
                    std::fs::metadata(dir.join(program))
                        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
                });
            if !available {
                return Err(
                    format!("missing installer command {program}; no disk was changed").into(),
                );
            }
        }
        let target = choose_disk()?;
        let answers = if let Some(p) = &profile {
            Answers::new(&p.hostname, &p.username, NodeProfile::Workstation, true)
                .map_err(|e| e.to_string())?
        } else {
            ask_answers()?
        };
        let map_volume = profile
            .as_ref()
            .and_then(|p| p.map.as_ref())
            .and_then(|m| m.partition.as_ref())
            .map(|device| inspect_map_volume(device, &target))
            .transpose()?;
        let map = if let Some(p) = &profile {
            p.map_choice()
        } else {
            ask_map(&target)?
        };
        if let mapdata::MapChoice::Download {
            region,
            destination,
        } = &map
        {
            if !mapdata::safe_source(&region.url)
                || !mapdata::safe_destination(&destination.to_string_lossy())
            {
                return Err("invalid map source or destination".into());
            }
            let retained = map_volume
                .as_ref()
                .map(|v| {
                    archie_installer::map_cache::inspect_cache(&region.url, Path::new(&v.2))
                        .map_err(|e| e.to_string())
                })
                .transpose()?
                .flatten();
            let required_download = if retained.is_some() {
                println!(
                    "Verified map already present on the retained volume; no download required."
                );
                Some(0)
            } else {
                content_length(&region.url)
            };
            mapdata::assess(
                &map,
                Path::new(plan::TARGET_ROOT),
                required_download,
                map_volume.as_ref().map_or(
                    target
                        .size_bytes
                        .saturating_sub(plan::ESP_BYTES + SYSTEM_RESERVE),
                    |v| v.1,
                ),
            )
            .map_err(|e| e.to_string())?;
        }
        let password = ask_password(answers.username())?;
        let (identity, fabric_fingerprint, started_new_fabric) = decide_fabric()?;

        // The installed system is told where the map went, so something can find it.
        let map_path = match &map {
            mapdata::MapChoice::Download {
                region,
                destination,
            } => Some(format!("{}/{}", destination.display(), region.file_name())),
            mapdata::MapChoice::Skip => None,
        };
        let mut config_toml = config::render(&answers, &fabric_fingerprint, map_path.as_deref());
        if let Some(location) = profile.as_ref().and_then(|p| p.location.as_ref()) {
            config_toml.push_str(&format!(
                "\n[node.location]\nlatitude = {}\nlongitude = {}\n",
                location.latitude, location.longitude
            ));
        }
        if let Some(p) = &profile {
            config_toml.push_str(&p.owner_toml()?);
        }
        // The generated file is parsed before it is written. A node that will not boot
        // because its own installer produced configuration the daemon rejects is a
        // failure that should happen here, in a second, and not on the next power cycle.
        ArchieConfig::from_toml(&config_toml).map_err(|e| {
            format!("the installer generated configuration it cannot itself parse: {e}")
        })?;

        let mut install = plan::build(&target, &answers, &identity, &config_toml, &map);
        if let Some(package) = profile.as_ref().and_then(|p| p.map.as_ref()) {
            for step in &mut install.steps {
                if let Action::Run { argv } = &mut step.action {
                    if argv.first().is_some_and(|arg| arg == "archie-map-fetch") {
                        for mirror in &package.mirrors {
                            argv.extend(["--mirror".into(), mirror.clone()]);
                        }
                    }
                }
            }
        }
        if let Some((device, _, _)) = &map_volume {
            if let mapdata::MapChoice::Download { destination, .. } = &map {
                plan::attach_map_volume(&mut install, device, &destination.to_string_lossy())?;
            }
        }
        if let Some(profile) = &profile {
            let index = install.steps.len().saturating_sub(1);
            install.steps.insert(
                index,
                plan::Step {
                    explain: "Save owner preferences and explicit perception consent".into(),
                    action: Action::Write {
                        path: "/etc/archie/onboarding.json".into(),
                        contents: profile.preferences_json()?,
                        mode: 0o600,
                    },
                    destroys_data: false,
                },
            );
        }

        show(&install, &fabric_fingerprint, started_new_fabric);
        if !confirm(&install)? {
            println!("\nNothing was written. The disk is as it was.");
            return Ok(());
        }

        // Re-read the target immediately before the first destructive step.
        let output = Command::new(probe::LSBLK_ARGV[0])
            .args(&probe::LSBLK_ARGV[1..])
            .output()?;
        if !output.status.success() {
            return Err("disk re-probe failed".into());
        }
        let devices = probe::parse(&String::from_utf8_lossy(&output.stdout))?;
        if !devices
            .iter()
            .any(|d| d == &target && disk::assess(d, MINIMUM_TARGET_BYTES).is_ok())
        {
            return Err("disk changed after review; restart the installer".into());
        }
        if let Some((device, _, _)) = &map_volume {
            inspect_map_volume(device, &target)?;
        }
        execute(&install, &password)?;

        println!("\nDone. Remove the installation medium and restart.");
        if started_new_fabric {
            println!(
                "\nThis machine owns fabric {fabric_fingerprint}. The key that admits other\n\
                 machines to it is at /var/lib/archie/identity/owner.key. Copy it somewhere\n\
                 safe: the fabric keeps working without it, but nothing new can join."
            );
        }
        Ok(())
    }

    fn inspect_map_volume(
        device: &str,
        target: &BlockDevice,
    ) -> Result<(String, u64, String), Box<dyn std::error::Error>> {
        if !mapdata::safe_device(device) {
            return Err("invalid map device".into());
        }
        let facts = Command::new("lsblk")
            .args(["-J", "-b", "-o", "PATH,TYPE,PKNAME,FSTYPE,RO", device])
            .output()?;
        if !facts.status.success() {
            return Err("map device probe failed".into());
        }
        let tree: serde_json::Value = serde_json::from_slice(&facts.stdout)?;
        let row = &tree["blockdevices"][0];
        let parent = format!("/dev/{}", row["pkname"].as_str().unwrap_or_default());
        if row["path"].as_str() != Some(device)
            || row["type"] != "part"
            || row["ro"].as_bool().unwrap_or(true)
            || !["ext4", "btrfs"].contains(&row["fstype"].as_str().unwrap_or_default())
            || parent == target.path
        {
            return Err("choose an existing ext4/btrfs partition on another disk".into());
        }
        let all = Command::new(probe::LSBLK_ARGV[0])
            .args(&probe::LSBLK_ARGV[1..])
            .output()?;
        if !all.status.success() {
            return Err("map parent probe failed".into());
        }
        let devices = probe::parse(&String::from_utf8_lossy(&all.stdout))?;
        if !devices
            .iter()
            .any(|d| d.path == parent && !d.carries_live_medium && !d.read_only)
        {
            return Err("map partition belongs to the live medium or an unknown disk".into());
        }
        // The user mounts/selects the filesystem in the live environment first.
        // df measures free space in that filesystem, never total partition capacity.
        let mounted = Command::new("findmnt")
            .args(["--json", "-o", "TARGET", "--source", device])
            .output()?;
        if !mounted.status.success() {
            return Err("mount the map partition in the live environment first".into());
        }
        let mounts: serde_json::Value = serde_json::from_slice(&mounted.stdout)?;
        let mount = mounts["filesystems"][0]["target"]
            .as_str()
            .ok_or("no map mount")?;
        let free = Command::new("df")
            .args(["-B1", "--output=avail", "--", &mount])
            .output()?;
        if !free.status.success() {
            return Err("cannot measure map filesystem space".into());
        }
        let bytes = String::from_utf8(free.stdout)?
            .split_whitespace()
            .last()
            .ok_or("missing free space")?
            .parse::<u64>()?;
        Ok((device.into(), bytes, mount.into()))
    }

    fn require_root() -> Fallible {
        let out = Command::new("id").arg("-u").output()?;
        if String::from_utf8_lossy(&out.stdout).trim() != "0" {
            return Err("the installer must run as root; try `sudo archie-install`".into());
        }
        Ok(())
    }

    // ---- asking -----------------------------------------------------------------

    fn prompt(question: &str, default: Option<&str>) -> Result<String, std::io::Error> {
        loop {
            match default {
                Some(d) => print!("{question} [{d}]: "),
                None => print!("{question}: "),
            }
            std::io::stdout().flush()?;
            let mut line = String::new();
            std::io::stdin().lock().read_line(&mut line)?;
            let answer = line.trim().to_string();
            if answer.is_empty() {
                if let Some(d) = default {
                    return Ok(d.to_string());
                }
                continue;
            }
            return Ok(answer);
        }
    }

    fn choose_disk() -> Result<BlockDevice, Box<dyn std::error::Error>> {
        let output = Command::new(probe::LSBLK_ARGV[0])
            .args(&probe::LSBLK_ARGV[1..])
            .output()?;
        let devices = probe::parse(&String::from_utf8_lossy(&output.stdout))?;
        let survey = disk::survey(&devices, MINIMUM_TARGET_BYTES);

        // Refusals are printed, not filtered away. Somebody who cannot find the disk they
        // expected goes looking for the flag that turns the safety off; somebody who is
        // told it is the stick they booted from stops looking.
        if !survey.refused.is_empty() {
            println!("Disks that will not be offered:\n");
            for (device, reason) in &survey.refused {
                println!("  {} ({}) — {reason}", device.path, device.human_size());
            }
            println!();
        }

        if survey.is_empty() {
            return Err(format!(
                "no disk here can be installed onto. ArchieOS needs {} GB.",
                MINIMUM_TARGET_BYTES / 1_000_000_000
            )
            .into());
        }

        println!("Install onto:\n");
        for (index, device) in survey.usable.iter().enumerate() {
            println!(
                "  {}) {}  {}  {}",
                index + 1,
                device.path,
                device.human_size(),
                device.model
            );
        }
        println!();

        loop {
            let answer = prompt("Disk", Some("1"))?;
            match answer.parse::<usize>() {
                Ok(n) if n >= 1 && n <= survey.usable.len() => {
                    return Ok(survey.usable[n - 1].clone())
                }
                _ => println!("Pick a number between 1 and {}.", survey.usable.len()),
            }
        }
    }

    fn ask_answers() -> Result<Answers, Box<dyn std::error::Error>> {
        println!("\nThis machine\n------------\n");
        for (index, profile) in config::OFFERED_PROFILES.iter().enumerate() {
            println!(
                "  {}) {:<12} {}",
                index + 1,
                profile.as_str(),
                config::describe(*profile)
            );
        }
        println!();
        let profile = loop {
            let answer = prompt("Role", Some("1"))?;
            match answer.parse::<usize>() {
                Ok(n) if n >= 1 && n <= config::OFFERED_PROFILES.len() => {
                    break config::OFFERED_PROFILES[n - 1]
                }
                _ => println!(
                    "Pick a number between 1 and {}.",
                    config::OFFERED_PROFILES.len()
                ),
            }
        };

        let ai = yes_no(
            "Run the Analytical Cortex on this machine? It is the reasoning service; \
             without it the node still works and does not think",
            profile != NodeProfile::DevVm,
        )?;

        // Validated at the prompt rather than three minutes into the install, which is
        // where `useradd` would otherwise report it.
        loop {
            let hostname = prompt("Hostname", Some("archie"))?;
            let username = prompt("Your username", None)?;
            match Answers::new(&hostname, &username, profile, ai) {
                Ok(answers) => return Ok(answers),
                Err(problem) => println!("  {problem}"),
            }
        }
    }

    /// Ask about the vector map.
    ///
    /// The globe ships with NASA imagery and needs no network. A vector map — buildings,
    /// roads, coastlines that stay sharp at any zoom — is far too large for an ISO, so it
    /// is fetched here, while there is a network and somebody who can say where it goes.
    ///
    /// Every refusal offers skipping, and skipping is the default. This is the one part
    /// of the install that depends on the outside world, so it is the one part that must
    /// never be able to stop the rest.
    fn ask_map(target: &BlockDevice) -> Result<mapdata::MapChoice, Box<dyn std::error::Error>> {
        println!(
            "
Map data
--------
"
        );
        println!(
            "The globe works offline with the imagery it ships with. A vector map adds
             buildings, roads and place names, comes from OpenStreetMap, and is too large
             to put on the installation medium — a country is hundreds of megabytes and a
             continent is several gigabytes.
"
        );
        if !yes_no("Download a vector map now", false)? {
            return Ok(mapdata::MapChoice::Skip);
        }

        let regions = mapdata::offered_regions();
        for (index, region) in regions.iter().enumerate() {
            println!("  {}) {}", index + 1, region.name);
        }
        println!(
            "  {}) somewhere else — paste a Geofabrik URL",
            regions.len() + 1
        );
        println!();

        let region = loop {
            let answer = prompt("Region", None)?;
            match answer.parse::<usize>() {
                Ok(n) if n >= 1 && n <= regions.len() => break regions[n - 1].clone(),
                Ok(n) if n == regions.len() + 1 => {
                    let url = prompt("URL of an .osm.pbf extract", None)?;
                    if !mapdata::safe_source(&url) {
                        println!("  that is not an https .osm.pbf extract");
                        continue;
                    }
                    break mapdata::MapRegion::new("custom", &url);
                }
                _ => println!("  pick a number between 1 and {}.", regions.len() + 1),
            }
        };

        // The size is asked of the server rather than assumed. An extract is rebuilt
        // daily and grows; a number compiled into the installer would be wrong within a
        // week, and wrong in the direction that promises a download fits when it does not.
        let reported = content_length(&region.url);
        match reported {
            Some(bytes) => println!(
                "
  {} is {}",
                region.name,
                mapdata::human_bytes(bytes)
            ),
            None => println!(
                "
  the server did not report a size for {} — it may be unreachable",
                region.name
            ),
        }

        loop {
            let destination = prompt(
                "Where should it live on the installed system",
                Some("/var/lib/archie/maps"),
            )?;
            let choice = mapdata::MapChoice::Download {
                region: region.clone(),
                destination: std::path::PathBuf::from(&destination),
            };

            // Free space on the partition about to be created, which is the whole disk
            // less what the system itself takes. Measured from the disk rather than from
            // the live medium: nothing is mounted at the destination yet.
            let available = target
                .size_bytes
                .saturating_sub(plan::ESP_BYTES + SYSTEM_RESERVE);
            match mapdata::assess(
                &choice,
                std::path::Path::new(plan::TARGET_ROOT),
                reported,
                available,
            ) {
                Ok(()) => return Ok(choice),
                Err(problem) => {
                    println!("  {problem}");
                    if yes_no("Continue without a map", true)? {
                        return Ok(mapdata::MapChoice::Skip);
                    }
                }
            }
        }
    }

    /// What the server says the file weighs, or nothing.
    ///
    /// `curl --head`, because the installer already needs curl to do the download and a
    /// second HTTP client would be a second thing that can be missing from the image.
    /// Any failure — no network, a redirect that loses the length, a server that will not
    /// say — is `None`, which the caller treats as "cannot check" rather than "fits".
    fn content_length(url: &str) -> Option<u64> {
        let output = Command::new("curl")
            .args([
                "--fail",
                "--silent",
                "--head",
                "--location",
                "--proto",
                "=https",
                "--proto-redir",
                "=https",
                "--max-time",
                "30",
                url,
            ])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.trim()
                    .eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<u64>().ok())?
            })
            // A redirect chain emits one set of headers per hop; the last is the file.
            .next_back()
    }

    fn yes_no(question: &str, default: bool) -> Result<bool, std::io::Error> {
        let hint = if default { "Y/n" } else { "y/N" };
        loop {
            print!("{question}? [{hint}]: ");
            std::io::stdout().flush()?;
            let mut line = String::new();
            std::io::stdin().lock().read_line(&mut line)?;
            match line.trim().to_ascii_lowercase().as_str() {
                "" => return Ok(default),
                "y" | "yes" => return Ok(true),
                "n" | "no" => return Ok(false),
                _ => {}
            }
        }
    }

    /// Read a password without echoing it.
    ///
    /// Done with `stty` rather than a crate: the installer is the one program that must
    /// work on a machine with nothing installed on it, and a terminal is one of the few
    /// things the live image is guaranteed to have. The echo is restored on every path
    /// out, including the one where the read fails, because a terminal left with echo off
    /// is a machine somebody has to reboot.
    fn ask_password(username: &str) -> Result<String, Box<dyn std::error::Error>> {
        loop {
            let first = read_hidden(&format!("Password for {username}"))?;
            if first.is_empty() {
                println!(
                    "  An account with no password can be logged into by anyone at the keyboard."
                );
                continue;
            }
            let again = read_hidden("Again")?;
            if first != again {
                println!("  Those do not match.");
                continue;
            }
            return Ok(first);
        }
    }

    fn read_hidden(question: &str) -> Result<String, Box<dyn std::error::Error>> {
        print!("{question}: ");
        std::io::stdout().flush()?;
        if !Command::new("stty").arg("-echo").status()?.success() {
            return Err("a terminal with hidden input is required for credentials".into());
        }
        let mut line = String::new();
        let read = std::io::stdin().lock().read_line(&mut line);
        let _ = Command::new("stty").arg("echo").status();
        println!();
        if read? == 0 {
            return Err("terminal input closed".into());
        }
        Ok(line.trim_end_matches(['\r', '\n']).to_string())
    }

    // ---- the fabric --------------------------------------------------------------

    /// Generate this node's identity and decide which fabric it belongs to.
    ///
    /// The default is a new fabric. There is no guest switch, because a switch can be left
    /// in the wrong position and the person that harms is the one least likely to check
    /// it. Isolation is what happens when somebody presses Enter.
    fn decide_fabric() -> Result<(IdentityFiles, String, bool), Box<dyn std::error::Error>> {
        println!("\nFabric\n------\n");
        println!(
            "By default this machine starts its own fabric and talks to no other ArchieOS\n\
             install, including any on this network. To join one instead you need the owner\n\
             key from a machine already in it.\n"
        );

        std::fs::create_dir_all(STAGING)?;
        std::fs::set_permissions(STAGING, std::fs::Permissions::from_mode(0o700))?;
        let node_key_path = format!("{STAGING}/node.key");
        let owner_key_path = format!("{STAGING}/owner.key");
        let _ = std::fs::remove_file(&node_key_path);
        let _ = std::fs::remove_file(&owner_key_path);

        let node = NodeIdentity::load_or_create(&node_key_path)?;

        let choice = if yes_no("Join an existing fabric", false)? {
            loop {
                let hex = read_hidden("Owner key (hex)")?;
                match fingerprint_of_owner_key(&hex) {
                    Ok(fingerprint) => {
                        println!("  That key belongs to fabric {fingerprint}.");
                        if yes_no("Join that fabric", true)? {
                            break FabricChoice::JoinWithOwnerKey {
                                owner_private_key_hex: hex,
                            };
                        }
                    }
                    Err(problem) => println!("  {problem}"),
                }
            }
        } else {
            FabricChoice::StartNew
        };

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as i64;
        let enrolment = plan_enrolment(&node, &choice, now_ms)?;

        // The owner key is staged only when this install made the fabric. A machine
        // joining an existing one is never given a copy — `plan_enrolment` refuses to
        // hand one back, so there is nothing here to write even by mistake.
        let owner_key_source = match &enrolment.owner {
            Some(owner) => {
                owner.save(&owner_key_path)?;
                Some(owner_key_path)
            }
            None => None,
        };

        Ok((
            IdentityFiles {
                node_key_source: node_key_path,
                membership_json: serde_json::to_string_pretty(&enrolment.membership)?,
                owner_key_source,
            },
            enrolment.fabric_fingerprint(),
            enrolment.started_new_fabric,
        ))
    }

    // ---- showing and confirming ---------------------------------------------------

    fn show(install: &InstallPlan, fabric_fingerprint: &str, started_new_fabric: bool) {
        println!("\n\nWhat will happen\n================\n");
        println!("{}\n", install.destruction_warning());
        if started_new_fabric {
            println!(
                "This machine will start its own fabric ({fabric_fingerprint}). It will not\n\
                 talk to any other ArchieOS install until you add them to it.\n"
            );
        } else {
            println!(
                "This machine will join fabric {fabric_fingerprint}. It will talk to the\n\
                 other nodes of that fabric and to nothing else.\n"
            );
        }
        for (index, step) in install.steps.iter().enumerate() {
            let mark = if step.destroys_data { "!" } else { " " };
            println!("{mark} {:>2}. {}", index + 1, step.explain);
        }
        println!(
            "\n{} of those {} steps destroy what is on the disk now.",
            install.destructive_steps(),
            install.steps.len()
        );
    }

    /// Confirm by typing the disk path.
    ///
    /// Not "yes". "Yes" is what somebody types without reading, and the sentence they did
    /// not read is the one naming the disk. Typing `/dev/sdb` cannot be done without
    /// having looked at which disk it is.
    fn confirm(install: &InstallPlan) -> Result<bool, std::io::Error> {
        println!("\nType the path of the disk to erase, exactly, or anything else to stop.");
        let typed = prompt(&format!("Erase {}", install.target.path), None)?;
        Ok(typed == install.target.path)
    }

    // ---- doing --------------------------------------------------------------------

    fn execute(install: &InstallPlan, password: &str) -> Fallible {
        let total = install.steps.len();
        for (index, step) in install.steps.iter().enumerate() {
            println!("\n[{}/{total}] {}", index + 1, step.explain);
            let outcome = perform(&step.action, password);
            if outcome.is_err()
                && matches!(&step.action,Action::Run{argv} if argv.first().is_some_and(|a|a=="archie-map-fetch"))
            {
                eprintln!("Map download incomplete. The system remains usable with its offline NASA globe; the partial package can be resumed later.");
                continue;
            }
            outcome.map_err(|error| {
                // Naming the step matters more than the error text. A person looking at a
                // half-installed disk needs to know whether it was partitioned.
                format!(
                    "step {} of {total} ({}) failed: {error}",
                    index + 1,
                    step.explain
                )
            })?;
        }
        Ok(())
    }

    fn perform(action: &Action, password: &str) -> Fallible {
        match action {
            Action::Run { argv } => run_command(argv, None),
            Action::InTarget { argv } => {
                let mut chrooted = vec!["arch-chroot".to_string(), plan::TARGET_ROOT.to_string()];
                chrooted.extend(argv.iter().cloned());
                run_command(&chrooted, None)
            }
            Action::Write {
                path,
                contents,
                mode,
            } => {
                let full = format!("{}{path}", plan::TARGET_ROOT);
                if let Some(parent) = Path::new(&full).parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&full, contents)?;
                std::fs::set_permissions(&full, std::fs::Permissions::from_mode(*mode))?;
                Ok(())
            }
            Action::InstallKey { from, to, mode } => {
                let full = format!("{}{to}", plan::TARGET_ROOT);
                if let Some(parent) = Path::new(&full).parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(from, &full)?;
                // Set after the copy: `fs::copy` takes the source permissions with it, and
                // the source lives in a tmpfs whose mode is not the one that matters.
                std::fs::set_permissions(&full, std::fs::Permissions::from_mode(*mode))?;
                Ok(())
            }
            // The password reaches `chpasswd` through a pipe. Not an argument, which every
            // process on the machine can read out of /proc, and not a temporary file.
            Action::SetPassword { user } => run_command(
                &[
                    "arch-chroot".into(),
                    plan::TARGET_ROOT.into(),
                    "chpasswd".into(),
                ],
                Some(format!("{user}:{password}\n")),
            ),
        }
    }

    fn run_command(argv: &[String], stdin: Option<String>) -> Fallible {
        let mut command = Command::new(&argv[0]);
        command.args(&argv[1..]);
        if stdin.is_some() {
            command.stdin(Stdio::piped());
        }
        let mut child = command.spawn().map_err(|e| format!("{}: {e}", argv[0]))?;
        if let Some(text) = stdin {
            use std::io::Write as _;
            child
                .stdin
                .as_mut()
                .ok_or("no stdin on the child process")?
                .write_all(text.as_bytes())?;
        }
        let status = child.wait()?;
        if !status.success() {
            return Err(format!("{} exited with {status}", argv[0]).into());
        }
        Ok(())
    }
}
