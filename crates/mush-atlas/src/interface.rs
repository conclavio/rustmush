//! S6 (doc 14 §4.6): the receptionist's and the bus-watcher's interfaces —
//! `atlas.describe` and `who.nearby` over `+atlas` and the room's occupants,
//! `travel.next` and `travel.status` over the journeys — every read filtered
//! for the asker exactly as their own command would be.

use mush_core::{
    AbilityCall, Ctx, InterfaceAbility, InterfaceAbilityProvider, InterfaceError, InterfaceReply,
    ObjId, World,
};
use serde_json::{Value, json};
use smol_str::SmolStr;

use crate::module::{AtlasState, is_staff};
use crate::travel::Traveler;

/// The receptionist's provider id.
pub const ATLAS_INTERFACE: &str = "atlas";

/// The bus-watcher's provider id.
pub const TRAVEL_INTERFACE: &str = "travel";

/// The most lines a described place carries into a reply.
pub const MAX_DESCRIBE_LINES: usize = 40;

/// The prose of a `+atlas` view without its text maps: the headings, the
/// rooms, the ways — a line with a dash, a colon or a parenthesis — since an
/// NPC speaks the place, it does not draw it.
#[must_use]
pub fn prose_lines(lines: Vec<String>) -> Vec<String> {
    lines
        .into_iter()
        .map(|l| l.trim_end().to_string())
        .filter(|l| !l.trim().is_empty())
        .filter(|l| l.contains('—') || l.contains(':') || l.contains('(') || is_room_line(l))
        .collect()
}

/// A room line of the view: one glyph, a space, a capitalised name (a room
/// with no one in it has no dash after its name).
fn is_room_line(line: &str) -> bool {
    let t = line.trim_start();
    let mut chars = t.chars();
    matches!(
        (chars.next(), chars.next(), chars.next()),
        (Some(g), Some(' '), Some(n)) if g.is_ascii_alphanumeric() && n.is_uppercase()
    )
}

fn state<'a>(ctx: &'a Ctx<'_>) -> Result<&'a AtlasState, InterfaceError> {
    ctx.state::<AtlasState>().ok_or_else(|| {
        InterfaceError::Refused("The atlas is not composed on this server.".to_string())
    })
}

fn here_of(w: &World, who: ObjId) -> Option<ObjId> {
    w.get(who).and_then(|o| o.location)
}

/// The receptionist: the lot's layout and who is around.
#[derive(Clone, Copy, Debug, Default)]
pub struct AtlasInterface;

impl InterfaceAbilityProvider for AtlasInterface {
    fn abilities(&self) -> Vec<InterfaceAbility> {
        vec![
            InterfaceAbility {
                name: SmolStr::new_static("atlas.describe"),
                description: "Describe a neighborhood the asker knows — its sites, rooms and ways — as +atlas prints it for them; the asker's own when no place is named."
                    .to_string(),
                params_schema: json!({
                    "type": "object",
                    "properties": { "place": { "type": "string", "description": "a neighborhood by name; omit for here" } }
                }),
                command: SmolStr::new_static("+atlas"),
            },
            InterfaceAbility {
                name: SmolStr::new_static("who.nearby"),
                description: "Who is in the asker's room right now, as they would see it themselves."
                    .to_string(),
                params_schema: json!({ "type": "object", "properties": {} }),
                command: SmolStr::new_static("look"),
            },
        ]
    }

