use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::process::Command;
use std::time::Duration;

use serde_json::Value;

use super::PortalTargetKind;

const APPIUM_ADDRESS: SocketAddr =
    SocketAddr::V4(std::net::SocketAddrV4::new(Ipv4Addr::LOCALHOST, 4723));

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DeviceKind {
    Emulator,
    Simulator,
    Attached,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceAvailability {
    Available,
    Offline,
    Unauthorized,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredDevice {
    platform: PortalTargetKind,
    kind: DeviceKind,
    id: String,
    name: String,
    availability: DeviceAvailability,
}

impl DiscoveredDevice {
    pub const fn platform(&self) -> PortalTargetKind {
        self.platform
    }

    pub const fn kind(&self) -> DeviceKind {
        self.kind
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub const fn availability(&self) -> &DeviceAvailability {
        &self.availability
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolProbe {
    Available,
    Unavailable { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceDiscoveryReport {
    pub appium: ToolProbe,
    pub android: ToolProbe,
    pub apple: ToolProbe,
    pub devices: Vec<DiscoveredDevice>,
}

pub struct DeviceDiscovery;

impl DeviceDiscovery {
    pub fn discover() -> DeviceDiscoveryReport {
        let appium = probe_appium();
        let (android, mut devices) = command_output("adb", &["devices", "-l"])
            .and_then(|output| parse_adb_devices(&output))
            .map(|devices| (ToolProbe::Available, devices))
            .unwrap_or_else(|reason| (ToolProbe::Unavailable { reason }, Vec::new()));

        let (apple, mut apple_devices) = if cfg!(target_os = "macos") {
            discover_apple_devices()
        } else {
            (
                ToolProbe::Unavailable {
                    reason: "Apple device discovery requires macOS and Xcode".to_owned(),
                },
                Vec::new(),
            )
        };
        devices.append(&mut apple_devices);
        devices.sort_by(|left, right| {
            (left.platform, left.kind, &left.name, &left.id).cmp(&(
                right.platform,
                right.kind,
                &right.name,
                &right.id,
            ))
        });

        DeviceDiscoveryReport {
            appium,
            android,
            apple,
            devices,
        }
    }
}

fn discover_apple_devices() -> (ToolProbe, Vec<DiscoveredDevice>) {
    let simulators = command_output(
        "xcrun",
        &["simctl", "list", "devices", "available", "--json"],
    )
    .and_then(|output| parse_simctl_devices(&output));
    let attached = command_output("xcrun", &["xctrace", "list", "devices"])
        .map(|output| parse_xctrace_devices(&output));

    match (simulators, attached) {
        (Ok(mut simulators), Ok(mut attached)) => {
            simulators.append(&mut attached);
            (ToolProbe::Available, simulators)
        }
        (Ok(devices), Err(_)) | (Err(_), Ok(devices)) => (ToolProbe::Available, devices),
        (Err(simulator_error), Err(attached_error)) => (
            ToolProbe::Unavailable {
                reason: format!(
                    "Xcode device discovery failed: {simulator_error}; {attached_error}"
                ),
            },
            Vec::new(),
        ),
    }
}

fn probe_appium() -> ToolProbe {
    match TcpStream::connect_timeout(&APPIUM_ADDRESS, Duration::from_millis(250)) {
        Ok(_) => ToolProbe::Available,
        Err(error) => ToolProbe::Unavailable {
            reason: format!("Appium is not listening on {APPIUM_ADDRESS}: {error}"),
        },
    }
}

fn command_output(program: &str, arguments: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(arguments)
        .output()
        .map_err(|error| format!("{program} could not be started: {error}"))?;
    if !output.status.success() {
        let detail =
            crate::security::redact_secrets(String::from_utf8_lossy(&output.stderr).trim())
                .into_owned();
        return Err(if detail.is_empty() {
            format!("{program} exited with {}", output.status)
        } else {
            format!("{program} failed: {detail}")
        });
    }
    String::from_utf8(output.stdout)
        .map_err(|error| format!("{program} output is not UTF-8: {error}"))
}

fn parse_adb_devices(output: &str) -> Result<Vec<DiscoveredDevice>, String> {
    if !output
        .lines()
        .any(|line| line.starts_with("List of devices"))
    {
        return Err("ADB returned an unrecognized device list".to_owned());
    }
    Ok(output
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let id = fields.next()?;
            let state = fields.next()?;
            let details = fields.collect::<Vec<_>>();
            let name = details
                .iter()
                .find_map(|field| field.strip_prefix("model:"))
                .unwrap_or(id)
                .replace('_', " ");
            let availability = match state {
                "device" => DeviceAvailability::Available,
                "unauthorized" => DeviceAvailability::Unauthorized,
                _ => DeviceAvailability::Offline,
            };
            Some(DiscoveredDevice {
                platform: PortalTargetKind::Android,
                kind: if id.starts_with("emulator-") {
                    DeviceKind::Emulator
                } else {
                    DeviceKind::Attached
                },
                id: id.to_owned(),
                name,
                availability,
            })
        })
        .collect())
}

fn parse_simctl_devices(output: &str) -> Result<Vec<DiscoveredDevice>, String> {
    let payload: Value = serde_json::from_str(output)
        .map_err(|error| format!("simctl returned invalid JSON: {error}"))?;
    let runtimes = payload
        .get("devices")
        .and_then(Value::as_object)
        .ok_or_else(|| "simctl output has no devices object".to_owned())?;
    let mut devices = Vec::new();
    for runtime_devices in runtimes.values().filter_map(Value::as_array) {
        for device in runtime_devices {
            if device.get("isAvailable").and_then(Value::as_bool) == Some(false) {
                continue;
            }
            let Some(id) = device.get("udid").and_then(Value::as_str) else {
                continue;
            };
            let name = device.get("name").and_then(Value::as_str).unwrap_or(id);
            devices.push(DiscoveredDevice {
                platform: PortalTargetKind::Ios,
                kind: DeviceKind::Simulator,
                id: id.to_owned(),
                name: name.to_owned(),
                availability: DeviceAvailability::Available,
            });
        }
    }
    Ok(devices)
}

fn parse_xctrace_devices(output: &str) -> Vec<DiscoveredDevice> {
    output
        .lines()
        .skip_while(|line| line.trim() != "== Devices ==")
        .skip(1)
        .take_while(|line| line.trim() != "== Simulators ==")
        .filter_map(parse_xctrace_device)
        .collect()
}

fn parse_xctrace_device(line: &str) -> Option<DiscoveredDevice> {
    let line = line.trim();
    let id_start = line.rfind(" (")? + 2;
    let id = line.get(id_start..line.len().checked_sub(1)?)?;
    if !id.contains('-') {
        return None;
    }
    let name_end = line[..id_start - 2].rfind(" (").unwrap_or(id_start - 2);
    let name = line[..name_end].trim();
    if name.is_empty() {
        return None;
    }
    Some(DiscoveredDevice {
        platform: PortalTargetKind::Ios,
        kind: DeviceKind::Attached,
        id: id.to_owned(),
        name: name.to_owned(),
        availability: DeviceAvailability::Available,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_android_emulators_devices_and_authorization() {
        let devices = parse_adb_devices(
            "List of devices attached\nemulator-5554 device product:sdk model:Pixel_8 device:emu\nR58M offline model:Galaxy_S23\nABC unauthorized usb:1-2\n",
        )
        .unwrap();

        assert_eq!(devices.len(), 3);
        assert_eq!(devices[0].kind(), DeviceKind::Emulator);
        assert_eq!(devices[0].name(), "Pixel 8");
        assert_eq!(devices[1].availability(), &DeviceAvailability::Offline);
        assert_eq!(devices[2].availability(), &DeviceAvailability::Unauthorized);
    }

    #[test]
    fn parses_available_apple_simulators() {
        let devices = parse_simctl_devices(
            r#"{"devices":{"com.apple.CoreSimulator.SimRuntime.iOS-18-0":[{"name":"iPhone 16","udid":"SIM-1","isAvailable":true},{"name":"Old","udid":"SIM-2","isAvailable":false}]}}"#,
        )
        .unwrap();

        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].kind(), DeviceKind::Simulator);
        assert_eq!(devices[0].id(), "SIM-1");
    }

    #[test]
    fn parses_attached_apple_devices_without_simulators() {
        let devices = parse_xctrace_devices(
            "== Devices ==\nJoao's iPhone (18.0) (00008110-001234)\nMacBook (15.0) (ABC)\n== Simulators ==\niPhone 16 Simulator (18.0) (SIM-1)\n",
        );

        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].name(), "Joao's iPhone");
        assert_eq!(devices[0].kind(), DeviceKind::Attached);
    }
}
