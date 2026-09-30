//! Rebuilds COSMIC's light and dark themes with frosted glass, Mac-like corners and
//! macOS's neutral window colours,
//! the same way COSMIC Settings does (builder -> build() -> theme).

use cosmic_config::CosmicConfigEntry;
use cosmic_theme::{BlurStrength, CornerRadii, Theme, ThemeBuilder, palette::Srgba};

fn apply(builder_cfg: cosmic_config::Config, theme_cfg: cosmic_config::Config, label: &str) {
    let mut b = ThemeBuilder::get_entry(&builder_cfg).unwrap_or_else(|(_, b)| b);

    // Liquid-glass-like: blurred see-through panel, applet popups and system UI.
    b.frosted = BlurStrength::High;
    b.frosted_panel = true;
    b.frosted_applets = false; // menus stay solid and readable (COSMIC blur is too faint behind text)
    b.frosted_system_interface = false; // notifications, launcher, OSD: solid
    b.frosted_windows = false; // app windows stay solid, like macOS
    b.corner_radii = CornerRadii {
        radius_0: [0.0; 4],
        radius_xs: [6.0; 4],
        radius_s: [12.0; 4],
        radius_m: [22.0; 4],
        radius_l: [32.0; 4],
        radius_xl: [160.0; 4],
    };
    b.active_hint = 0;
    // macOS window colours: neutral greys, no blue tint (#F5F5F7 light, #1E1E20 dark).
    b.neutral_tint = None;
    b.bg_color = Some(if label == "Dark" {
        Srgba::new(0.118, 0.118, 0.125, 1.0)
    } else {
        Srgba::new(0.961, 0.961, 0.969, 1.0)
    });
    // cards: white on light grey, raised grey on dark, like System Settings
    b.primary_container_bg = Some(if label == "Dark" {
        Srgba::new(0.173, 0.173, 0.180, 1.0)
    } else {
        Srgba::new(1.0, 1.0, 1.0, 1.0)
    });
    b.secondary_container_bg = None;

    if let Err(e) = b.write_entry(&builder_cfg) {
        eprintln!("{label}: could not save theme builder: {e:?}");
    }
    let theme: Theme = b.build();
    match theme.write_entry(&theme_cfg) {
        Ok(()) => println!("{label} theme rebuilt with frosted glass."),
        Err(e) => eprintln!("{label}: could not save theme: {e:?}"),
    }
}

fn main() {
    apply(
        ThemeBuilder::light_config().expect("light builder config"),
        Theme::light_config().expect("light theme config"),
        "Light",
    );
    apply(
        ThemeBuilder::dark_config().expect("dark builder config"),
        Theme::dark_config().expect("dark theme config"),
        "Dark",
    );
}
