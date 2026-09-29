//! UPower + power-profiles (tuned-ppd / power-profiles-daemon) over the system bus.
//!
//! One long-lived subscription watches property changes and emits a full
//! [`Snapshot`] every time anything moves, so the UI never polls.

use std::time::Duration;

use cosmic::iced::{Subscription, futures::SinkExt, stream};
use futures::{StreamExt, channel::mpsc::Sender};
use zbus::{Connection, proxy};

#[proxy(
    interface = "org.freedesktop.UPower.Device",
    default_service = "org.freedesktop.UPower",
    default_path = "/org/freedesktop/UPower/devices/DisplayDevice"
)]
trait Device {
    #[zbus(property)]
    fn percentage(&self) -> zbus::Result<f64>;
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn is_present(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn time_to_empty(&self) -> zbus::Result<i64>;
    #[zbus(property)]
    fn time_to_full(&self) -> zbus::Result<i64>;
}

#[proxy(
    interface = "org.freedesktop.UPower",
    default_service = "org.freedesktop.UPower",
    default_path = "/org/freedesktop/UPower"
)]
trait UPower {
    #[zbus(property)]
    fn on_battery(&self) -> zbus::Result<bool>;
}

/// Same API from power-profiles-daemon and Fedora's tuned-ppd.
#[proxy(
    interface = "net.hadess.PowerProfiles",
    default_service = "net.hadess.PowerProfiles",
    default_path = "/net/hadess/PowerProfiles"
)]
trait PowerProfiles {
    #[zbus(property)]
    fn active_profile(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn set_active_profile(&self, profile: &str) -> zbus::Result<()>;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Charge {
    #[default]
    Unknown,
    Charging,
    Discharging,
    Empty,
    Full,
    /// Plugged in but held (e.g. charge limit reached).
    NotCharging,
}

impl From<u32> for Charge {
    fn from(v: u32) -> Self {
        match v {
            1 => Self::Charging,
            2 | 6 => Self::Discharging,
            3 => Self::Empty,
            4 => Self::Full,
            5 => Self::NotCharging,
            _ => Self::Unknown,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub present: bool,
    pub percent: f64,
    pub charge: Charge,
    pub on_battery: bool,
    pub remaining: Option<Duration>,
    /// `None` when no power-profiles service is running.
    pub low_power: Option<bool>,
}

#[derive(Clone, Debug)]
pub enum Event {
    Connected(Connection),
    Update(Snapshot),
    Error(String),
}

pub fn subscription() -> Subscription<Event> {
    Subscription::run_with("cosmic-mac-battery-upower", |_| {
        stream::channel(8, |mut tx: Sender<Event>| async move {
            loop {
                match watch(&mut tx).await {
                    // the UI dropped the receiver: stop for good
                    Ok(()) => return,
                    Err(e) => {
                        let _ = tx.send(Event::Error(e.to_string())).await;
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
            }
        })
    })
}

async fn watch(tx: &mut Sender<Event>) -> zbus::Result<()> {
    let conn = Connection::system().await?;
    let device = DeviceProxy::new(&conn).await?;
    let upower = UPowerProxy::new(&conn).await?;
    let profiles = PowerProfilesProxy::new(&conn).await.ok();

    if tx.send(Event::Connected(conn.clone())).await.is_err() {
        return Ok(());
    }

    let mut percent = device.receive_percentage_changed().await;
    let mut state = device.receive_state_changed().await;
    let mut on_battery = upower.receive_on_battery_changed().await;
    let mut profile = match &profiles {
        Some(p) => Some(p.receive_active_profile_changed().await),
        None => None,
    };

    loop {
        let snap = snapshot(&device, &upower, profiles.as_ref()).await?;
        if tx.send(Event::Update(snap)).await.is_err() {
            return Ok(());
        }

        let profile_changed = async {
            match profile.as_mut() {
                Some(s) => {
                    s.next().await;
                }
                None => futures::future::pending::<()>().await,
            }
        };
        tokio::select! {
            _ = percent.next() => {}
            _ = state.next() => {}
            _ = on_battery.next() => {}
            _ = profile_changed => {}
            // time-remaining estimates drift without a change signal
            _ = tokio::time::sleep(Duration::from_secs(60)) => {}
        }
    }
}

async fn snapshot(
    device: &DeviceProxy<'_>,
    upower: &UPowerProxy<'_>,
    profiles: Option<&PowerProfilesProxy<'_>>,
) -> zbus::Result<Snapshot> {
    let charge = Charge::from(device.state().await?);
    let secs = match charge {
        Charge::Discharging => device.time_to_empty().await.unwrap_or(0),
        Charge::Charging => device.time_to_full().await.unwrap_or(0),
        _ => 0,
    };
    let low_power = match profiles {
        Some(p) => p.active_profile().await.ok().map(|p| p == "power-saver"),
        None => None,
    };
    Ok(Snapshot {
        present: device.is_present().await.unwrap_or(false),
        percent: device.percentage().await?,
        charge,
        on_battery: upower.on_battery().await?,
        remaining: (secs > 0).then(|| Duration::from_secs(secs as u64)),
        low_power,
    })
}

pub async fn set_low_power(conn: Connection, on: bool) -> Result<(), String> {
    let p = PowerProfilesProxy::new(&conn).await.map_err(|e| e.to_string())?;
    p.set_active_profile(if on { "power-saver" } else { "balanced" })
        .await
        .map_err(|e| e.to_string())
}