    fn invoke(
        &self,
        ctx: &mut Ctx<'_>,
        call: AbilityCall,
        name: &str,
        input: &Value,
    ) -> Result<InterfaceReply, InterfaceError> {
        let asker = call.asker;
        let staff = is_staff(ctx.w, asker);
        match name {
            "atlas.describe" => {
                let st = state(ctx)?;
                let here = here_of(ctx.w, asker);
                let asked = input
                    .get("place")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                    .map(str::to_string);
                let wanted = asked.as_ref().map(|p| p.to_ascii_lowercase());
                let mine = here
                    .and_then(|room| st.records.neighborhood_of_room(room))
                    .map(|nb| nb.region.clone());
                let nb = match &wanted {
                    None => here.and_then(|room| st.records.neighborhood_of_room(room)),
                    Some(needle) => st
                        .records
                        .neighborhoods
                        .values()
                        // A player's asker sees only their region's neighborhoods,
                        // as +atlas allows them; staff see every one.
                        .filter(|nb| staff || mine.as_ref() == Some(&nb.region))
                        .find(|nb| {
                            nb.name.to_ascii_lowercase() == *needle
                                || nb.key.pack.as_str() == needle
                                || nb.name.to_ascii_lowercase().contains(needle.as_str())
                        }),
                };
                let Some(nb) = nb else {
                    return Err(InterfaceError::Refused(match asked {
                        Some(p) => format!("I don't know a place called {p:?}."),
                        None => "You are nowhere the atlas knows.".to_string(),
                    }));
                };
                let lines = prose_lines(crate::view::neighborhood(st, ctx.w, asker, nb, staff));
                let lines: Vec<String> = lines.into_iter().take(MAX_DESCRIBE_LINES).collect();
                Ok(InterfaceReply::lines(&lines))
            }
            "who.nearby" => {
                let Some(room) = here_of(ctx.w, asker) else {
                    return Err(InterfaceError::Refused("You are nowhere.".to_string()));
                };
                let names = crate::view::occupants(ctx.w, room, asker);
                let others: Vec<String> = names
                    .into_iter()
                    .filter(|n| !n.ends_with("(you)"))
                    .collect();
                let room_name = mush_core::output::display_name(ctx.w, room);
                Ok(InterfaceReply::text(if others.is_empty() {
                    format!("No one else is in {room_name} right now.")
                } else {
                    format!("In {room_name}: {}.", others.join(", "))
                }))
            }
            other => Err(InterfaceError::Unknown(other.to_string())),
        }
    }
}

/// The bus-watcher: the carriers' journeys and the asker's own.
#[derive(Clone, Copy, Debug, Default)]
pub struct TravelInterface;

impl InterfaceAbilityProvider for TravelInterface {
    fn abilities(&self) -> Vec<InterfaceAbility> {
        vec![
            InterfaceAbility {
                name: SmolStr::new_static("travel.next"),
                description: "Where the buses and other carriers are and when each is due, as +travel shows."
                    .to_string(),
                params_schema: json!({
                    "type": "object",
                    "properties": { "carrier": { "type": "string", "description": "a carrier by name; omit for all" } }
                }),
                command: SmolStr::new_static("+travel"),
            },
            InterfaceAbility {
                name: SmolStr::new_static("travel.status"),
                description: "The asker's own journey, if they are travelling: where they are and what is next."
                    .to_string(),
                params_schema: json!({ "type": "object", "properties": {} }),
                command: SmolStr::new_static("+travel/status"),
            },
        ]
    }

    fn invoke(
        &self,
        ctx: &mut Ctx<'_>,
        call: AbilityCall,
        name: &str,
        input: &Value,
    ) -> Result<InterfaceReply, InterfaceError> {
        let asker = call.asker;
        match name {
            "travel.next" => {
                let st = state(ctx)?;
                let wanted = input
                    .get("carrier")
                    .and_then(Value::as_str)
                    .map(|c| c.trim().to_ascii_lowercase())
                    .filter(|c| !c.is_empty());
                let mut lines: Vec<String> = Vec::new();
                for j in st.travel.journeys.values() {
                    let Traveler::Carrier(carrier) = j.traveler else {
                        continue;
                    };
                    let carrier_name = mush_core::output::display_name(ctx.w, carrier);
                    if let Some(w) = &wanted
                        && !carrier_name.to_ascii_lowercase().contains(w.as_str())
                    {
                        continue;
                    }
                    lines.push(format!(
                        "The {carrier_name}: {}",
                        crate::travel::position_line(st, ctx.w, j)
                    ));
                    if lines.len() >= 6 {
                        break;
                    }
                }
                if lines.is_empty() {
                    lines.push(match wanted {
                        Some(w) => format!("Nothing called {w:?} is on the road right now."),
                        None => "Nothing is on the road right now.".to_string(),
                    });
                }
                Ok(InterfaceReply::lines(&lines))
            }
            "travel.status" => {
                let st = state(ctx)?;
                let here = here_of(ctx.w, asker);
                let line = if let Some(j) = st.travel.journey_of(asker) {
                    crate::travel::position_line(st, ctx.w, j)
                } else if let Some(j) = here.and_then(|h| st.travel.journey_of(h)) {
                    format!(
                        "You are aboard the {}: {}",
                        here.map_or_else(String::new, |h| mush_core::output::display_name(
                            ctx.w, h
                        )),
                        crate::travel::position_line(st, ctx.w, j)
                    )
                } else {
                    "You are not travelling.".to_string()
                };
                Ok(InterfaceReply::text(line))
            }
            other => Err(InterfaceError::Unknown(other.to_string())),
        }
    }
}
