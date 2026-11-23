//! WoW FPV's own presets, written into the player's libfpv preset folder before libfpv reads it,
//! beside libfpv's shipped ones (`assets/presets/README.md`: libfpv restores its own, `buildIn`,
//! on first launch and after an update; others stay). The player picks them in F8 > Drone; the
//! default drone stays libfpv's.
//!
//! - `libfpv-wow-long-range-vtx`: the Freestyler's TBS Unify Pro32 HV at nine times its 400 mW,
//!   so the analog picture reaches three times as far (free-space loss grows with distance², so
//!   ×9 power is ×3 distance, +9.5 dB of link budget). The user asked for it: Stormwind's walls
//!   broke the picture up a street or two away.
//! - `libfpv-wow-freestyler-5-long-range`: libfpv's Freestyler 5" (`freestyler-5.json` at
//!   `ed4739d`) with that VTX.

use std::path::Path;

use bevy::prelude::*;

/// Each preset: its path under the preset folder and its contents.
const FILES: [(&str, &str); 2] = [
    (
        "vtx/libfpv-wow-long-range-vtx.json",
        include_str!("../presets/vtx/libfpv-wow-long-range-vtx.json"),
    ),
    (
        "quad/libfpv-wow-freestyler-5-long-range.json",
        include_str!("../presets/quad/libfpv-wow-freestyler-5-long-range.json"),
    ),
];

/// Writes each preset that is missing or differs into `<data_dir>/presets`.
pub fn install(data_dir: &Path) {
    let root = data_dir.join("presets");
    for (rel, text) in FILES {
        let path = root.join(rel);
        if std::fs::read_to_string(&path).is_ok_and(|t| t == text) {
            continue;
        }
        let written = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&path, text));
        match written {
            Ok(()) => info!("libfpv: installed the preset {}", path.display()),
            Err(e) => warn!("libfpv: cannot write the preset {}: {e}", path.display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_presets_parse_and_name_each_other() {
        let field = |text: &str, key: &str| {
            let at = text.find(&format!("\"{key}\": \"")).expect(key) + key.len() + 5;
            text[at..at + text[at..].find('"').unwrap()].to_owned()
        };
        let vtx = FILES[0].1;
        let quad = FILES[1].1;
        assert_eq!(
            field(quad, "vtx"),
            field(vtx, "name"),
            "the quad names the VTX"
        );
        assert!(vtx.contains("\"powerMw\": 3600"), "nine times 400 mW");
        for (rel, text) in FILES {
            assert!(
                text.contains("\"buildIn\": false"),
                "{rel}: libfpv must not restore it"
            );
            assert!(field(text, "fileName").starts_with("libfpv-wow"), "{rel}");
            assert!(
                rel.ends_with(&format!("{}.json", field(text, "fileName"))),
                "{rel}"
            );
        }
    }

    #[test]
    fn install_writes_once_and_repairs_a_changed_file() {
        let dir = std::env::temp_dir().join(format!("libfpv-presets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        install(&dir);
        let vtx = dir.join("presets").join(FILES[0].0);
        assert_eq!(std::fs::read_to_string(&vtx).unwrap(), FILES[0].1);
        std::fs::write(&vtx, "edited").unwrap();
        install(&dir);
        assert_eq!(std::fs::read_to_string(&vtx).unwrap(), FILES[0].1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
