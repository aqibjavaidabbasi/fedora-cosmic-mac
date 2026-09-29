# macOS-style top bar for COSMIC: plan

Written for this machine: Fedora, COSMIC 1.8.0, libcosmic pinned at `d4d71fd`, tuned-ppd,
UPower, NetworkManager. Everything else in this folder (`control-center/`, `patches/`, `rebuild.sh`) stays as is.

## 1. Architecture decision

| Piece | Build it as | Why |
|---|---|---|
| Tray, input badge, battery, Wi-Fi, search, Control Center, assistant orb | **Native libcosmic applets** (one process each, `X-CosmicApplet=true`) | cosmic-panel only hosts applets. Their popovers are `xdg_popup`s anchored to the button, which is the macOS dropdown behavior. |
| Control Center, Wi-Fi, Battery popovers | **Native applets** | They fit in an applet popup: up to about 400×900, with full iced widgets. |
| Right-hand widget drawer (calendar, weather, world clocks, stocks, screen time) | **Separate libcosmic layer-shell app**, *or* eww as a quick prototype | A popup is anchored to its button and sized to its content. A full-height slide-over sheet needs its own `zwlr_layer_shell` surface (anchor top+right+bottom, exclusive zone 0). cosmic-comp supports layer-shell, so eww and AGS also work. They just won't follow the COSMIC theme. |

Do **not** use waybar. It would replace cosmic-panel rather than extend it, and you would lose
panel-native applets such as the status area and the workspaces button.

Two ways to build the applets:

* **Command-line backend** (what `control-center/` does, using `nmcli`/`wpctl`/`busctl`): simple, and
  COSMIC updates never break it, but it has to poll.
* **D-Bus backend** (what `battery/` does, with `zbus` 5 proxies and property-change streams):
  instant updates and no polling. Use this for anything shown in the bar all the time: battery, Wi-Fi, clock.

## 2. Panel layout (right wing, left → right)

Config file: `~/.config/cosmic/com.system76.CosmicPanel.Panel/v1/plugins_wings`.
It is safer to change the order in *Settings → Desktop → Panel → Configure panel applets*. The file is shown here for reference.

```ron
Some(([
    "com.system76.CosmicPanelAppButton",
    "com.system76.CosmicPanelLauncherButton",
    "com.system76.CosmicPanelWorkspacesButton",
], [
    "com.system76.CosmicAppletStatusArea",      // 1. SNI tray (background apps)
    "com.system76.CosmicAppletInputSources",    // 2. [US] badge
    "com.aqib.CosmicMacBattery",                // 3. 54% + glyph  (this repo: battery/)
    "com.aqib.CosmicMacWifi",                   // 4. Wi-Fi waves  (to build: see §4)
    "com.system76.CosmicPanelLauncherButton",   // 5. magnifier → cosmic-launcher
    "com.aqib.CosmicMacControlCenter",          // 6. dual-slider glyph (this repo: control-center/)
    "com.aqib.CosmicMacAssistant",              // 7. gradient orb (to build: icon button that runs your assistant)
    "com.system76.CosmicAppletNotifications",   // 8. clock/date → your patched Notification Center
]))
```

Your current bar also has minimon, clipboard manager, caffeine and status-hub. Keep them to the
left of item 1 or drop them. macOS hides that kind of item behind the tray.

Notes per item:
* **Input badge**: the stock applet already shows the layout code. To get the macOS pill
  look you'd patch its `view()` to wrap the text in `style::card()` with radius 5. Put that in
  `patches/` alongside the others so `rebuild.sh` re-applies it.
* **Search**: `CosmicPanelLauncherButton` is the right target. For a magnifier glyph, set its
  icon in `data/` of cosmic-panel-button (you already patch that crate) to `system-search-symbolic`.
* **Clock format `Tue Sep 29 2:14 AM`**: your `patches/notifications.py` now owns the clock.
  Format there with chrono `"%a %b %-d %-I:%M %p"`, using the `%-` flags so there is no zero padding.
