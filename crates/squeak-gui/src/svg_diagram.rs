//! Interactive SVG mouse diagram helpers (button highlight & hit-testing).

use squeak_core::resolve_builtin_svg;

/// Normalized bounding box inside the `320x520` SVG viewBox.
#[derive(Debug, Clone, PartialEq)]
pub struct ButtonHitRegion {
    /// SVG element ID (e.g. `"btn-left"`).
    pub svg_id: String,
    /// Minimum X coordinate in SVG viewBox units.
    pub min_x: f64,
    /// Minimum Y coordinate in SVG viewBox units.
    pub min_y: f64,
    /// Maximum X coordinate in SVG viewBox units.
    pub max_x: f64,
    /// Maximum Y coordinate in SVG viewBox units.
    pub max_y: f64,
}

impl ButtonHitRegion {
    /// Returns `true` when `(x, y)` in SVG viewBox coordinates falls within this region.
    #[must_use]
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.min_x && x <= self.max_x && y >= self.min_y && y <= self.max_y
    }
}

/// Returns the rendered SVG XML string with optional `selected_svg_id` highlighted.
#[must_use]
pub fn render_svg_with_highlight(svg_name: Option<&str>, selected_svg_id: Option<&str>) -> String {
    let base = resolve_builtin_svg(svg_name);
    let Some(sel) = selected_svg_id else {
        return base.to_owned();
    };
    let needle = format!("id=\"{sel}\"");
    if let Some(pos) = base.find(&needle) {
        let mut out = String::with_capacity(base.len() + 64);
        out.push_str(&base[..pos + needle.len()]);
        out.push_str(" style=\"fill:#3584e4;stroke:#ffffff;stroke-width:3\"");
        out.push_str(&base[pos + needle.len()..]);
        out
    } else {
        base.to_owned()
    }
}

/// Returns clickable button hit regions for `svg_name` (`320x520` viewBox).
#[must_use]
pub fn button_hit_regions(svg_name: Option<&str>) -> Vec<ButtonHitRegion> {
    match svg_name {
        Some("g502-hero.svg") => vec![
            region("btn-left", 84.0, 96.0, 162.0, 316.0),
            region("btn-right", 192.0, 70.0, 268.0, 296.0),
            region("btn-g8", 66.0, 122.0, 88.0, 176.0),
            region("btn-g7", 62.0, 174.0, 84.0, 224.0),
            region("btn-middle", 166.0, 120.0, 190.0, 194.0),
            region("btn-scroll-left", 146.0, 140.0, 163.0, 174.0),
            region("btn-scroll-right", 193.0, 140.0, 210.0, 174.0),
            region("btn-g9", 167.0, 240.0, 190.0, 276.0),
            region("btn-forward", 60.0, 226.0, 75.0, 268.0),
            region("btn-back", 62.0, 268.0, 78.0, 312.0),
            region("btn-dpi-shift", 44.0, 270.0, 60.0, 314.0),
        ],
        _ => vec![
            region("btn-left", 68.0, 42.0, 152.0, 204.0),
            region("btn-right", 168.0, 42.0, 252.0, 204.0),
            region("btn-middle", 148.0, 84.0, 172.0, 152.0),
            region("btn-forward", 54.0, 186.0, 68.0, 232.0),
            region("btn-back", 54.0, 242.0, 68.0, 288.0),
        ],
    }
}

/// Finds the button `svg_id` under viewBox coordinates `(x, y)`.
#[must_use]
pub fn hit_test_button(svg_name: Option<&str>, x: f64, y: f64) -> Option<String> {
    button_hit_regions(svg_name)
        .into_iter()
        .find(|r| r.contains(x, y))
        .map(|r| r.svg_id)
}

fn region(svg_id: &str, min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> ButtonHitRegion {
    ButtonHitRegion {
        svg_id: svg_id.to_owned(),
        min_x,
        min_y,
        max_x,
        max_y,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlights_selected_button_id_in_g502_svg() {
        let rendered = render_svg_with_highlight(Some("g502-hero.svg"), Some("btn-dpi-shift"));
        assert!(
            rendered.contains(
                "id=\"btn-dpi-shift\" style=\"fill:#3584e4;stroke:#ffffff;stroke-width:3\""
            )
        );
    }

    #[test]
    fn hit_tests_g502_buttons_and_generic_fallback() {
        assert_eq!(
            hit_test_button(Some("g502-hero.svg"), 120.0, 180.0).as_deref(),
            Some("btn-left")
        );
        assert_eq!(
            hit_test_button(Some("g502-hero.svg"), 52.0, 290.0).as_deref(),
            Some("btn-dpi-shift")
        );
        assert_eq!(
            hit_test_button(None, 200.0, 100.0).as_deref(),
            Some("btn-right")
        );
        assert_eq!(hit_test_button(None, 10.0, 10.0), None);
    }
}
