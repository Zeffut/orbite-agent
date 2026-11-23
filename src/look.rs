//! What an Agent looks like: a shape with two eyes, in a colour, both its
//! Owner's choice. Orbite's own mark is the planet with a ring; an Agent is
//! somebody of its own. A shape is said once, as how far its edge is from
//! its middle in every direction, and drawn from that wherever it shows:
//! in the terminal, on the page, in the apps.

use std::f32::consts::PI;

/// The shapes an Agent can have, the first being the one it has unasked.
pub const SHAPES: [&str; 12] = [
    "pebble", "egg", "square", "clover", "flower", "star", "triangle", "drop", "bean", "heart",
    "hexagon", "blob",
];

/// The colours an Agent can have: its name, how the page writes it, and
/// the nearest of the 256 a terminal has.
pub const COLOURS: [(&str, &str, u8); 11] = [
    ("white", "#f2f2f0", 255),
    ("clay", "#c08552", 173),
    ("red", "#ff4d5a", 203),
    ("orange", "#ff7a1a", 208),
    ("amber", "#f5a623", 214),
    ("green", "#2fc46d", 41),
    ("teal", "#1fbfa3", 43),
    ("blue", "#3b8cff", 75),
    ("violet", "#9b6bff", 141),
    ("pink", "#ff4fa3", 205),
    ("grey", "#8a8a8a", 245),
];

/// How an Agent looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Look {
    pub shape: &'static str,
    pub colour: &'static str,
}

impl Default for Look {
    fn default() -> Self {
        Self { shape: SHAPES[0], colour: COLOURS[0].0 }
    }
}

impl Look {
    /// The look called `shape` and `colour`; whatever is not one of ours
    /// is the one an Agent has unasked.
    pub fn called(shape: Option<&str>, colour: Option<&str>) -> Self {
        let unasked = Self::default();
        Self {
            shape: SHAPES
                .iter()
                .find(|known| Some(**known) == shape)
                .copied()
                .unwrap_or(unasked.shape),
            colour: COLOURS
                .iter()
                .find(|(known, ..)| Some(*known) == colour)
                .map_or(unasked.colour, |(known, ..)| known),
        }
    }

    /// The colour as a page writes it.
    pub fn on_a_page(&self) -> &'static str {
        COLOURS.iter().find(|(name, ..)| *name == self.colour).map_or(COLOURS[0].1, |found| found.1)
    }

    /// The colour as a terminal of 256 colours numbers it.
    pub fn in_a_terminal(&self) -> u8 {
        COLOURS.iter().find(|(name, ..)| *name == self.colour).map_or(COLOURS[0].2, |found| found.2)
    }

    /// True when the point (`x`, `y`) is inside the shape, turned by
    /// `turned` radians: the shape fits the square from -1 to 1, `y` going
    /// down.
    pub fn covers(&self, x: f32, y: f32, turned: f32) -> bool {
        // The heart is said from the notch at its top, not from its middle.
        if self.shape == "heart" {
            let (sin, cos) = (-turned).sin_cos();
            let (x, y) = (x * cos - y * sin, x * sin + y * cos);
            let (from_x, from_y) = (x, -y - 0.52);
            let angle = from_y.atan2(from_x);
            let reach = angle.sin() * angle.cos().abs().sqrt() / (angle.sin() + 1.4)
                - 2.0 * angle.sin()
                + 2.0;
            return from_x.hypot(from_y) <= 0.37 * reach;
        }
        x.hypot(y) <= edge(self.shape, y.atan2(x) - turned)
    }

    /// Where the eyes sit, down from the middle: lower in a shape whose
    /// weight is low.
    pub fn eyes_down(&self) -> f32 {
        match self.shape {
            "triangle" | "drop" => 0.2,
            "star" => 0.08,
            "heart" => -0.02,
            _ => 0.0,
        }
    }

    /// The outline as a page draws it, in a square of 100 with the middle
    /// at 50.
    pub fn outline(&self) -> String {
        const POINTS: usize = 120;
        let mut path = String::new();
        for point in 0..POINTS {
            let angle = point as f32 / POINTS as f32 * 2.0 * PI;
            // How far the edge is that way, found by stepping out.
            let mut reach = 0.0_f32;
            while reach < 1.4 && self.covers(reach * angle.cos(), reach * angle.sin(), 0.0) {
                reach += 0.004;
            }
            let (x, y) = (50.0 + 42.0 * reach * angle.cos(), 50.0 + 42.0 * reach * angle.sin());
            path.push_str(&format!("{}{x:.1} {y:.1}", if point == 0 { "M" } else { "L" }));
        }
        path.push('Z');
        path
    }

    /// The Agent as a picture a page shows: its shape in its colour, and
    /// its two eyes.
    pub fn picture(&self) -> String {
        let down = 42.0 * self.eyes_down();
        format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\"><path d=\"{}\" \
             fill=\"{}\"/><g fill=\"#0d0d0d\"><rect x=\"38\" y=\"{:.1}\" width=\"7\" height=\"15\" \
             rx=\"3.5\"/><rect x=\"55\" y=\"{:.1}\" width=\"7\" height=\"15\" rx=\"3.5\"/></g></svg>",
            self.outline(),
            self.on_a_page(),
            36.0 + down,
            36.0 + down,
        )
    }
}

