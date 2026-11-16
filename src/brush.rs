//! App side of `brush`: the parts that need the window, the app state or
//! app-only modules. The engine part is `dudel_core::brush`, re-exported
//! here so `crate::brush::…` paths keep working.
pub(crate) use dudel_core::brush::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_brushes_are_held_to_wider_spacing_only_along_the_stroke() {
        let mut turning = Vec::new();
        for brush in crate::workspace::default_brushes() {
            for pressure in [0.1, 0.5, 1.0] {
                let sample = crate::stroke::Sample {
                    pressure,
                    ..Default::default()
                };
                let factors = brush.factors(&sample);
                let [asked, used] = brush.dab_spacing(&factors);
                match brush.reach_along_stroke() {
                    // Spaced on its length along the stroke, not its width.
                    // (Above the half-pixel least spacing.)
                    Some(reach) => {
                        let ratio = reach / brush.aspect;
                        assert!(
                            asked <= 0.5 || (used / asked - ratio).abs() < 1e-3,
                            "{} at pressure {pressure}: {asked} {used}",
                            brush.name
                        );
                        turning.push(brush.name.clone());
                    }
                    None => assert_eq!(asked, used, "{} at pressure {pressure}", brush.name),
                }
            }
        }
        turning.sort();
        turning.dedup();
        assert_eq!(
            turning,
            ["Dry gouache", "Flat bristle", "Impasto", "Palette knife"]
        );
        // The knife at 300 px lays a dab every 9 px, not every 2.
        let knife = crate::workspace::default_brushes()
            .into_iter()
            .find(|b| b.name == "Palette knife")
            .map(|b| Brush { size: 300.0, ..b })
            .unwrap();
        let [asked, used] = knife.dab_spacing(&knife.factors(&crate::stroke::Sample {
            pressure: 1.0,
            ..Default::default()
        }));
        assert!(
            (asked - 1.98).abs() < 0.01 && (used - 9.0).abs() < 0.01,
            "{asked} {used}"
        );
    }
}
