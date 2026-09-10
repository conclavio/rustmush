//! S6 (doc 14 §4.6): the meteorologist's interface — `weather.report` and
//! `weather.forecast` over the cell of the asker's own place, the read side of
//! `weather` and `@weather <zone>`. The forecast reads the climate's
//! transition table and rolls nothing: the likeliest next state, whatever
//! the season's row says, never the instruments.

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_core::{
    AbilityCall, Ctx, InterfaceAbility, InterfaceAbilityProvider, InterfaceError, InterfaceReply,
};
use serde_json::{Value, json};
use smol_str::SmolStr;

use crate::climate::ClimateSet;
use crate::{describe_state, resolve_room_weather};

/// The provider id.
pub const WEATHER_INTERFACE: &str = "weather";

/// The meteorologist.
#[derive(Clone, Debug)]
pub struct WeatherInterface {
    /// The loaded climates.
    pub climates: Arc<ClimateSet>,
}

impl InterfaceAbilityProvider for WeatherInterface {
    fn abilities(&self) -> Vec<InterfaceAbility> {
        vec![
            InterfaceAbility {
                name: SmolStr::new_static("weather.report"),
                description: "The weather where the asker stands right now: state, temperature, wind, visibility."
                    .to_string(),
                params_schema: json!({ "type": "object", "properties": {} }),
                command: SmolStr::new_static("weather"),
            },
            InterfaceAbility {
                name: SmolStr::new_static("weather.forecast"),
                description: "What the weather where the asker stands is likeliest to turn to next."
                    .to_string(),
                params_schema: json!({ "type": "object", "properties": {} }),
                command: SmolStr::new_static("weather"),
            },
        ]
    }

    fn invoke(
        &self,
        ctx: &mut Ctx<'_>,
        call: AbilityCall,
        name: &str,
        _input: &Value,
    ) -> Result<InterfaceReply, InterfaceError> {
        let Some(room) = ctx.w.get(call.asker).and_then(|o| o.location) else {
            return Err(InterfaceError::Refused("You are nowhere.".to_string()));
        };
        let Some((cell, def, sheltered)) = resolve_room_weather(ctx.w, &self.climates, room) else {
            return Err(InterfaceError::Refused(
                "There is no weather to speak of where you are standing.".to_string(),
            ));
        };
        match name {
            "weather.report" => {
                let snap = cell.snapshot(def);
                let mut line = format!(
                    "It is {} out: {}°C, wind {:.0} m/s, visibility {:.0}%.",
                    describe_state(&snap.state),
                    cell.temp,
                    snap.wind,
                    snap.visibility * 100.0
                );
                if sheltered {
                    line.push_str(" You are under cover here.");
                }
                Ok(InterfaceReply::public(line))
            }
            "weather.forecast" => {
                let Some(climate) = self.climates.get(&cell.climate) else {
                    return Err(InterfaceError::Refused(
                        "The climate here is not on file.".to_string(),
                    ));
                };
                // Every season's row for the current state, summed: no roll, no
                // clock, the table's own leaning.
                let mut totals: BTreeMap<&SmolStr, u64> = BTreeMap::new();
                for matrix in climate.seasonal.values() {
                    if let Some(row) = matrix.get(&cell.state) {
                        for (to, weight) in row {
                            *totals.entry(to).or_insert(0) += u64::from(*weight);
                        }
                    }
                }
                let stays = totals.get(&cell.state).copied().unwrap_or(0);
                let next = totals
                    .iter()
                    .filter(|(to, _)| ***to != cell.state)
                    .max_by_key(|(_, w)| **w)
                    .map(|(to, _)| (*to).clone());
                let now = describe_state(&cell.state);
                Ok(InterfaceReply::public(match next {
                    Some(to) if stays == 0 || totals.get(&to).copied().unwrap_or(0) >= stays => {
                        format!(
                            "It is {now} now; I'd say it turns {} next.",
                            describe_state(&to)
                        )
                    }
                    Some(to) => format!(
                        "It is {now} now and likely to hold; if it turns, it turns {}.",
                        describe_state(&to)
                    ),
                    None => format!("It is {now}, and it is staying that way."),
                }))
            }
            other => Err(InterfaceError::Unknown(other.to_string())),
        }
    }
}
