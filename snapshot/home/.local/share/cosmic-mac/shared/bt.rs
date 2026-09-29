//! Bluetooth state and actions through bluetoothctl (BlueZ).
//! Shared by the cosmic-mac applets (the same file is copied into each crate).
#![allow(dead_code)]

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Device {
    pub mac: String,
    pub name: String,
    pub icon: String, // BlueZ icon name, e.g. "audio-headset"
    pub paired: bool,
    pub connected: bool,
    pub battery: Option<u8>,
}

#[derive(Clone, Debug, Default)]
pub struct State {
    pub available: bool,
    pub powered: bool,
    pub devices: Vec<Device>, // paired first (connected on top), then nearby
}

async fn output(args: &[&str]) -> String {
    tokio::process::Command::new("bluetoothctl")
        .args(args)
        .output()
        .await
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

fn parse_list(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            let rest = l.trim().strip_prefix("Device ")?;
            let (mac, name) = rest.split_once(' ').unwrap_or((rest, rest));
            Some((mac.to_string(), name.to_string()))
        })
        .collect()
}

/// Read adapter and device state. `nearby` also lists unpaired devices BlueZ has seen.
pub async fn gather(nearby: bool) -> State {
    let show = output(&["show"]).await;
    let mut s = State {
        available: show.contains("Controller"),
        powered: show.lines().any(|l| l.trim() == "Powered: yes"),
        devices: Vec::new(),
    };
    if !s.powered {
        return s;
    }
    let paired = parse_list(&output(&["devices", "Paired"]).await);
    let connected: Vec<String> = parse_list(&output(&["devices", "Connected"]).await).into_iter().map(|(m, _)| m).collect();
    for (mac, name) in &paired {
        let info = output(&["info", mac]).await;
        let field = |k: &str| info.lines().find_map(|l| l.trim().strip_prefix(k).map(|v| v.trim().to_string()));
        s.devices.push(Device {
            mac: mac.clone(),
            name: name.clone(),
            icon: field("Icon:").unwrap_or_default(),
            paired: true,
            connected: connected.contains(mac),
            battery: field("Battery Percentage:").and_then(|v| v.split('(').nth(1)?.trim_end_matches(')').parse().ok()),
        });
    }
    if nearby {
        for (mac, name) in parse_list(&output(&["devices"]).await) {
            // Unnamed devices show their address as the name; macOS hides those.
            if paired.iter().any(|(m, _)| *m == mac) || name.replace('-', ":") == mac {
                continue;
            }
            s.devices.push(Device { mac, name, ..Default::default() });
        }
    }
    s.devices.sort_by(|a, b| b.paired.cmp(&a.paired).then(b.connected.cmp(&a.connected)).then(a.name.cmp(&b.name)));
    s
}

fn spawn(args: Vec<String>) {
    std::thread::spawn(move || {
        let _ = std::process::Command::new("bluetoothctl").args(&args).status();
    });
}

pub fn set_powered(on: bool) {
    if on {
        let _ = std::process::Command::new("rfkill").args(["unblock", "bluetooth"]).status();
    }
    spawn(vec!["power".into(), if on { "on" } else { "off" }.into()]);
}

/// Connect or disconnect a device; unpaired devices get paired and trusted first.
pub fn toggle(d: &Device) {
    let mac = d.mac.clone();
    if d.connected {
        spawn(vec!["disconnect".into(), mac]);
    } else if d.paired {
        spawn(vec!["connect".into(), mac]);
    } else {
        std::thread::spawn(move || {
            let run = |a: &[&str]| std::process::Command::new("bluetoothctl").args(a).status();
            let _ = run(&["pair", &mac]);
            let _ = run(&["trust", &mac]);
            let _ = run(&["connect", &mac]);
        });
    }
}

/// Scan for nearby devices for a few seconds in the background.
pub fn scan() {
    spawn(vec!["--timeout".into(), "8".into(), "scan".into(), "on".into()]);
}

/// Symbolic icon for a BlueZ device icon name.
pub fn device_icon(icon: &str) -> &'static str {
    match icon {
        "audio-headset" | "audio-headphones" => "audio-headphones-symbolic",
        "audio-card" | "audio-speakers" => "audio-speakers-symbolic",
        "input-keyboard" => "input-keyboard-symbolic",
        "input-mouse" => "input-mouse-symbolic",
        "input-gaming" => "input-gaming-symbolic",
        "input-tablet" => "input-tablet-symbolic",
        "phone" => "phone-symbolic",
        "computer" => "computer-symbolic",
        _ => "bluetooth-active-symbolic",
    }
}
