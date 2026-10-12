//! Matter (chip-tool) and Bluetooth (bluetoothctl) adapters. All process
//! interaction goes through a FakeRunner that records argv and replays
//! fixtures; no real binary is ever spawned.
use archie_sentry::{
    adapters::{
        bluetooth::{
            parse_address, parse_gatt_value, parse_hex_payload, parse_info, parse_scan, valid_uuid,
            BleSwitch, BleTarget, Bluetooth,
        },
        matter::{
            level_for_percent, pairing_succeeded, parse_bool_attribute, parse_int_attribute,
            percent_for_level, Matter, MatterKind, MatterTarget,
        },
        process::{CommandOutput, CommandRunner},
    },
    contracts::*,
    engine::*,
    registry::*,
};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};

// ---------------------------------------------------------------------------
// FakeRunner: records every argv, replays scripted replies in order.
// A call beyond the script gets status 1, which the adapters treat as failure -
// so an unexpected extra command can never silently pass a test.
// ---------------------------------------------------------------------------
struct FakeRunner {
    calls: Mutex<Vec<Vec<String>>>,
    replies: Mutex<VecDeque<Result<CommandOutput, AdapterError>>>,
}
impl FakeRunner {
    fn with(replies: Vec<Result<CommandOutput, AdapterError>>) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(vec![]),
            replies: Mutex::new(replies.into()),
        })
    }
    fn ok(stdout: &str) -> Result<CommandOutput, AdapterError> {
        Ok(CommandOutput {
            status: 0,
            stdout: stdout.as_bytes().to_vec(),
            stderr: vec![],
        })
    }
    fn fail(status: i32) -> Result<CommandOutput, AdapterError> {
        Ok(CommandOutput {
            status,
            stdout: vec![],
            stderr: b"tool failed".to_vec(),
        })
    }
    fn calls(&self) -> Vec<Vec<String>> {
        self.calls.lock().unwrap().clone()
    }
}
impl CommandRunner for FakeRunner {
    fn run(&self, argv: &[String], _timeout: Duration) -> Result<CommandOutput, AdapterError> {
        self.calls.lock().unwrap().push(argv.to_vec());
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Self::fail(1))
    }
}

fn chip_path() -> &'static str {
    if cfg!(windows) {
        "C:\\chip-tool\\chip-tool.exe"
    } else {
        "/usr/bin/chip-tool"
    }
}
fn bluetoothctl_path() -> &'static str {
    if cfg!(windows) {
        "C:\\bluez\\bluetoothctl.exe"
    } else {
        "/usr/bin/bluetoothctl"
    }
}

// ---------------------------------------------------------------------------
// Fixtures: chip-tool's human-readable report format
// ---------------------------------------------------------------------------
const CHIP_ON_TRUE: &str = "\
[1700000001.100][1234:1234] [TOO] Endpoint: 1 Cluster: 0x0000_0006 Attribute 0x0000_0000 DataVersion: 3712345678\n\
[1700000001.101][1234:1234] [TOO]   OnOff: TRUE\n";
const CHIP_ON_FALSE: &str = "\
[1700000001.200][1234:1234] [TOO] Endpoint: 1 Cluster: 0x0000_0006 Attribute 0x0000_0000 DataVersion: 3712345679\n\
[1700000001.201][1234:1234] [TOO]   OnOff: FALSE\n";
const CHIP_LEVEL_127: &str = "\
[1700000002.100][1234:1234] [TOO] Endpoint: 1 Cluster: 0x0000_0008 Attribute 0x0000_0000 DataVersion: 3712345680\n\
[1700000002.101][1234:1234] [TOO]   CurrentLevel: 127\n";
const CHIP_LEVEL_0: &str = "\
[1700000002.200][1234:1234] [TOO] Endpoint: 1 Cluster: 0x0000_0008 Attribute 0x0000_0000 DataVersion: 3712345681\n\
[1700000002.201][1234:1234] [TOO]   CurrentLevel: 0\n";
const CHIP_SETPOINT_2250: &str = "\
[1700000003.100][1234:1234] [TOO] Endpoint: 1 Cluster: 0x0000_0201 Attribute 0x0000_0012 DataVersion: 3712345682\n\
[1700000003.101][1234:1234] [TOO]   OccupiedHeatingSetpoint: 2250\n";
const CHIP_PAIR_OK: &str = "\
[1700000004.100][1234:1234] [TOO] Commissioning complete for node ID 0x0000000000000001\n";
const CHIP_PAIR_STALE: &str = "\
[1700000004.200][1234:1234] [TOO] Run command failure: ../../examples/chip-tool/commands/pairing/PairingCommand.cpp:120: CHIP Error 0x00000032: Timeout\n";

