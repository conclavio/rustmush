//! Deduplicate common terrain/source bytes and compress each exact game body.
//! Only one bounded body is expanded at a time. In particular, the snapshot's
//! opaque canonical state string is never parsed/reordered during packaging.
use super::{BulkError, invalid, model::MAX_ARTIFACT_BYTES};
use crate::arena::{
    clock::ReplayClock,
    input_tape::{EngineStamp, FrozenSource, TapeHeader},
    mind_seal::MindSeal,
    replay_ground::ReplayGround,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const FORMAT: u16 = 2;
// The existing per-header/artifact bound also bounds the expanded per-game
// body. Compression improves packing, never enlarges an admitted raw game.
const MAX_GAME_BYTES: usize = MAX_ARTIFACT_BYTES as usize;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceRef {
    seal: [u8; 32],
    sha256: [u8; 32],
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GameBody {
    seed: u64,
    swapped: bool,
    format: u16,
    match_id: u64,
    limit: u64,
    engine: EngineStamp,
    opening: serde_json::Value,
    clock: ReplayClock,
    seals: Vec<MindSeal>,
    sources: Vec<SourceRef>,
}
/// Small queryable summary; every field is checked against the bounded body.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GameStart {
    pub seed: u64,
    pub swapped: bool,
    match_id: u64,
    pub unit_count: u16,
    raw_bytes: u32,
    raw_sha256: String,
    zlib: String,
}
impl GameStart {
    fn pack(body: &GameBody) -> Result<Self, BulkError> {
        if body.match_id == 0 || !(2..=12).contains(&body.seals.len()) || body.sources.len() > 12 {
            return Err(invalid("frozen proving game roster"));
        }
        let raw = super::encoding::encode(body, MAX_GAME_BYTES)?;
        let compressed = super::encoding::zlib(&raw, MAX_GAME_BYTES)?;
        Ok(Self {
            seed: body.seed,
            swapped: body.swapped,
            match_id: body.match_id,
            unit_count: body.seals.len() as u16,
            raw_bytes: raw.len() as u32,
            raw_sha256: crate::arena::digest_text(&Sha256::digest(&raw).into()),
            zlib: hex(&compressed),
        })
    }
    fn body(&self) -> Result<GameBody, BulkError> {
        let length = self.raw_bytes as usize;
        if self.match_id == 0
            || !(2..=12).contains(&self.unit_count)
            || !(1..=MAX_GAME_BYTES).contains(&length)
            || self.raw_sha256.len() != 64
        {
            return Err(invalid("frozen proving game dimensions"));
        }
        let compressed = unhex(&self.zlib)?;
        // The codec writes into this fixed slice. A forged length or compressed
        // bomb cannot grow it; the extra byte detects an undersized declaration.
        let mut raw = vec![0; length + 1];
        let mut decoder = flate2::Decompress::new(true);
        let status = decoder
            .decompress(&compressed, &mut raw, flate2::FlushDecompress::Finish)
            .map_err(|_| invalid("frozen proving compressed body"))?;
        if status != flate2::Status::StreamEnd
            || decoder.total_in() != compressed.len() as u64
            || decoder.total_out() != u64::from(self.raw_bytes)
        {
            return Err(invalid(
                "frozen proving truncated, trailing or oversized body",
            ));
        }
        raw.truncate(length);
        if crate::arena::digest_text(&Sha256::digest(&raw).into()) != self.raw_sha256 {
            return Err(invalid("frozen proving body digest"));
        }
        let body: GameBody =
            serde_json::from_slice(&raw).map_err(|_| invalid("frozen proving game body"))?;
        if body.seed != self.seed
            || body.swapped != self.swapped
            || body.match_id != self.match_id
            || body.seals.len() != usize::from(self.unit_count)
            || body.sources.len() > 12
        {
            return Err(invalid("frozen proving summary differs from body"));
        }
        Ok(body)
    }
}
fn hex(bytes: &[u8]) -> String {
    let digit = |n: u8| char::from(if n < 10 { b'0' + n } else { b'a' + (n - 10) });
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(digit(byte >> 4));
        out.push(digit(byte & 15));
    }
    out
}
fn unhex(text: &str) -> Result<Vec<u8>, BulkError> {
    if text.is_empty() || text.len() > MAX_GAME_BYTES * 2 || !text.len().is_multiple_of(2) {
        return Err(invalid("frozen proving compressed byte bound"));
    }
    let digit = |byte| match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(invalid("frozen proving noncanonical compressed hex")),
    };
    text.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|[high, low]| Ok((digit(*high)? << 4) | digit(*low)?))
        .collect()
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
    /// Worker-side decode only: cap the complete encoded document before serde,
    /// expand/validate one bounded game at a time, retain only compressed bodies.
    pub fn decode(bytes: &[u8]) -> Result<Self, BulkError> {
        if bytes.is_empty() || bytes.len() as u64 > MAX_ARTIFACT_BYTES {
            return Err(invalid("frozen proving byte bound"));
        }
        // Probe only the version first: old game bodies have a different
        // shape and must receive the named format refusal, not a decode error.
        // Ignored fields are streamed past without retaining their values.
        #[derive(Deserialize)]
        struct Version {
            format: u16,
        }
        let version: Version =
            serde_json::from_slice(bytes).map_err(|_| invalid("frozen proving document"))?;
        if version.format != FORMAT {
            return Err(invalid("unsupported frozen proving start format"));
        }
        let value: Self =
            serde_json::from_slice(bytes).map_err(|_| invalid("frozen proving document"))?;
        if !(10..=100).contains(&value.games.len())
            || !value.games.len().is_multiple_of(2)
            || value.sources.len() > 1_200
            || value
                .games
                .iter()
                .map(|g| g.match_id)
                .collect::<BTreeSet<_>>()
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
        let mut used = BTreeSet::new();
        for (i, game) in value.games.iter().enumerate() {
            let first = value
                .games
                .get(i / 2 * 2)
                .ok_or_else(|| invalid("frozen proving seed pair"))?;
            if game.swapped != (i % 2 == 1) || game.seed != first.seed {
                return Err(invalid("frozen proving paired sides or roster"));
            }
            let body = game.body()?;
            for source in &body.sources {
                used.insert(crate::arena::digest_text(&source.sha256));
            }
            value.expand(body)?;
        }
        if used.len() != value.sources.len() || value.sources.keys().any(|key| !used.contains(key))
        {
            return Err(invalid("unreferenced frozen source"));
        }
        Ok(value)
    }
    pub fn new() -> Self {
        Self {
            format: FORMAT,
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
        if !(2..=12).contains(&header.seals.len()) || header.sources.len() > 12 {
            return Err(invalid("frozen proving game roster"));
        }
        if self.games.len() >= 100 || self.games.iter().any(|g| g.match_id == header.match_id) {
            return Err(invalid("frozen proving game identity bound"));
        }
        if self
            .ground
            .as_ref()
            .is_some_and(|ground| ground != &header.ground)
        {
            return Err(invalid("proving games do not share one frozen ground"));
        }
        // Stage all additions before publishing even this private collection.
        let mut estimate = self.estimated_bytes;
        if self.ground.is_none() {
            estimate = estimate
                .saturating_add(super::encoding::encode(&header.ground, MAX_GAME_BYTES)?.len())
                .saturating_add(32);
        }
        let mut added = BTreeMap::new();
        let mut sources = Vec::new();
        for source in header.sources {
            let key = crate::arena::digest_text(&source.sha256);
            match self.sources.get(&key).or_else(|| added.get(&key)) {
                Some(old) if old != &source.text => return Err(invalid("source digest collision")),
                Some(_) => {}
                None => {
                    estimate = estimate
                        .saturating_add(
                            super::encoding::encode(&source.text, MAX_GAME_BYTES)?.len(),
                        )
                        .saturating_add(key.len() + 8);
                    added.insert(key, source.text);
                }
            }
            sources.push(SourceRef {
                seal: source.seal,
                sha256: source.sha256,
            });
        }
        let game = GameStart::pack(&GameBody {
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
        })?;
        let packed_game_bytes = super::encoding::encode(&game, MAX_GAME_BYTES)?.len();
        estimate = estimate.saturating_add(packed_game_bytes).saturating_add(2);
        if estimate > MAX_GAME_BYTES {
            return Err(invalid("frozen proving start byte bound"));
        }
        if self.ground.is_none() {
            self.ground = Some(header.ground);
        }
        self.sources.extend(added);
        self.estimated_bytes = estimate;
        self.games.push(game);
        Ok(())
    }
    pub fn header(&self, index: usize) -> Result<TapeHeader, BulkError> {
        if self.format != FORMAT {
            return Err(invalid("unsupported frozen proving start format"));
        }
        self.expand(self.games.get(index).ok_or(BulkError::Missing)?.body()?)
    }
    fn expand(&self, body: GameBody) -> Result<TapeHeader, BulkError> {
        let sources = body
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
            format: body.format,
            match_id: body.match_id,
            limit: body.limit,
            engine: body.engine,
            opening: body.opening,
            clock: body.clock,
            ground: self
                .ground
                .clone()
                .ok_or_else(|| invalid("frozen proving ground missing"))?,
            seals: body.seals,
            sources,
        };
        header
            .validate_shape()
            .map_err(|e| invalid(e.to_string()))?;
        Ok(header)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::{
        Team,
        input_tape::TapeRecorder,
        test_support::{arena_of, pose, tank},
    };
    use mush_core::ObjId;

    fn header_for(match_id: u64, scripted: bool) -> TapeHeader {
        // Full lawful custody and the opaque format13 state string travel
        // through the real capture. The scripted case also stages shared source.
        let mut red = tank(ObjId(4), Team::Red, pose(1500, 1500));
        let mut blue = tank(ObjId(5), Team::Blue, pose(5500, 1500));
        let terms = crate::match_::SealTerms {
            match_id: crate::arena_task::MatchId(match_id),
            engine_for_minds: if scripted {
                crate::arena::mind::SealEngine::Scripted
            } else {
                crate::arena::mind::SealEngine::Native
            },
            ..crate::match_::SealTerms::fixture()
        };
        red.reseal(&terms).unwrap();
        blue.reseal(&terms).unwrap();
        let seals = vec![red.seal.clone(), blue.seal.clone()];
        let sources = if scripted {
            seals
                .iter()
                .map(|seal| (seal.digest(), crate::minds::STANDARD_RHAI.to_owned()))
                .collect()
        } else {
            BTreeMap::new()
        };
        let arena = arena_of(vec![red, blue]);
        TapeHeader::capture(
            &arena,
            match_id,
            seals,
            &sources,
            EngineStamp::current(),
            MAX_ARTIFACT_BYTES,
        )
        .unwrap()
    }
    fn packed() -> (FrozenStarts, TapeHeader) {
        let header = header_for(1, false);
        let mut starts = FrozenStarts::new();
        starts.push(1, false, header.clone()).unwrap();
        (starts, header)
    }
    #[test]
    fn m6_frozen_v2_preserves_every_header_byte_and_tape_root() {
        let (starts, original) = packed();
        let restored = starts.header(0).unwrap();
        assert_eq!(
            serde_json::to_vec(&restored).unwrap(),
            serde_json::to_vec(&original).unwrap()
        );
        assert_eq!(
            restored.opening.get("state").unwrap(),
            original.opening.get("state").unwrap()
        );
        let a = TapeRecorder::open(&original).unwrap();
        let b = TapeRecorder::open(&restored).unwrap();
        assert_eq!(a.cursor(), b.cursor());
    }
    #[test]
    fn m6_frozen_v2_refuses_corrupt_truncated_trailing_bomb_and_summary() {
        let (starts, _) = packed();
        let original = starts.games.first().unwrap();
        let rejects = |game: GameStart| assert!(game.body().is_err());
        let mut bad = original.clone();
        bad.zlib.truncate(bad.zlib.len() - 2);
        rejects(bad);
        let mut bad = original.clone();
        bad.zlib.push_str("00");
        rejects(bad);
        let mut bad = original.clone();
        bad.zlib.push_str(&original.zlib);
        rejects(bad);
        let mut bad = original.clone();
        let end = bad.zlib.len();
        let replacement = if bad.zlib.get(end - 2..).unwrap() == "00" {
            "01"
        } else {
            "00"
        };
        bad.zlib.replace_range(end - 2..end, replacement);
        rejects(bad);
        let mut bad = original.clone();
        bad.raw_bytes = 1;
        rejects(bad);
        let mut bad = original.clone();
        bad.raw_bytes = MAX_GAME_BYTES as u32 + 1;
        rejects(bad);
        let mut bad = original.clone();
        bad.raw_sha256 = "0".repeat(64);
        rejects(bad);
        let mut bad = original.clone();
        bad.seed += 1;
        rejects(bad);
        let mut bad = original.clone();
        bad.swapped = !bad.swapped;
        rejects(bad);
        let mut bad = original.clone();
        bad.match_id += 1;
        rejects(bad);
        let mut bad = original.clone();
        bad.unit_count += 1;
        rejects(bad);
        let mut bad = original.clone();
        bad.zlib = bad.zlib.to_uppercase();
        rejects(bad);
        let mut bad = original.clone();
        bad.zlib.push('0');
        rejects(bad);
    }
    #[test]
    fn m6_frozen_v2_refuses_old_format_and_does_not_publish_failed_additions() {
        let (mut starts, original) = packed();
        let before = serde_json::to_vec(&starts).unwrap();
        assert!(starts.push(2, true, original).is_err());
        assert_eq!(serde_json::to_vec(&starts).unwrap(), before);

        // Reach the late cumulative-cap refusal after encoding the ground,
        // staging two shared-source references and packing a new valid game.
        // A near-full counter avoids allocating a filler artifact just for this.
        let mut full = FrozenStarts::new();
        full.estimated_bytes = MAX_GAME_BYTES;
        let before = serde_json::to_vec(&full).unwrap();
        let before_estimate = full.estimated_bytes;
        let fresh = header_for(2, true);
        assert_eq!(fresh.sources.len(), 2);
        assert_eq!(
            full.push(2, false, fresh.clone()).unwrap_err(),
            invalid("frozen proving start byte bound")
        );
        assert_eq!(serde_json::to_vec(&full).unwrap(), before);
        assert_eq!(full.estimated_bytes, before_estimate);
        assert!(full.ground.is_none() && full.sources.is_empty() && full.games.is_empty());
        let mut clean = FrozenStarts::new();
        full.estimated_bytes = clean.estimated_bytes;
        full.push(2, false, fresh.clone()).unwrap();
        clean.push(2, false, fresh).unwrap();
        assert_eq!(
            serde_json::to_vec(&full).unwrap(),
            serde_json::to_vec(&clean).unwrap()
        );
        assert_eq!(full.estimated_bytes, clean.estimated_bytes);
        assert_eq!(
            full.sources.len(),
            1,
            "the exact shared source remains deduplicated"
        );
        assert_eq!(
            full.header(0).unwrap().sources.len(),
            2,
            "both seals retain their exact source references"
        );

        starts.format = 1;
        let expected = invalid("unsupported frozen proving start format");
        assert_eq!(starts.header(0).unwrap_err(), expected);
        assert_eq!(
            FrozenStarts::decode(&serde_json::to_vec(&starts).unwrap()).unwrap_err(),
            expected
        );
        let mut old = serde_json::to_value(&starts).unwrap();
        *old.get_mut("games").unwrap() = serde_json::json!([{
            "seed": 1, "swapped": false, "format": 1, "match_id": 1,
            "opening": {"state": "old canonical bytes"}, "seals": []
        }]);
        assert_eq!(
            FrozenStarts::decode(&serde_json::to_vec(&old).unwrap()).unwrap_err(),
            expected,
            "old game shape is refused by version before the new body decoder"
        );
    }
}
