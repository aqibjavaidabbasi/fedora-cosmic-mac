//! Wi-Fi state and actions through nmcli (NetworkManager).
//! Shared by the cosmic-mac applets (the same file is copied into each crate).
#![allow(dead_code)]

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Network {
    pub ssid: String,
    pub signal: u8,
    pub security: String,
    pub in_use: bool,
    pub known: bool,
}

impl Network {
    pub fn secured(&self) -> bool {
        !self.security.trim().is_empty()
    }
    /// Open, WEP, or still allowing WPA1 (TKIP): what macOS flags as "Weak Security".
    pub fn weak(&self) -> bool {
        let s = self.security.to_uppercase();
        s.trim().is_empty() || s.contains("WEP") || s.contains("WPA1")
    }
}

#[derive(Clone, Debug, Default)]
pub struct State {
    pub enabled: bool,
    pub networks: Vec<Network>, // connected first, then strongest; one entry per SSID
}

impl State {
    pub fn current(&self) -> Option<&Network> {
        self.networks.iter().find(|n| n.in_use)
    }
    pub fn known_ssids(&self) -> Vec<String> {
        self.networks.iter().filter(|n| n.known).map(|n| n.ssid.clone()).collect()
    }
}

async fn output(args: &[&str]) -> String {
    tokio::process::Command::new("nmcli")
        .args(args)
        .output()
        .await
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

/// Split one `nmcli -t` line on unescaped ':'.
pub fn fields(line: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(n) = chars.next() {
                    out.last_mut().unwrap().push(n);
                }
            }
            ':' => out.push(String::new()),
            _ => out.last_mut().unwrap().push(c),
        }
    }
    out
}

pub async fn saved_ssids() -> Vec<String> {
    let mut ssids = Vec::new();
    for line in output(&["-t", "-f", "NAME,TYPE", "connection", "show"]).await.lines() {
        let f = fields(line);
        if f.get(1).map(String::as_str) == Some("802-11-wireless") {
            let ssid = output(&["-g", "802-11-wireless.ssid", "connection", "show", &f[0]]).await;
            let ssid = ssid.trim();
            ssids.push(if ssid.is_empty() { f[0].clone() } else { ssid.to_string() });
        }
    }
    ssids
}

/// Read Wi-Fi state. `known` = saved SSIDs if already looked up (None looks them up).
pub async fn gather(rescan: bool, known: Option<Vec<String>>) -> State {
    let enabled = output(&["-t", "radio", "wifi"]).await.trim() == "enabled";
    if !enabled {
        return State { enabled, networks: Vec::new() };
    }
    let known = match known {
        Some(k) => k,
        None => saved_ssids().await,
    };
    let list = output(&["-t", "-f", "IN-USE,SSID,SIGNAL,SECURITY", "dev", "wifi", "list", "--rescan", if rescan { "yes" } else { "no" }]).await;
    let mut networks: Vec<Network> = Vec::new();
    for line in list.lines() {
        let f = fields(line);
        if f.len() < 4 || f[1].trim().is_empty() {
            continue;
        }
        let n = Network {
            in_use: f[0] == "*",
            ssid: f[1].clone(),
            signal: f[2].parse().unwrap_or(0),
            security: f[3].clone(),
            known: known.contains(&f[1]),
        };
        match networks.iter_mut().find(|m| m.ssid == n.ssid) {
            Some(m) => {
                m.in_use |= n.in_use;
                m.signal = m.signal.max(n.signal);
            }
            None => networks.push(n),
        }
    }
    networks.sort_by(|a, b| b.in_use.cmp(&a.in_use).then(b.signal.cmp(&a.signal)));
    State { enabled, networks }
}

fn spawn(program: &'static str, args: Vec<String>) {
    std::thread::spawn(move || {
        let _ = std::process::Command::new(program).args(&args).status();
    });
}

pub fn set_enabled(on: bool) {
    spawn("nmcli", vec!["radio".into(), "wifi".into(), if on { "on" } else { "off" }.into()]);
}

/// Join a network. Returns false when a password is needed (open Settings instead).
pub fn connect(n: &Network) -> bool {
    if n.known {
        spawn("nmcli", vec!["connection".into(), "up".into(), "id".into(), n.ssid.clone()]);
    } else if !n.secured() {
        spawn("nmcli", vec!["device".into(), "wifi".into(), "connect".into(), n.ssid.clone()]);
    } else {
        return false;
    }
    true
}

pub fn signal_icon(signal: u8) -> &'static str {
    match signal {
        70.. => "network-wireless-signal-excellent-symbolic",
        50..70 => "network-wireless-signal-good-symbolic",
        30..50 => "network-wireless-signal-ok-symbolic",
        _ => "network-wireless-signal-weak-symbolic",
    }
}