// ---------------------------------------------------------------------------
// Fixtures: bluetoothctl
// ---------------------------------------------------------------------------
const INFO_PAIRED_CONNECTED: &str = "\
Device AA:BB:CC:DD:EE:FF (public)\n\
\tName: Test Bulb\n\
\tAlias: Test Bulb\n\
\tPaired: yes\n\
\tBonded: yes\n\
\tTrusted: no\n\
\tBlocked: no\n\
\tConnected: yes\n\
\tLegacyPairing: no\n";
const INFO_PAIRED_DISCONNECTED: &str = "\
Device AA:BB:CC:DD:EE:FF (public)\n\
\tName: Test Bulb\n\
\tPaired: yes\n\
\tConnected: no\n";
const INFO_UNPAIRED: &str = "\
Device AA:BB:CC:DD:EE:FF (public)\n\
\tPaired: no\n\
\tConnected: no\n";
const INFO_UNKNOWN_DEVICE: &str = "Device AA:BB:CC:DD:EE:FF not available\n";

const GATT_READ_ON: &str = "\
Attempting to read /org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF/service0008/char0009\n\
[CHG] Attribute /org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF/service0008/char0009 Value:\n\
        01                                                .\n";
const GATT_READ_OFF_INLINE: &str = "\
Attempting to read /org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF/service0008/char0009\n\
[CHG] Attribute /org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF/service0008/char0009 Value: 0x00\n";
const GATT_READ_UNKNOWN_VALUE: &str = "\
[CHG] Attribute /org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF/service0008/char0009 Value:\n\
        7f                                                .\n";
const GATT_READ_PROSE: &str = "Attempting to read /org/bluez/hci0/...\nFailed to read attribute\n";

const SCAN_OUTPUT: &str = "\
Discovery started\n\
[CHG] Controller F8:34:41:AA:BB:CC Discovering: yes\n\
[NEW] Device AA:BB:CC:DD:EE:01 BLE Bulb\n\
[CHG] Device AA:BB:CC:DD:EE:02 RSSI: -61\n\
[NEW] Device AA:BB:CC:DD:EE:02\n\
[NEW] Device aa:bb:cc:dd:ee:01 Duplicate Seen Again\n";

const MAC: &str = "AA:BB:CC:DD:EE:FF";
const READ_UUID: &str = "00002a19-0000-1000-8000-00805f9b34fb";
const WRITE_UUID: &str = "00002a19-0000-1000-8000-00805f9b34fb";

// ---------------------------------------------------------------------------
// Shared builders
// ---------------------------------------------------------------------------
fn matter_resource(kind_capability: CommandKind) -> ResourceDescriptor {
    ResourceDescriptor {
        resource_id: "lamp".into(),
        edge_id: "edge".into(),
        adapter_id: "matter".into(),
        paired: true,
        capabilities: vec![kind_capability],
    }
}
fn matter_target(kind: MatterKind) -> MatterTarget {
    MatterTarget {
        node_id: 1,
        endpoint: 1,
        kind,
    }
}
fn matter_adapter(runner: Arc<FakeRunner>, kind: MatterKind) -> Matter {
    Matter::new(
        "edge",
        "matter",
        chip_path(),
        BTreeMap::from([("lamp".into(), matter_target(kind))]),
        runner,
    )
    .unwrap()
}
fn ble_resource(capabilities: Vec<CommandKind>) -> ResourceDescriptor {
    ResourceDescriptor {
        resource_id: "bulb".into(),
        edge_id: "edge".into(),
        adapter_id: "ble".into(),
        paired: true,
        capabilities,
    }
}
fn ble_target(with_switch: bool) -> BleTarget {
    BleTarget {
        address: MAC.into(),
        switch: with_switch.then(|| BleSwitch {
            read_uuid: READ_UUID.into(),
            write_uuid: WRITE_UUID.into(),
            on: "01".into(),
            off: "00".into(),
        }),
    }
}
fn ble_adapter(runner: Arc<FakeRunner>, with_switch: bool) -> Bluetooth {
    Bluetooth::new(
        "edge",
        "ble",
        bluetoothctl_path(),
        BTreeMap::from([("bulb".into(), ble_target(with_switch))]),
        runner,
    )
    .unwrap()
}

// ---------------------------------------------------------------------------
// Matter: parsers
// ---------------------------------------------------------------------------
#[test]
fn chip_bool_attribute_parsing() {
    assert_eq!(parse_bool_attribute(CHIP_ON_TRUE, "OnOff"), Some(true));
    assert_eq!(parse_bool_attribute(CHIP_ON_FALSE, "OnOff"), Some(false));
    // Not a bool, missing, or a longer attribute name ending the same way.
    assert_eq!(parse_bool_attribute(CHIP_LEVEL_127, "OnOff"), None);
    assert_eq!(parse_bool_attribute("nothing here", "OnOff"), None);
    assert_eq!(parse_bool_attribute("  NotOnOff: TRUE\n", "OnOff"), None);
}

