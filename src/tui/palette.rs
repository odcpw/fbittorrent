//! Shared 8-bit sRGB roles; all expensive role calculation happens on theme load.
use super::theme::Theme;
use ftui::PackedRgba as Color;
fn channels(color: Color) -> [u8; 3] {
    [color.r(), color.g(), color.b()]
}
fn map(color: Color, f: impl Fn(u8) -> u8) -> Color {
    let c = channels(color).map(f);
    Color::rgb(c[0], c[1], c[2])
}
fn luminance(color: Color) -> f64 {
    let c = channels(color).map(|c| {
        let v = f64::from(c) / 255.;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    });
    c[0] * 0.2126 + c[1] * 0.7152 + c[2] * 0.0722
}
pub(super) fn contrast(a: Color, b: Color) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}
pub(super) fn mix(fg: Color, bg: Color, weight: f64) -> Color {
    let (a, b) = (channels(fg), channels(bg));
    let c = std::array::from_fn::<_, 3, _>(|i| {
        (weight * f64::from(a[i]) + (1.0 - weight) * f64::from(b[i])).round() as u8
    });
    Color::rgb(c[0], c[1], c[2])
}
pub(super) fn dim(fg: Color, bg: Color) -> (Color, usize) {
    // Changing HSV value alone scales all three RGB channels by the same factor.
    let rungs = [
        map(fg, |c| (f64::from(c) / 1.4).round() as u8),
        mix(fg, bg, 0.75),
        mix(fg, bg, 0.85),
        fg,
    ];
    rungs
        .into_iter()
        .enumerate()
        .find(|(_, c)| contrast(*c, bg) >= 4.5)
        .map(|(i, c)| (c, i + 1))
        .unwrap_or((fg, 4))
}
pub(super) fn accent_text(accent: Color, fg: Color, bg: Color) -> bool {
    contrast(accent, bg) < 4.5
        || (channels(accent)
            .into_iter()
            .zip(channels(fg))
            .all(|(a, b)| a.abs_diff(b) < 32)
            && contrast(accent, fg) < 1.3)
}
pub(super) fn low_chroma(theme: &Theme) -> bool {
    channels(theme.red)
        .into_iter()
        .zip(channels(theme.green))
        .all(|(a, b)| a.abs_diff(b) < 32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::BTreeMap,
        path::{Path, PathBuf},
    };
    const ROOT: &str = "contracts/ui/themes";
    fn fixture(name: &str) -> Theme {
        Theme::from_toml(
            &std::fs::read_to_string(Path::new(ROOT).join(format!("{name}.toml"))).unwrap(),
        )
        .unwrap()
    }
    fn expected() -> BTreeMap<String, serde_json::Value> {
        serde_json::from_str(
            &std::fs::read_to_string(Path::new(ROOT).join("expected.json")).unwrap(),
        )
        .unwrap()
    }
    fn hex(c: Color) -> String {
        format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
    }
    fn check(path: &Path, theme: &Theme) {
        let (dim, rung) = super::dim(theme.fg, theme.bg);
        let ratio = contrast(dim, theme.bg);
        let fg = contrast(theme.fg, theme.bg);
        println!(
            "theme={} rung={rung} dim={} ratio={ratio:.6} fg_ratio={fg:.6}",
            path.display(),
            hex(dim)
        );
        assert!(
            ratio >= 4.5 && fg >= 4.5,
            "{}: dim={ratio} foreground={fg}",
            path.display()
        );
    }
    #[test]
    fn contrast_every_installed_theme() {
        for name in expected().keys() {
            let t = fixture(name);
            check(&Path::new(ROOT).join(format!("{name}.toml")), &t);
        }
        let installed = Path::new("/usr/share/omarchy/themes");
        if let Ok(entries) = std::fs::read_dir(installed) {
            let mut paths: Vec<_> = entries
                .map(|e| e.unwrap().path().join("colors.toml"))
                .filter(|p| p.is_file())
                .collect();
            paths.sort();
            for path in paths {
                let t = Theme::from_toml(&std::fs::read_to_string(&path).unwrap()).unwrap();
                check(&path, &t);
            }
        } else {
            println!("installed theme sweep skipped: /usr/share/omarchy/themes is absent");
        }
        let owner = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/missing"))
            .join(".local/state/omarchy/current/theme/colors.toml");
        if owner.is_file() {
            check(
                &owner,
                &Theme::from_toml(&std::fs::read_to_string(&owner).unwrap()).unwrap(),
            );
        } else {
            println!("live owner theme absent; the owner fixture was still checked");
        }
        let bad = fixture("_bad");
        assert!(
            contrast(bad.fg, bad.bg) < 4.5 && contrast(bad.dim, bad.bg) < 4.5,
            "negative theme must fail the same gate"
        );
    }
    #[test]
    fn dim_matches_fixture() {
        for (name, row) in expected() {
            let t = fixture(&name);
            let expected = super::super::theme::parse_hex(row["dim"].as_str().unwrap()).unwrap();
            let (actual, rung) = dim(t.fg, t.bg);
            println!(
                "theme={name} rung={rung} dim={} expect={} ratio={:.6}",
                hex(actual),
                hex(expected),
                contrast(actual, t.bg)
            );
            assert!(
                channels(actual)
                    .into_iter()
                    .zip(channels(expected))
                    .all(|(a, b)| a.abs_diff(b) <= 1),
                "{name}"
            );
            assert_eq!(rung, row["dim_rung"].as_u64().unwrap() as usize);
        }
    }
    #[test]
    fn low_chroma_detection() {
        for (name, want) in [("owner", true), ("tokyo-night", false)] {
            let t = fixture(name);
            println!("theme={name} low_chroma={}", low_chroma(&t));
            assert_eq!(low_chroma(&t), want);
        }
    }
    #[test]
    fn accent_fallback_table() {
        for (name, want) in [
            ("catppuccin-latte", true),
            ("miasma", true),
            ("rose-pine", true),
            ("kanagawa", true),
            ("owner", false),
            ("tokyo-night", false),
        ] {
            let t = fixture(name);
            let got = accent_text(t.accent, t.fg, t.bg);
            println!(
                "theme={name} accent_ratio={:.6} accent_fallback={got}",
                contrast(t.accent, t.bg)
            );
            assert_eq!(got, want, "{name}");
        }
    }
}
