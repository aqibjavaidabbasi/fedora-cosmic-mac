//! Rebuilds COSMIC's light and dark themes with frosted glass + Mac-like corners,
//! the same way COSMIC Settings does (builder -> build() -> theme).

use cosmic_config::CosmicConfigEntry;
use cosmic_theme::{BlurStrength, CornerRadii, Theme, ThemeBuilder};

fn apply(builder_cfg: cosmic_config::Config, theme_cfg: cosmic_config::Config, label: &str) {
    let mut b = ThemeBuilder::get_entry(&builder_cfg).unwrap_or_else(|(_, b)| b);

    // Liquid-glass-like: blurred see-through panel, applet popups and system UI.
    b.frosted = BlurStrength::High;
    b.frosted_panel = true;
    b.frosted_applets = true;
    b.frosted_system_interface = true;
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
