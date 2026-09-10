//! An explicit local test substitute for model responses. No provider request is made.
use crate::{
    Result,
    costs::{Amount, Receipt},
    model::Reply,
};
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, path::Path, sync::Mutex, time::Duration};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub version: u32,
    pub replies: Vec<Entry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    #[serde(default)]
    pub delay_ms: u64,
    pub content: Option<String>,
    pub error: Option<String>,
}

impl Source {
    pub fn load(path: &Path) -> Result<Self> {
        let source: Self = crate::experiment::load_json(path, 2_000_000)?;
        if source.version != 1
            || source.replies.is_empty()
            || source.replies.len() > 1000
            || source.replies.iter().any(|r| {
                r.delay_ms > 300_000
                    || r.content.is_some() == r.error.is_some()
                    || r.content.as_ref().is_some_and(|c| c.len() > 128_000)
                    || r.error
                        .as_ref()
                        .is_some_and(|e| e.is_empty() || e.len() > 4000)
            })
        {
            return Err("invalid model tape: version 1, 1..1000 replies with content OR error, delay <= 300000 ms".into());
        }
        Ok(source)
    }
}

pub struct Tape(Mutex<VecDeque<Entry>>);
impl Tape {
    pub fn new(source: Source) -> Self {
        Self(Mutex::new(source.replies.into()))
    }
    pub async fn reply(&self) -> Result<Reply> {
        // Consume before waiting: cancelled requests must not replay the same stale reply.
        let entry = self
            .0
            .lock()
            .map_err(|_| "model tape lock poisoned")?
            .pop_front();
        let entry = entry.unwrap_or(Entry {
            delay_ms: 0,
            content: None,
            error: Some("model tape exhausted".into()),
        });
        tokio::time::sleep(Duration::from_millis(entry.delay_ms)).await;
        Ok(Reply {
            content: entry.content.unwrap_or_default(),
            error: entry.error,
            usage: serde_json::json!({"synthetic":true,"cost":0}),
            receipt: Receipt {
                response_model: Some("test/tape".into()),
                provider: Some("local-test-tape".into()),
                currency: Some("USD".into()),
                cost: Some(Amount::default()),
                reported_cost: Some("0".into()),
                ..Receipt::default()
            },
        })
    }
}
