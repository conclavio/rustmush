//! The engine-defined terrain vocabulary (doc 14 §5.1): a small closed set of cell
//! classes with stable byte codes. S1 declares the classes and their glyphs; the
//! passability and cost tables over `(class, locomotion, slope)` are S5's (§5.2), and a
//! ruleset adds *definitions* against these names, never new codes.

use serde::{Deserialize, Serialize};

/// A cell's ground class.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum TerrainClass {
    /// Not a surface (outside the map, inside a solid, on another floor).
    #[default]
    Void = 0,
    /// Concrete, asphalt, flagstones.
    Paved = 1,
    /// Bare earth.
    Ground = 2,
    /// Grass, lawn, scrub.
    Grass = 3,
    /// Open water.
    Water = 4,
    /// An indoor floor.
    Floor = 5,
    /// Broken ground and debris.
    Rubble = 6,
    /// A road surface (paved, but a road).
    Road = 7,
    /// Sand.
    Sand = 8,
    /// Bare rock.
    Rock = 9,
    /// A wall cell: solid.
    Wall = 10,
    /// A doorway or gateway through a wall.
    Door = 11,
    /// A raised platform or dock.
    Platform = 12,
}

impl TerrainClass {
    /// Every class, in code order.
    pub const ALL: [TerrainClass; 13] = [
        TerrainClass::Void,
        TerrainClass::Paved,
        TerrainClass::Ground,
        TerrainClass::Grass,
        TerrainClass::Water,
        TerrainClass::Floor,
        TerrainClass::Rubble,
        TerrainClass::Road,
        TerrainClass::Sand,
        TerrainClass::Rock,
        TerrainClass::Wall,
        TerrainClass::Door,
        TerrainClass::Platform,
    ];

    /// The byte code.
    #[must_use]
    pub fn code(self) -> u8 {
        self as u8
    }

    /// From a byte code.
    #[must_use]
    pub fn from_code(code: u8) -> Option<TerrainClass> {
        TerrainClass::ALL.iter().copied().find(|c| c.code() == code)
    }

    /// The snake-case name.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            TerrainClass::Void => "void",
            TerrainClass::Paved => "paved",
            TerrainClass::Ground => "ground",
            TerrainClass::Grass => "grass",
            TerrainClass::Water => "water",
            TerrainClass::Floor => "floor",
            TerrainClass::Rubble => "rubble",
            TerrainClass::Road => "road",
            TerrainClass::Sand => "sand",
            TerrainClass::Rock => "rock",
            TerrainClass::Wall => "wall",
            TerrainClass::Door => "door",
            TerrainClass::Platform => "platform",
        }
    }

    /// Parse a snake-case name.
    #[must_use]
    pub fn parse(name: &str) -> Option<TerrainClass> {
        TerrainClass::ALL
            .iter()
            .copied()
            .find(|c| c.label() == name)
    }

    /// The text-map glyph (doc 14 §6).
    #[must_use]
    pub fn glyph(self) -> char {
        match self {
            TerrainClass::Void => ' ',
            TerrainClass::Paved => '.',
            TerrainClass::Ground => ',',
            TerrainClass::Grass => '"',
            TerrainClass::Water => '~',
            TerrainClass::Floor => '_',
            TerrainClass::Rubble => 'x',
            TerrainClass::Road => '=',
            TerrainClass::Sand => ':',
            TerrainClass::Rock => '^',
            TerrainClass::Wall => '#',
            TerrainClass::Door => '+',
            TerrainClass::Platform => 'D',
        }
    }

    /// Whether the class is solid by construction (a wall). Everything else is a
    /// question for S5's locomotion table.
    #[must_use]
    pub fn is_solid(self) -> bool {
        matches!(self, TerrainClass::Wall)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_and_labels_round_trip() {
        for class in TerrainClass::ALL {
            assert_eq!(TerrainClass::from_code(class.code()), Some(class));
            assert_eq!(TerrainClass::parse(class.label()), Some(class));
            assert_eq!(
                serde_json::from_str::<TerrainClass>(&format!("\"{}\"", class.label())).ok(),
                Some(class)
            );
        }
        assert_eq!(TerrainClass::from_code(200), None);
    }
}
