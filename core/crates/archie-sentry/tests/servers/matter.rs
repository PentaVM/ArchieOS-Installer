//! A Matter fabric seen through `chip-tool`: a [`CommandRunner`] that parses
//! chip-tool's argv and answers from commissioned nodes with real cluster
//! state (OnOff, LevelControl, Thermostat), printing chip-tool's report
//! format. A node that is not commissioned times out (exit 1), as chip-tool
//! does when CASE cannot be established.
//!
//! What this is not: Matter on the wire. chip-tool owns PASE/CASE and the
//! interaction model; the adapter's path ends at the argv it hands to the
//! runner, and that is the path tested here. The process spawn itself is
//! [`archie_sentry::adapters::process::ProcessRunner`], untouched by this.
use archie_sentry::{
    adapters::process::{CommandOutput, CommandRunner},
    engine::AdapterError,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub on: bool,
    pub level: u8,
    pub heating_setpoint: i16,
}

#[derive(Debug, Default)]
pub struct Fabric {
    /// (node, endpoint) → clusters, for commissioned nodes.
    pub nodes: BTreeMap<(u64, u16), Endpoint>,
    /// Devices on the network waiting to be commissioned: setup PIN → endpoint 1 state.
    pub uncommissioned: BTreeMap<u32, Endpoint>,
    pub calls: Vec<Vec<String>>,
}

#[derive(Clone)]
pub struct ChipTool {
    pub fabric: Arc<Mutex<Fabric>>,
    pub binary: String,
}
impl ChipTool {
    pub fn new(binary: &str) -> Self {
        Self {
            fabric: Arc::default(),
            binary: binary.into(),
        }
    }
    pub fn commissioned(self, node: u64, endpoint: u16, state: Endpoint) -> Self {
        self.fabric
            .lock()
            .unwrap()
            .nodes
            .insert((node, endpoint), state);
        self
    }
    pub fn calls(&self) -> Vec<Vec<String>> {
        self.fabric.lock().unwrap().calls.clone()
    }
}

fn out(status: i32, text: String) -> CommandOutput {
    CommandOutput {
        status,
        stdout: text.into_bytes(),
        stderr: Vec::new(),
    }
}
fn report(endpoint: u16, cluster: u32, attribute: u32, line: String) -> String {
    format!(
        "[1760000000.000] [1:2] [DMG] ReportDataMessage =\n\
         [1760000000.001] [1:2] [TOO] Endpoint: {endpoint} Cluster: 0x0000_{cluster:04X} Attribute 0x0000_{attribute:04X} DataVersion: 1\n\
         [1760000000.002] [1:2] [TOO]   {line}\n"
    )
}
fn timeout() -> CommandOutput {
    out(
        1,
        "[1760000000.000] [1:2] [TOO] Run command failure: CHIP Error 0x00000032: Timeout\n".into(),
    )
}

impl CommandRunner for ChipTool {
    fn run(&self, argv: &[String], _timeout: Duration) -> Result<CommandOutput, AdapterError> {
        let mut fabric = self.fabric.lock().unwrap();
        fabric.calls.push(argv.to_vec());
        if argv.first() != Some(&self.binary) {
            return Err(AdapterError::Protocol);
        }
        let args: Vec<&str> = argv[1..].iter().map(String::as_str).collect();
        if let ["pairing", "onnetwork", node, pin] = args.as_slice() {
            let (Ok(node), Ok(pin)) = (node.parse::<u64>(), pin.parse::<u32>()) else {
                return Ok(out(1, "invalid argument".into()));
            };
            return Ok(match fabric.uncommissioned.remove(&pin) {
                Some(state) => {
                    fabric.nodes.insert((node, 1), state);
                    out(
                        0,
                        "[TOO] Secure Pairing Success\n[CTL] Commissioning complete for node ID\n"
                            .into(),
                    )
                }
                None => timeout(),
            });
        }
        // Cluster commands end in `<node> <endpoint>`.
        if args.len() < 4 {
            return Ok(out(1, "missing arguments".into()));
        }
        let (Ok(node), Ok(endpoint)) = (
            args[args.len() - 2].parse::<u64>(),
            args[args.len() - 1].parse::<u16>(),
        ) else {
            return Ok(out(1, "invalid node or endpoint".into()));
        };
        let Some(ep) = fabric.nodes.get_mut(&(node, endpoint)) else {
            return Ok(timeout());
        };
        let middle = &args[2..args.len() - 2];
        let ok = || {
            out(
                0,
                "[DMG] Received Command Response Status for Endpoint ... Status=0x0\n".into(),
            )
        };
        Ok(match (args[0], args[1], middle) {
            ("onoff", "on", []) => {
                ep.on = true;
                ok()
            }
            ("onoff", "off", []) => {
                ep.on = false;
                ok()
            }
            ("onoff", "toggle", []) => {
                ep.on = !ep.on;
                ok()
            }
            ("onoff", "read", ["on-off"]) => out(
                0,
                report(
                    endpoint,
                    6,
                    0,
                    format!("OnOff: {}", if ep.on { "TRUE" } else { "FALSE" }),
                ),
            ),
            (
                "levelcontrol",
                "move-to-level-with-on-off",
                [level, _transition, _mask, _override],
            ) => match level.parse::<u8>() {
                Ok(level) if level <= 254 => {
                    ep.level = level;
                    ep.on = level > 0;
                    ok()
                }
                _ => out(1, "CHIP Error: constraint error".into()),
            },
            ("levelcontrol", "read", ["current-level"]) => out(
                0,
                report(endpoint, 8, 0, format!("CurrentLevel: {}", ep.level)),
            ),
            ("thermostat", "write", ["occupied-heating-setpoint", value]) => {
                // The thermostat accepts 7.00..=30.00 °C; outside, CONSTRAINT_ERROR.
                match value.parse::<i16>() {
                    Ok(v) if (700..=3000).contains(&v) => {
                        ep.heating_setpoint = v;
                        out(0, "[DMG] WriteResponseMessage Status=0x0\n".into())
                    }
                    _ => out(
                        1,
                        "[DMG] WriteResponseMessage Status=0x87 (CONSTRAINT_ERROR)\n".into(),
                    ),
                }
            }
            ("thermostat", "read", ["occupied-heating-setpoint"]) => out(
                0,
                report(
                    endpoint,
                    0x201,
                    0x12,
                    format!("OccupiedHeatingSetpoint: {}", ep.heating_setpoint),
                ),
            ),
            _ => out(1, "Unknown command\n".into()),
        })
    }
}
