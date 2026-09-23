//! # The contacts table (interface plan I8, D-I8-1) — `tactics.contacts`
//!
//! The viewer's contacts as a panel keyframe in the generic-view convention
//! (interface plan I9: a declared `schema` plus `rows`), so both clients
//! render it with the renderer they already have and no unit vocabulary.
//!
//! **It is `+contacts`, cell for cell.** Every row is built by
//! [`contact_cells`] — the same function
//! [`render_contact`](crate::commands::render_contact) formats — over the same
//! roster `viewable_holders_of`
//! answers, so the table and the text can never disagree for one viewer at one
//! instant (§4 rule 4), and cut 7's leak arguments (a false contact renders
//! exactly as a real one; one observer's picture is invisible to another; no
//! dbref anywhere) extend to the row by construction rather than by argument.
//! `params` are ignored: the subject is always the actor.

use mush_core::{ContactLedgerState, ObjId, QueryHandler, World};
use mush_proto::{OutputEvent, PanelFrameKind, RichText};
use serde_json::{Value, json};
use smol_str::SmolStr;

use crate::commands::{
    UNAVAILABLE, contact_cells, holder_rows, viewable_holders_of, viewer_contact_on,
};
use crate::record::{ContactProvenance, PropagationStore, Provenance};

/// The feed id. Named for the family it projects (tactics owns contacts) —
/// the feed namespace has no owner fence, only uniqueness.
pub const CONTACTS_FEED: &str = "tactics.contacts";

/// The most rows one keyframe carries (== the web renderer's row cap); past
/// it `truncated` is set from the viewer's own row count, never from the
/// ledger's global paging flag.
pub const MAX_CONTACT_VIEW_ROWS: usize = 256;

/// The `tactics.contacts` query handler.
pub struct ContactsQuery {
    feed: SmolStr,
}

impl ContactsQuery {
    /// A handler answering on `feed`.
    #[must_use]
    pub fn new(feed: &str) -> ContactsQuery {
        ContactsQuery {
            feed: SmolStr::new(feed),
        }
    }
}

fn schema() -> Value {
    json!({
        "id": "tactics.contacts",
        "version": 1,
        "shape": "list",
        "columns": [
            { "key": "id",     "label": "contact", "align": "left"  },
            { "key": "gen",    "label": "gen",     "align": "right" },
            { "key": "rev",    "label": "rev",     "align": "right" },
            { "key": "state",  "label": "state",   "align": "left"  },
            { "key": "age",    "label": "age",     "align": "right" },
            { "key": "band",   "label": "band",    "align": "left"  },
            { "key": "source", "label": "source",  "align": "left"  },
        ],
        "fields": [],
        "actionKey": "prefill",
    })
}

impl QueryHandler for ContactsQuery {
    fn query(&self, w: &World, actor: ObjId, _params: &Value) -> Result<OutputEvent, RichText> {
        let (Some(ledger), Some(store)) = (
            w.module_state::<ContactLedgerState>(),
            w.module_state::<PropagationStore>(),
        ) else {
            return Err(RichText::tag("error", UNAVAILABLE));
        };
        let at = w.world_ic_now();
        let holders = viewable_holders_of(w, actor);
        // A FAIR share per holder (cut 7 fold O6, at the view layer too): a
        // flat cut over holder-sorted rows would starve every holder after
        // the first. Each holder keeps up to its share, then the remainder
        // fills in holder order; `truncated` says a row was cut.
        let per_holder: Vec<Vec<&mush_core::ContactLedgerRow>> = holders
            .iter()
            .map(|holder| holder_rows(ledger, std::slice::from_ref(holder)))
            .collect();
        let total: usize = per_holder.iter().map(Vec::len).sum();
        let truncated = total > MAX_CONTACT_VIEW_ROWS;
        let share = (MAX_CONTACT_VIEW_ROWS / holders.len().max(1)).max(1);
        let mut kept: Vec<&mush_core::ContactLedgerRow> = Vec::new();
        for rows in &per_holder {
            kept.extend(rows.iter().take(share));
        }
        for rows in &per_holder {
            for row in rows.iter().skip(share) {
                if kept.len() >= MAX_CONTACT_VIEW_ROWS {
                    break;
                }
                kept.push(row);
            }
        }
        kept.truncate(MAX_CONTACT_VIEW_ROWS);
        let rows: Vec<Value> = kept
            .iter()
            .map(|row| {
                let provenance = store.provenance_of(row.contact);
                let source = match provenance.map(ContactProvenance::provenance) {
                    Some(Provenance::Reported { source, .. }) => {
                        viewer_contact_on(ledger, &holders, *source)
                    }
                    _ => None,
                };
                let cells = contact_cells(row, provenance, &at, source);
                json!({
                    "id": cells.id,
                    "gen": cells.generation,
                    "rev": cells.rev,
                    "state": cells.state,
                    "age": cells.age,
                    "band": cells.band,
                    "source": cells.source,
                    "prefill": format!("+contact {}", cells.id),
                })
            })
            .collect();
        Ok(OutputEvent::Panel {
            feed: self.feed.clone(),
            kind: PanelFrameKind::Keyframe,
            data: json!({ "schema": schema(), "rows": rows, "truncated": truncated }),
        })
    }
}
