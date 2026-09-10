//! S6 (doc 14 §4.6): the front office's interface — `request.file` and
//! `request.status` over `+request`, the asker's own tickets only.

use mush_core::{
    AbilityCall, Ctx, InterfaceAbility, InterfaceAbilityProvider, InterfaceError, InterfaceReply,
};
use serde_json::{Value, json};
use smol_str::SmolStr;

use crate::api::{OpenJob, open_job};
use crate::data::JobsState;
use crate::{MAX_JOB_TITLE_BYTES, REQ_QUEUE};

/// The provider id.
pub const REQUEST_INTERFACE: &str = "request";

/// The most tickets `request.status` lists.
pub const MAX_STATUS_ROWS: usize = 10;

/// The front office.
#[derive(Clone, Copy, Debug, Default)]
pub struct RequestInterface;

impl InterfaceAbilityProvider for RequestInterface {
    fn abilities(&self) -> Vec<InterfaceAbility> {
        vec![
            InterfaceAbility {
                name: SmolStr::new_static("request.file"),
                description:
                    "File a request with staff in the asker's name: a one-line title and the text."
                        .to_string(),
                params_schema: json!({
                    "type": "object",
                    "properties": {
                        "title": { "type": "string" },
                        "text": { "type": "string" }
                    },
                    "required": ["title"]
                }),
                command: SmolStr::new_static("+request"),
            },
            InterfaceAbility {
                name: SmolStr::new_static("request.status"),
                description: "The asker's own open requests and where each stands.".to_string(),
                params_schema: json!({ "type": "object", "properties": {} }),
                command: SmolStr::new_static("+request"),
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
        match name {
            "request.file" => {
                let title = input
                    .get("title")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .ok_or_else(|| InterfaceError::Invalid("a title".to_string()))?;
                if title.len() > MAX_JOB_TITLE_BYTES {
                    return Err(InterfaceError::Refused(format!(
                        "A title is at most {MAX_JOB_TITLE_BYTES} bytes."
                    )));
                }
                let text = input
                    .get("text")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .map(str::to_string);
                let id = open_job(
                    ctx,
                    OpenJob {
                        queue: SmolStr::new_static(REQ_QUEUE),
                        title: title.to_string(),
                        opener: call.asker,
                        text,
                        payload: None,
                        tags: vec![SmolStr::new_static("interface")],
                    },
                )
                .map_err(InterfaceError::Refused)?;
                Ok(InterfaceReply::text(format!(
                    "Filed as request #{id}. Staff read every one; `+request` shows where it stands."
                )))
            }
            "request.status" => {
                let Some(st) = ctx.w.module_state::<JobsState>() else {
                    return Err(InterfaceError::Refused(
                        "The front office is closed on this server.".to_string(),
                    ));
                };
                // The asker's own, newest first — never anyone else's ticket.
                let mut rows: Vec<String> = st
                    .jobs
                    .values()
                    .filter(|j| j.opener == call.asker)
                    .rev()
                    .take(MAX_STATUS_ROWS)
                    .map(|j| format!("#{} {} — {}", j.id, j.status, j.title))
                    .collect();
                if rows.is_empty() {
                    rows.push("You have no requests on file.".to_string());
                }
                Ok(InterfaceReply::lines(&rows))
            }
            other => Err(InterfaceError::Unknown(other.to_string())),
        }
    }
}