#[test]
fn chip_int_attribute_parsing() {
    assert_eq!(
        parse_int_attribute(CHIP_LEVEL_127, "CurrentLevel"),
        Some(127)
    );
    assert_eq!(parse_int_attribute(CHIP_LEVEL_0, "CurrentLevel"), Some(0));
    assert_eq!(
        parse_int_attribute(CHIP_SETPOINT_2250, "OccupiedHeatingSetpoint"),
        Some(2250)
    );
    // Negative setpoints (a cold-room thermostat) parse signed.
    assert_eq!(
        parse_int_attribute(
            "  OccupiedHeatingSetpoint: -500\n",
            "OccupiedHeatingSetpoint"
        ),
        Some(-500)
    );
    assert_eq!(
        parse_int_attribute("  CurrentLevel: big\n", "CurrentLevel"),
        None
    );
    assert_eq!(parse_int_attribute("no markers", "CurrentLevel"), None);
}

#[test]
fn chip_pairing_marker_is_required() {
    assert!(pairing_succeeded(CHIP_PAIR_OK));
    assert!(pairing_succeeded("[TOO] Secure Pairing Success\n"));
    assert!(!pairing_succeeded(CHIP_PAIR_STALE));
    assert!(!pairing_succeeded(""));
}

#[test]
fn level_percent_round_trip_is_exact() {
    assert_eq!(level_for_percent(0), 0);
    assert_eq!(level_for_percent(50), 127);
    assert_eq!(level_for_percent(100), 254);
    assert_eq!(percent_for_level(0), Some(0));
    assert_eq!(percent_for_level(127), Some(50));
    assert_eq!(percent_for_level(254), Some(100));
    assert_eq!(percent_for_level(255), None, "0xFF is out of range");
    for percent in 0u8..=100 {
        let level = level_for_percent(percent);
        assert_eq!(
            percent_for_level(level),
            Some(percent),
            "percent {percent} -> level {level} must read back exactly"
        );
    }
}

// ---------------------------------------------------------------------------
// Matter: configuration validation
// ---------------------------------------------------------------------------
#[test]
fn matter_configuration_is_validated() {
    let runner = FakeRunner::with(vec![]);
    let one = |t: MatterTarget| BTreeMap::from([("lamp".into(), t)]);
    // Relative tool path.
    assert!(Matter::new(
        "edge",
        "matter",
        "chip-tool",
        one(matter_target(MatterKind::Switch)),
        runner.clone()
    )
    .is_err());
    // Node id 0 and endpoint 0 are never actuable devices.
    assert!(Matter::new(
        "edge",
        "matter",
        chip_path(),
        one(MatterTarget {
            node_id: 0,
            endpoint: 1,
            kind: MatterKind::Switch
        }),
        runner.clone()
    )
    .is_err());
    assert!(Matter::new(
        "edge",
        "matter",
        chip_path(),
        one(MatterTarget {
            node_id: 1,
            endpoint: 0,
            kind: MatterKind::Switch
        }),
        runner.clone()
    )
    .is_err());
    // Empty target set, bad resource id.
    assert!(Matter::new(
        "edge",
        "matter",
        chip_path(),
        BTreeMap::new(),
        runner.clone()
    )
    .is_err());
    assert!(Matter::new(
        "edge",
        "matter",
        chip_path(),
        BTreeMap::from([("bad id!".into(), matter_target(MatterKind::Switch))]),
        runner
    )
    .is_err());
}

// ---------------------------------------------------------------------------
// Matter: commands and read-back
// ---------------------------------------------------------------------------
#[tokio::test]
async fn switch_set_invokes_onoff_with_exact_argv() {
    let runner = FakeRunner::with(vec![FakeRunner::ok("")]);
    let mut adapter = matter_adapter(runner.clone(), MatterKind::Switch);
    let resource = matter_resource(CommandKind::SwitchSet);
    let ack = adapter
        .apply(&resource, &Command::SwitchSet { on: true })
        .await
        .unwrap();
    assert!(ack.accepted);
    assert_eq!(
        runner.calls(),
        vec![vec![
            chip_path().to_string(),
            "onoff".into(),
            "on".into(),
            "1".into(),
            "1".into(),
        ]]
    );

    // The postcondition: on-off reads back as the requested state.
    let runner = FakeRunner::with(vec![FakeRunner::ok(CHIP_ON_TRUE)]);
    let mut adapter = matter_adapter(runner.clone(), MatterKind::Switch);
    let observation = adapter.observe(&resource).await.unwrap();
    assert_eq!(observation.state, ResourceState::Switch { on: true });
    assert_eq!(observation.origin, ObservationOrigin::Measurement);
    assert!(observation.evidence_id.starts_with("matter-"));
    assert!(Command::SwitchSet { on: true }.matches(&observation.state));
    assert!(!Command::SwitchSet { on: false }.matches(&observation.state));
    assert_eq!(
        runner.calls(),
        vec![vec![
            chip_path().to_string(),
            "onoff".into(),
            "read".into(),
            "on-off".into(),
            "1".into(),
            "1".into(),
        ]]
    );
}