* **Assistant orb**: `icon_button_from_handle` with an `icon::from_svg_bytes` gradient SVG.
  A symbolic icon can't be multicolor, so ship a full-color SVG and don't call `.symbolic(true)`.

## 3. Theming

Put the global look in the COSMIC theme, not in each applet, so every popover
(stock and custom) matches:

* `~/.config/cosmic/com.system76.CosmicTheme.Dark.Builder/v1/corner_radii`: today `radius_s: 12`,
  `radius_m: 22`. Popovers use `radius_m`, so set `radius_m: (16.0, 16.0, 16.0, 16.0)` for the
  macOS 16px corners. Do the same in `…Light.Builder`, then re-apply the theme from Settings. Your `apply-settings.sh` is the
  place to make that permanent.
* `is_frosted: true` is already set. Whether you get real backdrop blur or only translucency
  depends on the running cosmic-comp. Your patched compositor is the one that decides.
* Panel `opacity` is `0.3` today; macOS sits at about 0.2 to 0.35 depending on wallpaper, which your `panel-contrast.py` already adjusts.
* Per-surface styling in Rust: `battery/src/style.rs` has `card()` (16px radius, a
  `rgba(255,255,255,0.10)` hairline in dark and `rgba(0,0,0,0.08)` in light), `badge(active)` (a round
  accent-filled icon chip) and the Low Power yellow `#FFCC00`. Copy that file into each applet.
* **Font**: SF Pro's license only allows use on Apple platforms. **Inter** (`rsms-inter-fonts`
  on Fedora) is the closest free match. Set it as the interface font in Settings → Desktop → Appearance.
  Use Inter Display for the big numbers in widgets.

## 4. Wi-Fi popover (next applet to build)

Copy `battery/` and swap `power.rs` for an `nm.rs` built on the same pattern:

```rust
#[proxy(interface = "org.freedesktop.NetworkManager",
        default_service = "org.freedesktop.NetworkManager",
        default_path = "/org/freedesktop/NetworkManager")]
trait NetworkManager {
    #[zbus(property)] fn wireless_enabled(&self) -> zbus::Result<bool>;
    #[zbus(property)] fn set_wireless_enabled(&self, on: bool) -> zbus::Result<()>;
    fn get_devices(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
    fn activate_connection(&self, conn: &ObjectPath<'_>, dev: &ObjectPath<'_>, specific: &ObjectPath<'_>)
        -> zbus::Result<OwnedObjectPath>;
}
#[proxy(interface = "org.freedesktop.NetworkManager.Device", default_service = "org.freedesktop.NetworkManager")]
trait Device { #[zbus(property)] fn device_type(&self) -> zbus::Result<u32>; }      // 2 = Wi-Fi
#[proxy(interface = "org.freedesktop.NetworkManager.Device.Wireless", default_service = "org.freedesktop.NetworkManager")]
trait Wireless {
    fn get_all_access_points(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
    fn request_scan(&self, opts: HashMap<&str, Value<'_>>) -> zbus::Result<()>;
    #[zbus(property)] fn active_access_point(&self) -> zbus::Result<OwnedObjectPath>;
}
#[proxy(interface = "org.freedesktop.NetworkManager.AccessPoint", default_service = "org.freedesktop.NetworkManager")]
trait AccessPoint {
    #[zbus(property)] fn ssid(&self) -> zbus::Result<Vec<u8>>;
    #[zbus(property)] fn strength(&self) -> zbus::Result<u8>;
    #[zbus(property)] fn flags(&self) -> zbus::Result<u32>;      // 0x1 = privacy
    #[zbus(property)] fn wpa_flags(&self) -> zbus::Result<u32>;
    #[zbus(property)] fn rsn_flags(&self) -> zbus::Result<u32>;
}
// Known networks: org.freedesktop.NetworkManager.Settings.ListConnections() →
// Settings.Connection.GetSettings()["802-11-wireless"]["ssid"].
```