/// How far the edge of `shape` is from its middle, towards `angle`.
fn edge(shape: &str, angle: f32) -> f32 {
    let (sin, cos) = angle.sin_cos();
    // The corners of a shape with flat sides, softened.
    let sided = |sides: f32, softness: f32| {
        let slice = 2.0 * PI / sides;
        let within = angle.rem_euclid(slice) - slice / 2.0;
        ((slice / 2.0).cos() / within.cos()).powf(softness)
    };
    match shape {
        "egg" => 0.82 * (1.0 + 0.17 * sin) / (cos * cos + (sin / 1.12).powi(2)).sqrt(),
        "square" => 0.84 / (cos.abs().powi(4) + sin.abs().powi(4)).powf(0.25),
        "clover" => 0.76 * (1.0 - 0.18 * (4.0 * angle).cos()),
        "flower" => 0.8 * (1.0 + 0.15 * (6.0 * angle).cos()),
        "star" => 0.74 * (1.0 + 0.26 * (5.0 * angle + PI / 2.0).cos()),
        "triangle" => 0.76 * (1.0 + 0.21 * (3.0 * angle + 1.5 * PI).cos()),
        "drop" => 0.78 + 0.22 * (angle + PI / 2.0).cos().max(0.0).powi(6),
        "bean" => {
            // Wider than tall, with a dent in its top.
            let oval = 0.94 / (cos * cos + (sin / 0.74).powi(2)).sqrt();
            let from_the_top = (angle + PI / 2.0 + PI).rem_euclid(2.0 * PI) - PI;
            oval * (1.0 - 0.2 * (-from_the_top * from_the_top / 0.12).exp())
        }
        "hexagon" => 0.94 * sided(6.0, 1.0),
        "blob" => 0.8 * (1.0 + 0.1 * (3.0 * angle).cos() + 0.07 * (5.0 * angle + 1.0).cos()),
        _ => 0.92,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shape_holds_its_eyes_and_fits_its_square() {
        for shape in SHAPES {
            let look = Look { shape, colour: "white" };
            let down = look.eyes_down();
            // The four corners of the two eyes.
            for x in [-0.29, -0.12, 0.12, 0.29] {
                for y in [-0.33 + down, 0.03 + down] {
                    assert!(look.covers(x, y, 0.0), "an eye of the {shape} sticks out at {x} {y}");
                }
            }
            for corner in [(-1.0, -1.0), (1.0, 1.0), (-1.0, 1.0), (1.0, -1.0)] {
                assert!(!look.covers(corner.0, corner.1, 0.0), "the {shape} fills its square");
            }
            assert!(look.outline().starts_with('M') && look.outline().ends_with('Z'));
        }
    }

    #[test]
    fn a_look_that_is_not_one_of_ours_is_the_one_an_agent_has_unasked() {
        assert_eq!(Look::called(Some("dragon"), Some("gold")), Look::default());
        let chosen = Look::called(Some("star"), Some("teal"));
        assert_eq!((chosen.shape, chosen.colour), ("star", "teal"));
        assert_eq!(chosen.on_a_page(), "#1fbfa3");
    }
}