#[tokio::test]
async fn light_brightness_maps_onto_current_level() {
    // 50% -> level 127, move-to-level-with-on-off with immediate transition.
    let runner = FakeRunner::with(vec![FakeRunner::ok("")]);
    let mut adapter = matter_adapter(runner.clone(), MatterKind::Light);
    let resource = matter_resource(CommandKind::LightSetBrightness);
    adapter
        .apply(&resource, &Command::LightSetBrightness { percent: 50 })
        .await
        .unwrap();
    assert_eq!(
        runner.calls(),
        vec![vec![
            chip_path().to_string(),
            "levelcontrol".into(),
            "move-to-level-with-on-off".into(),
            "127".into(),
            "0".into(),
            "0".into(),
            "0".into(),
            "1".into(),
            "1".into(),
        ]]
    );

    // Postcondition: on-off TRUE and current-level 127 read back as 50%.
    let runner = FakeRunner::with(vec![
        FakeRunner::ok(CHIP_ON_TRUE),
        FakeRunner::ok(CHIP_LEVEL_127),
    ]);
    let mut adapter = matter_adapter(runner.clone(), MatterKind::Light);
    let observation = adapter.observe(&resource).await.unwrap();
    assert_eq!(
        observation.state,
        ResourceState::Light {
            on: true,
            brightness_percent: 50
        }
    );
    assert!(Command::LightSetBrightness { percent: 50 }.matches(&observation.state));
    assert!(!Command::LightSetBrightness { percent: 51 }.matches(&observation.state));
    assert_eq!(runner.calls().len(), 2);

    // 0% drives level 0, which the with-on-off variant turns off; the device
    // then reads back on: false, brightness: 0.
    let runner = FakeRunner::with(vec![FakeRunner::ok("")]);
    let mut adapter = matter_adapter(runner.clone(), MatterKind::Light);
    adapter
        .apply(&resource, &Command::LightSetBrightness { percent: 0 })
        .await
        .unwrap();
    assert_eq!(runner.calls()[0][3], "0");
    let runner = FakeRunner::with(vec![
        FakeRunner::ok(CHIP_ON_FALSE),
        FakeRunner::ok(CHIP_LEVEL_0),
    ]);
    let mut adapter = matter_adapter(runner.clone(), MatterKind::Light);
    let observation = adapter.observe(&resource).await.unwrap();
    assert!(Command::LightSetBrightness { percent: 0 }.matches(&observation.state));
}

