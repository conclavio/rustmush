//! Deduplicate the common terrain and immutable source bytes across a proving
//! series. Reconstructing a header reproduces its original canonical tape root.
use super::{BulkError, invalid, model::MAX_ARTIFACT_BYTES};
use crate::arena::{
    clock::ReplayClock,
    input_tape::{EngineStamp, FrozenSource, TapeHeader},
    mind_seal::MindSeal,
    replay_ground::ReplayGround,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceRef {
    seal: [u8; 32],
    sha256: [u8; 32],
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GameStart {
    pub seed: u64,
    pub swapped: bool,
    format: u16,
    match_id: u64,
    limit: u64,
    engine: EngineStamp,
    opening: serde_json::Value,
    clock: ReplayClock,
    pub seals: Vec<MindSeal>,
    sources: Vec<SourceRef>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FrozenStarts {
    format: u16,
    ground: Option<ReplayGround>,
    sources: BTreeMap<String, String>,
    pub games: Vec<GameStart>,
    pub context: Option<super::report::Context>,
    #[serde(skip)]
    estimated_bytes: usize,
}
impl FrozenStarts {
    /// Worker-side decode only, with the encoded allocation capped before
    /// serde and every reconstructed header revalidated before execution.
    pub fn decode(bytes: &[u8]) -> Result<Self, BulkError> {
        if bytes.is_empty() || bytes.len() as u64 > MAX_ARTIFACT_BYTES {
            return Err(invalid("frozen proving byte bound"));
        }
        let value: Self =
            serde_json::from_slice(bytes).map_err(|_| invalid("frozen proving document"))?;
        if value.format != 1
            || !(10..=100).contains(&value.games.len())
            || !value.games.len().is_multiple_of(2)
            || value.sources.len() > 1_200
            || value
                .games
                .iter()
                .map(|g| g.match_id)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != value.games.len()
        {
            return Err(invalid("frozen proving dimensions"));
        }
        value
            .context
            .as_ref()
            .ok_or_else(|| invalid("missing frozen report context"))?
            .validate()?;
        let mut used = std::collections::BTreeSet::new();
        for (i, game) in value.games.iter().enumerate() {
            let first = value
                .games
                .get(i / 2 * 2)
                .ok_or_else(|| invalid("frozen proving seed pair"))?;
            if game.swapped != (i % 2 == 1)
                || game.seed != first.seed
                || game.sources.len() > 12
                || !(2..=12).contains(&game.seals.len())
            {
                return Err(invalid("frozen proving paired sides or roster"));
            }
            for source in &game.sources {
                used.insert(crate::arena::digest_text(&source.sha256));
            }
            value.header(i)?;
        }
        if used.len() != value.sources.len() || value.sources.keys().any(|key| !used.contains(key))
        {
            return Err(invalid("unreferenced frozen source"));
        }
        Ok(value)
    }
    pub fn new() -> Self {
        Self {
            format: 1,
            ground: None,
            sources: BTreeMap::new(),
            games: Vec::new(),
            context: None,
            estimated_bytes: 256,
        }
    }
    pub fn push(&mut self, seed: u64, swapped: bool, header: TapeHeader) -> Result<(), BulkError> {
        header
            .validate_shape()
            .map_err(|e| invalid(e.to_string()))?;
        if self.games.len() >= 100 || self.games.iter().any(|g| g.match_id == header.match_id) {
            return Err(invalid("frozen proving game identity bound"));
        }
        if let Some(ground) = &self.ground {
            if ground != &header.ground {
                return Err(invalid("proving games do not share one frozen ground"));
            }
        } else {
            self.estimated_bytes +=
                super::encoding::encode(&header.ground, MAX_ARTIFACT_BYTES as usize)?.len() + 32;
            self.ground = Some(header.ground.clone());
        }
        let mut sources = Vec::new();
        for source in header.sources {
            let key = crate::arena::digest_text(&source.sha256);
            match self.sources.get(&key) {
                Some(old) if old != &source.text => return Err(invalid("source digest collision")),
                Some(_) => {}
                None => {
                    self.estimated_bytes +=
                        super::encoding::encode(&source.text, MAX_ARTIFACT_BYTES as usize)?.len()
                            + key.len()
                            + 8;
                    self.sources.insert(key, source.text);
                }
            }
            sources.push(SourceRef {
                seal: source.seal,
                sha256: source.sha256,
            });
        }
        let game = GameStart {
            seed,
            swapped,
            format: header.format,
            match_id: header.match_id,
            limit: header.limit,
            engine: header.engine,
            opening: header.opening,
            clock: header.clock,
            seals: header.seals,
            sources,
        };
        self.estimated_bytes +=
            super::encoding::encode(&game, MAX_ARTIFACT_BYTES as usize)?.len() + 2;
        if self.estimated_bytes > MAX_ARTIFACT_BYTES as usize {
            return Err(invalid("frozen proving start byte bound"));
        }
        self.games.push(game);
        Ok(())
    }
    pub fn header(&self, index: usize) -> Result<TapeHeader, BulkError> {
        if self.format != 1 {
            return Err(invalid("frozen proving start format"));
        }
        let game = self.games.get(index).ok_or(BulkError::Missing)?;
        let sources = game
            .sources
            .iter()
            .map(|r| {
                Ok(FrozenSource {
                    seal: r.seal,
                    sha256: r.sha256,
                    text: self
                        .sources
                        .get(&crate::arena::digest_text(&r.sha256))
                        .cloned()
                        .ok_or_else(|| invalid("frozen proving source missing"))?,
                })
            })
            .collect::<Result<Vec<_>, BulkError>>()?;
        let header = TapeHeader {
            format: game.format,
            match_id: game.match_id,
            limit: game.limit,
            engine: game.engine.clone(),
            opening: game.opening.clone(),
            clock: game.clock,
            ground: self
                .ground
                .clone()
                .ok_or_else(|| invalid("frozen proving ground missing"))?,
            seals: game.seals.clone(),
            sources,
        };
        header
            .validate_shape()
            .map_err(|e| invalid(e.to_string()))?;
        Ok(header)
    }
}