View mapping:
* Header: `row![text::heading("Wi-Fi").width(Fill), toggler(enabled).on_toggle(Msg::Wifi)]`.
* **"Weak Security" banner**: for the active AP, `flags & 1 != 0 && wpa == 0 && rsn == 0` means WEP.
  `rsn == 0 && wpa != 0` means WPA1 only. Mark either as weak, and mark `flags == 0` as open. Render it as a `style::card()` row with
  `dialog-warning-symbolic` tinted yellow.
* **Known Networks**: saved SSIDs ∩ scan results, strongest first. The active one gets `style::badge(true)`
  around `network-wireless-signal-*-symbolic`. Use `network-wireless-encrypted-symbolic` as the lock.
* **Other Networks**: a `menu_button` with `go-next-symbolic`/`go-down-symbolic` that toggles a `bool`,
  then lists the unknown APs.
* Footer: `menu_button("Wi-Fi Settings…")` → `cosmic-settings wireless`. That's the page id your control
  center already uses. `network` also exists.
* Subscribe to `Wireless.receive_active_access_point_changed()`, to `NetworkManager.receive_wireless_enabled_changed()`,
  and to `AccessPointAdded`/`Removed` signals. Call `request_scan` when the popover opens.

## 5. Control Center v2 (upgrade `control-center/`)

The toggles and actions already work. What's left is layout:

* 2×2 connectivity block: a `Column` of `style::card()`s, each holding a `row![badge(on), column![title, caption(ssid|"On")]]` in a `button::custom`.
  AirDrop equivalent: toggle LocalSend's autostart or `systemctl --user start localsend`. Hide it if LocalSend isn't installed.
* Pill sliders: `slider(...)` inside a `card()` with the icon `row![icon, slider]`. The upstream slider's rail
  height is fixed; for a thick macOS rail use `slider::Style` via `theme::iced::Slider::Custom`.
* Media card: MPRIS over zbus (`org.mpris.MediaPlayer2.Player` on the session bus: `Metadata`,
  `PlaybackStatus`, `Previous/PlayPause/Next`). Load album art from `mpris:artUrl` with `widget::image`.
* Focus: already done with the DND config write. Show it as a pill with `weather-clear-night-symbolic`.
* Screen recording: there's no stock COSMIC recorder. `wf-recorder` doesn't work on cosmic-comp
  (it needs wlr-screencopy), so use `gpu-screen-recorder` (portal capture) and track its PID for the
  "Recording" tag. Selection capture: `cosmic-screenshot --interactive`.

## 6. Widget drawer

Its own binary, `cosmic-mac-widgets`: a libcosmic `Application` (not an applet) that opens one
layer surface via `cosmic::iced::platform_specific::shell::wayland::commands::layer_surface::get_layer_surface`
(anchor `TOP|RIGHT|BOTTOM`, width 360, `KeyboardInteractivity::OnDemand`, namespace `cosmic-mac-widgets`).
Your patched notifications applet toggles it over D-Bus (`com.aqib.Widgets.Toggle`) when the clock is clicked.

Data sources, none of which need an API key:
* Calendar: evolution-data-server over D-Bus, or parse `.ics` with the `icalendar` crate.
* Weather: Open-Meteo (`api.open-meteo.com/v1/forecast?...&current=temperature_2m,weather_code&daily=temperature_2m_max,temperature_2m_min`).
* World clocks: `chrono-tz`, with dials drawn on `canvas::Frame` (hands = two `Path::line`s).
* Stocks: there is no free, keyless, ToS-clean quote API. Plan on a user-supplied key (e.g. Finnhub or Alpha Vantage).
* Screen time: there's no system source. Log the focused app from `cosmic-toplevel-info` (the
  `cctk` toplevel protocol) into SQLite and draw bars on a `canvas`.

For a quick visual prototype, eww with the same card CSS works:
```css
.card { border-radius: 16px; background: rgba(30,30,32,0.55); border: 1px solid rgba(255,255,255,0.10); }
```