#[tokio::test]
async fn climate_temperature_uses_hundredths_of_a_degree() {
    // 22.5 C = 22500 milli-Celsius = 2250 centi-degrees.
    let runner = FakeRunner::with(vec![FakeRunner::ok("")]);
    let mut adapter = matter_adapter(runner.clone(), MatterKind::Climate);
    let resource = matter_resource(CommandKind::ClimateSetTemperature);
    adapter
        .apply(
            &resource,
            &Command::ClimateSetTemperature {
                milli_celsius: 22500,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        runner.calls(),
        vec![vec![
            chip_path().to_string(),
            "thermostat".into(),
            "write".into(),
            "occupied-heating-setpoint".into(),
            "2250".into(),
            "1".into(),
            "1".into(),
        ]]
    );

    let runner = FakeRunner::with(vec![FakeRunner::ok(CHIP_SETPOINT_2250)]);
    let mut adapter = matter_adapter(runner.clone(), MatterKind::Climate);
    let observation = adapter.observe(&resource).await.unwrap();
    assert_eq!(
        observation.state,
        ResourceState::Climate {
            target_milli_celsius: 22500
        }
    );
    assert!(Command::ClimateSetTemperature {
        milli_celsius: 22500
    }
    .matches(&observation.state));
}

#[tokio::test]
async fn matter_pairing_runs_and_requires_its_marker() {
    let runner = FakeRunner::with(vec![FakeRunner::ok(CHIP_PAIR_OK)]);
    let adapter = matter_adapter(runner.clone(), MatterKind::Switch);
    adapter.pair_code(1, "MT:Y.K90SO527J06.00").unwrap();
    assert_eq!(
        runner.calls(),
        vec![vec![
            chip_path().to_string(),
            "pairing".into(),
            "code".into(),
            "1".into(),
            "MT:Y.K90SO527J06.00".into(),
        ]]
    );

    let runner = FakeRunner::with(vec![FakeRunner::ok(CHIP_PAIR_OK)]);
    let adapter = matter_adapter(runner.clone(), MatterKind::Switch);
    adapter.pair_onnetwork(2, 20202021).unwrap();
    assert_eq!(
        runner.calls()[0],
        vec![
            chip_path().to_string(),
            "pairing".into(),
            "onnetwork".into(),
            "2".into(),
            "20202021".into(),
        ]
    );

    // Exit 0 without the commissioning marker is not success.
    let runner = FakeRunner::with(vec![FakeRunner::ok(CHIP_PAIR_STALE)]);
    let adapter = matter_adapter(runner.clone(), MatterKind::Switch);
    assert_eq!(
        adapter.pair_code(3, "34970112332").unwrap_err(),
        AdapterError::Unavailable
    );
    // A failed tool is Unavailable; an invalid code never spawns a process.
    let runner = FakeRunner::with(vec![FakeRunner::fail(1)]);
    let adapter = matter_adapter(runner.clone(), MatterKind::Switch);
    assert_eq!(
        adapter.pair_onnetwork(3, 20202021).unwrap_err(),
        AdapterError::Unavailable
    );
    assert!(adapter.pair_code(0, "34970112332").is_err());
    assert!(adapter.pair_code(3, "code with spaces").is_err());
    assert!(adapter.pair_onnetwork(3, 100_000_000).is_err());
    assert_eq!(runner.calls().len(), 1, "validation failures never spawn");
}

#[tokio::test]
async fn matter_refuses_unsupported_and_unpaired_without_acting() {
    let runner = FakeRunner::with(vec![]);
    let mut adapter = matter_adapter(runner.clone(), MatterKind::Switch);
    // Locks, color, media and every other kind: no readable postcondition.
    let resource = ResourceDescriptor {
        resource_id: "lamp".into(),
        edge_id: "edge".into(),
        adapter_id: "matter".into(),
        paired: true,
        capabilities: vec![
            CommandKind::SwitchSet,
            CommandKind::RemoteKey,
            CommandKind::WakeHost,
            CommandKind::LightSetBrightness,
        ],
    };
    for command in [
        Command::RemoteKey {
            key: RemoteKeyName::PowerOn,
        },
        Command::WakeHost,
        // A Switch-kind target does not take light commands.
        Command::LightSetBrightness { percent: 50 },
    ] {
        assert_eq!(
            adapter.apply(&resource, &command).await.unwrap_err(),
            AdapterError::Unsupported,
            "{command:?} must be refused"
        );
    }
    // Not declared in the resource's capabilities.
    assert_eq!(
        adapter
            .apply(
                &matter_resource(CommandKind::LightSetBrightness),
                &Command::SwitchSet { on: true }
            )
            .await
            .unwrap_err(),
        AdapterError::Unsupported
    );
    // Unpaired resource.
    let mut unpaired = matter_resource(CommandKind::SwitchSet);
    unpaired.paired = false;
    assert_eq!(
        adapter
            .apply(&unpaired, &Command::SwitchSet { on: true })
            .await
            .unwrap_err(),
        AdapterError::Unsupported
    );
    assert!(runner.calls().is_empty(), "refusals never spawn chip-tool");
}

#[tokio::test]
async fn matter_unparsable_readback_verifies_nothing() {
    // A release whose report format differs: Protocol, never a guessed state.
    let runner = FakeRunner::with(vec![FakeRunner::ok("on-off: true (maybe)\n")]);
    let mut adapter = matter_adapter(runner.clone(), MatterKind::Switch);
    assert_eq!(
        adapter
            .observe(&matter_resource(CommandKind::SwitchSet))
            .await
            .unwrap_err(),
        AdapterError::Protocol
    );
    // A level beyond the Matter range is not evidence.
    let runner = FakeRunner::with(vec![
        FakeRunner::ok(CHIP_ON_TRUE),
        FakeRunner::ok("  CurrentLevel: 300\n"),
    ]);
    let mut adapter = matter_adapter(runner.clone(), MatterKind::Light);
    assert_eq!(
        adapter
            .observe(&matter_resource(CommandKind::LightSetBrightness))
            .await
            .unwrap_err(),
        AdapterError::Protocol
    );
    // A tool failure is Unavailable.
    let runner = FakeRunner::with(vec![FakeRunner::fail(1)]);
    let mut adapter = matter_adapter(runner.clone(), MatterKind::Switch);
    assert_eq!(
        adapter
            .observe(&matter_resource(CommandKind::SwitchSet))
            .await
            .unwrap_err(),
        AdapterError::Unavailable
    );
}

// ---------------------------------------------------------------------------
// Bluetooth: parsers and validation
// ---------------------------------------------------------------------------
#[test]
fn ble_address_uuid_and_payload_validation() {
    assert_eq!(parse_address(MAC), Some(MAC.to_string()));
    // Lowercase input normalizes to BlueZ's canonical uppercase.
    assert_eq!(parse_address("aa:bb:cc:dd:ee:ff"), Some(MAC.to_string()));
    for bad in [
        "AA:BB:CC:DD:EE",       // too short
        "AA:BB:CC:DD:EE:FF:00", // too long
        "AA-BB-CC-DD-EE-FF",    // hyphens
        "GG:BB:CC:DD:EE:FF",    // not hex
        "AA:BB:CC:DD:EE:F",     // single digit octet
        "",
    ] {
        assert_eq!(parse_address(bad), None, "{bad} must be rejected");
    }

    assert!(valid_uuid(READ_UUID));
    assert!(valid_uuid("2a19"));
    assert!(!valid_uuid("2A19")); // short form is lowercase or digits
    assert!(!valid_uuid("00002A19-0000-1000-8000-00805f9b34fb")); // uppercase long form
    assert!(!valid_uuid("00002a190000-1000-8000-00805f9b34fb")); // missing dash

    assert_eq!(parse_hex_payload("01"), Some(vec![0x01]));
    assert_eq!(parse_hex_payload("0A1b"), Some(vec![0x0a, 0x1b]));
    assert_eq!(parse_hex_payload(""), None);
    assert_eq!(parse_hex_payload("0"), None); // odd length
    assert_eq!(parse_hex_payload("zz"), None);
    assert_eq!(parse_hex_payload(&"00".repeat(33)), None); // over 32 bytes
}

#[test]
fn ble_info_parsing_both_directions() {
    let info = parse_info(INFO_PAIRED_CONNECTED);
    assert_eq!(info.paired, Some(true));
    assert_eq!(info.connected, Some(true));
    let info = parse_info(INFO_PAIRED_DISCONNECTED);
    assert_eq!(info.paired, Some(true));
    assert_eq!(info.connected, Some(false));
    let info = parse_info(INFO_UNPAIRED);
    assert_eq!(info.paired, Some(false));
    // An unknown device is all unknowns; nothing is invented.
    let info = parse_info(INFO_UNKNOWN_DEVICE);
    assert_eq!(info.paired, None);
    assert_eq!(info.connected, None);
}

#[test]
fn ble_scan_parsing_dedups_and_bounds() {
    let devices = parse_scan(SCAN_OUTPUT);
    assert_eq!(devices.len(), 2, "duplicate address collapses");
    assert_eq!(
        devices[0],
        ("AA:BB:CC:DD:EE:01".to_string(), "BLE Bulb".to_string())
    );
    assert_eq!(devices[1].0, "AA:BB:CC:DD:EE:02");
    assert_eq!(devices[1].1, "", "a nameless device keeps an empty name");
    assert!(parse_scan("").is_empty());
}

#[test]
fn ble_gatt_value_formats() {
    // Hexdump block with ASCII gutter.
    assert_eq!(parse_gatt_value(GATT_READ_ON), Some(vec![0x01]));
    // Inline 0x-prefixed token on the Value line.
    assert_eq!(parse_gatt_value(GATT_READ_OFF_INLINE), Some(vec![0x00]));
    // Multi-byte hexdump.
    assert_eq!(
        parse_gatt_value("Value:\n        2a 19 00\n"),
        Some(vec![0x2a, 0x19, 0x00])
    );
    // Prose without a marker, or a marker without bytes, is unknown.
    assert_eq!(parse_gatt_value(GATT_READ_PROSE), None);
    assert_eq!(parse_gatt_value("Value:\n"), None);
}

// ---------------------------------------------------------------------------
// Bluetooth: connection management with read-back
// ---------------------------------------------------------------------------
#[tokio::test]
async fn ble_connect_disconnect_and_pair_verify_through_info() {
    // Already connected: no connect command is sent.
    let runner = FakeRunner::with(vec![FakeRunner::ok(INFO_PAIRED_CONNECTED)]);
    let adapter = ble_adapter(runner.clone(), true);
    adapter.connect(MAC).unwrap();
    assert_eq!(runner.calls().len(), 1, "info only, no connect argv");

    // Disconnected: connect, then info must confirm.
    let runner = FakeRunner::with(vec![
        FakeRunner::ok(INFO_PAIRED_DISCONNECTED),
        FakeRunner::ok(""),
        FakeRunner::ok(INFO_PAIRED_CONNECTED),
    ]);
    let adapter = ble_adapter(runner.clone(), true);
    adapter.connect(MAC).unwrap();
    let calls = runner.calls();
    assert_eq!(calls.len(), 3);
    assert_eq!(
        calls[1],
        vec![
            bluetoothctl_path().to_string(),
            "connect".into(),
            MAC.into()
        ]
    );

    // Connect that does not confirm is Unavailable.
    let runner = FakeRunner::with(vec![
        FakeRunner::ok(INFO_PAIRED_DISCONNECTED),
        FakeRunner::ok(""),
        FakeRunner::ok(INFO_PAIRED_DISCONNECTED),
    ]);
    let adapter = ble_adapter(runner.clone(), true);
    assert_eq!(adapter.connect(MAC).unwrap_err(), AdapterError::Unavailable);

    // Disconnect verifies Connected: no.
    let runner = FakeRunner::with(vec![
        FakeRunner::ok(INFO_PAIRED_CONNECTED),
        FakeRunner::ok(""),
        FakeRunner::ok(INFO_PAIRED_DISCONNECTED),
    ]);
    let adapter = ble_adapter(runner.clone(), true);
    adapter.disconnect(MAC).unwrap();
    assert_eq!(
        runner.calls()[1],
        vec![
            bluetoothctl_path().to_string(),
            "disconnect".into(),
            MAC.into()
        ]
    );

    // Pair verifies Paired: yes; an unchanged report is Unavailable.
    let runner = FakeRunner::with(vec![
        FakeRunner::ok(""),
        FakeRunner::ok(INFO_PAIRED_CONNECTED),
    ]);
    let adapter = ble_adapter(runner.clone(), true);
    adapter.pair(MAC).unwrap();
    assert_eq!(
        runner.calls()[0],
        vec![bluetoothctl_path().to_string(), "pair".into(), MAC.into()]
    );
    let runner = FakeRunner::with(vec![FakeRunner::ok(""), FakeRunner::ok(INFO_UNPAIRED)]);
    let adapter = ble_adapter(runner.clone(), true);
    assert_eq!(adapter.pair(MAC).unwrap_err(), AdapterError::Unavailable);

    // An unconfigured address is never touched.
    let runner = FakeRunner::with(vec![]);
    let adapter = ble_adapter(runner.clone(), true);
    assert_eq!(
        adapter.connect("11:22:33:44:55:66").unwrap_err(),
        AdapterError::Unsupported
    );
    assert!(runner.calls().is_empty());
}

#[tokio::test]
async fn ble_scan_is_bounded_and_parsed() {
    let runner = FakeRunner::with(vec![FakeRunner::ok(SCAN_OUTPUT)]);
    let adapter = ble_adapter(runner.clone(), true);
    let devices = adapter.scan(5).unwrap();
    assert_eq!(devices.len(), 2);
    assert_eq!(
        runner.calls(),
        vec![vec![
            bluetoothctl_path().to_string(),
            "--timeout".into(),
            "5".into(),
            "scan".into(),
            "on".into(),
        ]]
    );
    // Out-of-range windows are refused before any process exists.
    assert!(adapter.scan(0).is_err());
    assert!(adapter.scan(31).is_err());
    assert_eq!(runner.calls().len(), 1);
}

// ---------------------------------------------------------------------------
// Bluetooth: SwitchSet through GATT with read-back
// ---------------------------------------------------------------------------
#[tokio::test]
async fn ble_switch_set_writes_the_configured_payload() {
    let runner = FakeRunner::with(vec![
        FakeRunner::ok(INFO_PAIRED_CONNECTED),
        FakeRunner::ok(""), // select-attribute
        FakeRunner::ok(""), // write
    ]);
    let mut adapter = ble_adapter(runner.clone(), true);
    let resource = ble_resource(vec![CommandKind::SwitchSet]);
    let ack = adapter
        .apply(&resource, &Command::SwitchSet { on: true })
        .await
        .unwrap();
    assert!(ack.accepted);
    let calls = runner.calls();
    assert_eq!(calls.len(), 3);
    assert_eq!(
        calls[1],
        vec![
            bluetoothctl_path().to_string(),
            "gatt.select-attribute".into(),
            WRITE_UUID.into(),
        ]
    );
    assert_eq!(
        calls[2],
        vec![
            bluetoothctl_path().to_string(),
            "gatt.write".into(),
            "01".into(),
        ]
    );

    // Off writes the off payload.
    let runner = FakeRunner::with(vec![
        FakeRunner::ok(INFO_PAIRED_CONNECTED),
        FakeRunner::ok(""),
        FakeRunner::ok(""),
    ]);
    let mut adapter = ble_adapter(runner.clone(), true);
    adapter
        .apply(&resource, &Command::SwitchSet { on: false })
        .await
        .unwrap();
    assert_eq!(runner.calls()[2][2], "00");
}

#[tokio::test]
async fn ble_observe_reads_the_configured_characteristic_back() {
    // on payload reads back as Switch { on: true }.
    let runner = FakeRunner::with(vec![
        FakeRunner::ok(INFO_PAIRED_CONNECTED),
        FakeRunner::ok(""), // select-attribute
        FakeRunner::ok(GATT_READ_ON),
    ]);
    let mut adapter = ble_adapter(runner.clone(), true);
    let resource = ble_resource(vec![CommandKind::SwitchSet]);
    let observation = adapter.observe(&resource).await.unwrap();
    assert_eq!(observation.state, ResourceState::Switch { on: true });
    assert_eq!(observation.origin, ObservationOrigin::Measurement);
    assert!(observation.evidence_id.starts_with("ble-"));
    assert!(Command::SwitchSet { on: true }.matches(&observation.state));
    assert_eq!(
        runner.calls()[1],
        vec![
            bluetoothctl_path().to_string(),
            "gatt.select-attribute".into(),
            READ_UUID.into(),
        ]
    );

    // The off payload in the inline 0x format reads back as off.
    let runner = FakeRunner::with(vec![
        FakeRunner::ok(INFO_PAIRED_CONNECTED),
        FakeRunner::ok(""),
        FakeRunner::ok(GATT_READ_OFF_INLINE),
    ]);
    let mut adapter = ble_adapter(runner.clone(), true);
    let observation = adapter.observe(&resource).await.unwrap();
    assert_eq!(observation.state, ResourceState::Switch { on: false });
    assert!(Command::SwitchSet { on: false }.matches(&observation.state));

    // A value that is neither configured state is Protocol, never guessed.
    let runner = FakeRunner::with(vec![
        FakeRunner::ok(INFO_PAIRED_CONNECTED),
        FakeRunner::ok(""),
        FakeRunner::ok(GATT_READ_UNKNOWN_VALUE),
    ]);
    let mut adapter = ble_adapter(runner.clone(), true);
    assert_eq!(
        adapter.observe(&resource).await.unwrap_err(),
        AdapterError::Protocol
    );
    // A reply without a Value marker is Protocol too.
    let runner = FakeRunner::with(vec![
        FakeRunner::ok(INFO_PAIRED_CONNECTED),
        FakeRunner::ok(""),
        FakeRunner::ok(GATT_READ_PROSE),
    ]);
    let mut adapter = ble_adapter(runner.clone(), true);
    assert_eq!(
        adapter.observe(&resource).await.unwrap_err(),
        AdapterError::Protocol
    );

    // A disconnected device is connected first; an unpaired one is not.
    let runner = FakeRunner::with(vec![
        FakeRunner::ok(INFO_PAIRED_DISCONNECTED),
        FakeRunner::ok(""),
        FakeRunner::ok(INFO_PAIRED_CONNECTED),
        FakeRunner::ok(""),
        FakeRunner::ok(GATT_READ_ON),
    ]);
    let mut adapter = ble_adapter(runner.clone(), true);
    adapter.observe(&resource).await.unwrap();
    assert_eq!(runner.calls()[1][1], "connect");
    let runner = FakeRunner::with(vec![FakeRunner::ok(INFO_UNPAIRED)]);
    let mut adapter = ble_adapter(runner.clone(), true);
    assert_eq!(
        adapter.observe(&resource).await.unwrap_err(),
        AdapterError::Unavailable
    );
    assert_eq!(
        runner.calls().len(),
        1,
        "no implicit pairing, no GATT traffic"
    );
}

#[tokio::test]
async fn ble_refuses_unsupported_without_acting() {
    let runner = FakeRunner::with(vec![]);
    // A target without a switch block is never actuable.
    let mut adapter = ble_adapter(runner.clone(), false);
    let resource = ble_resource(vec![CommandKind::SwitchSet]);
    assert_eq!(
        adapter
            .apply(&resource, &Command::SwitchSet { on: true })
            .await
            .unwrap_err(),
        AdapterError::Unsupported
    );
    assert_eq!(
        adapter.observe(&resource).await.unwrap_err(),
        AdapterError::Unsupported
    );

    // Every other command kind has no readable postcondition here.
    let mut adapter = ble_adapter(runner.clone(), true);
    let resource = ble_resource(vec![
        CommandKind::SwitchSet,
        CommandKind::LightSetBrightness,
        CommandKind::RemoteKey,
    ]);
    for command in [
        Command::LightSetBrightness { percent: 50 },
        Command::RemoteKey {
            key: RemoteKeyName::PowerOff,
        },
    ] {
        assert_eq!(
            adapter.apply(&resource, &command).await.unwrap_err(),
            AdapterError::Unsupported,
            "{command:?} must be refused"
        );
    }
    // Undeclared capability and unpaired resource.
    let resource = ble_resource(vec![CommandKind::RemoteKey]);
    assert_eq!(
        adapter
            .apply(&resource, &Command::SwitchSet { on: true })
            .await
            .unwrap_err(),
        AdapterError::Unsupported
    );
    let mut unpaired = ble_resource(vec![CommandKind::SwitchSet]);
    unpaired.paired = false;
    assert_eq!(
        adapter
            .apply(&unpaired, &Command::SwitchSet { on: true })
            .await
            .unwrap_err(),
        AdapterError::Unsupported
    );
    assert!(
        runner.calls().is_empty(),
        "refusals never spawn bluetoothctl"
    );
}

#[test]
fn ble_configuration_is_validated() {
    let runner = FakeRunner::with(vec![]);
    let one = |t: BleTarget| BTreeMap::from([("bulb".into(), t)]);
    // Relative tool path.
    assert!(Bluetooth::new(
        "edge",
        "ble",
        "bluetoothctl",
        one(ble_target(true)),
        runner.clone()
    )
    .is_err());
    // Bad address.
    assert!(Bluetooth::new(
        "edge",
        "ble",
        bluetoothctl_path(),
        one(BleTarget {
            address: "not-a-mac".into(),
            switch: None,
        }),
        runner.clone(),
    )
    .is_err());
    // Bad UUID.
    let mut target = ble_target(true);
    target.switch.as_mut().unwrap().read_uuid = "not-a-uuid".into();
    assert!(Bluetooth::new(
        "edge",
        "ble",
        bluetoothctl_path(),
        one(target),
        runner.clone()
    )
    .is_err());
    // Identical on/off payloads could never be told apart.
    let mut target = ble_target(true);
    target.switch.as_mut().unwrap().off = "01".into();
    assert!(Bluetooth::new(
        "edge",
        "ble",
        bluetoothctl_path(),
        one(target),
        runner.clone()
    )
    .is_err());
    // Empty targets.
    assert!(Bluetooth::new("edge", "ble", bluetoothctl_path(), BTreeMap::new(), runner).is_err());
}
