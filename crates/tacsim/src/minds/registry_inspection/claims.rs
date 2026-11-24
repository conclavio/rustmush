//! **The quirk claim** (doc 17h cut M8g G3; doc 17 §10 "The quirk", D-UM-55), tacsim's half of
//! [`mush_roll::commerce::registry_inspection::claims`].
//!
//! * `@registry/claims` (`read`): the Registry operators' read of what authors have filed. It
//!   is a line-mode read, gated live by `roll.registry.claims` through its own observe twin
//!   (the lead's ruling (a): a read writes nothing, so it has no action, no projected offer and
//!   no `registry_verbs` entry). Anyone the writer does not authorize now — no operator, or an
//!   operator whose grant was revoked — gets the driver's one sentence and nothing else.
//!
//! * `@mind/claim <hull> [<hull>...] <text>` (`file`) and `mind_quirk_claim`'s provider
//!   (`Provider`): an author (a charter title holder) files testimony naming hulls their own
//!   sensors earned ([`earned`]). A hull the author never earned is refused with the observer
//!   rule's one sentence ([`crate::surface::hull::NO_BOX`], `+hull/box`'s: no new wording);
//!   anyone who holds no charter title gets the driver's one sentence. The author's reply is the
//!   provider's receipt, "Your claim is filed." - one line that confirms nothing.
//!
//! A claim is testimony: nothing here confirms or denies a likeness, and the read prints the
//! author's words as theirs.

use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;

use mush_core::action::{
    ActionError, ActionErrorCode, ActionProvider, AuthoritativeActionOutput, PrincipalId,
};
use mush_core::{
    ActionInvocation, ActionTxn, CmdError, Ctx, DefinitionRef, ModError, ObjId, PerceivedRef,
    PrepareCtx, World,
};
use mush_roll::commerce::registry_inspection::{
    ClaimRecord, MAX_CLAIM_HULLS, RegisteredInspectionWriter, claims::valid_text,
};
use serde::Deserialize;
use serde_json::json;

use super::InspectionHost;
use crate::minds::market_commands::words::{self, fill};

/// The claim action's id (the Roll's policy's).
pub const CLAIM_ACTION: &str = "mind_quirk_claim";
/// The claim outcome's id.
pub const CLAIM_OUTCOME: &str = "mind_quirk_claimed";
/// `@mind/claim`'s usage line, its `@mind` help line.
pub(crate) const USAGE: &str = "@mind/claim #<hull> [#<hull>...] <text> (testimony to the Registry: hulls your own sensors saw)";

/// What the claim composed: its registered action, for the command's submission.
#[derive(Clone)]
pub struct ClaimHost {
    pub(crate) action: DefinitionRef,
}

/// **Whether `author`'s own sensors earned `hull`** (D-UM-55: recognition rests only on what
/// the author's sensors earned in the field, the observer rule).
#[must_use]
pub fn earned(w: &World, author: ObjId, hull: ObjId) -> bool {
    crate::minds::earned_contacts::has_earned(w, author, hull)
}

/// Whether `author` may claim: a holder of a mind charter title.
fn holds_charter(w: &World, author: ObjId) -> bool {
    w.module_state::<mush_economy::EconomyStore>()
        .is_some_and(|e| {
            !e.is_frozen()
                && e.snapshot()
                    .holds_charter_title(&mush_org::PrincipalId::Object(author))
        })
}

/// `<hull> [<hull>...] <text>`: the leading `#<n>` words are the hulls (ascending, distinct,
/// at most [`MAX_CLAIM_HULLS`]), the rest is the author's words.
fn parse(raw: &str) -> Option<(Vec<ObjId>, String)> {
    let mut rest = raw.trim();
    let mut hulls = Vec::new();
    loop {
        let (word, after) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        let Some(number) = word.strip_prefix('#') else {
            break;
        };
        let number = crate::port::families::strict_decimal_u32(number).filter(|n| *n != 0)?;
        hulls.push(ObjId(number));
        rest = after.trim_start();
    }
    hulls.sort_unstable();
    hulls.dedup();
    let text = rest.trim();
    (!hulls.is_empty() && hulls.len() <= MAX_CLAIM_HULLS && valid_text(text))
        .then(|| (hulls, text.to_owned()))
}

/// **`@mind/claim #<hull> [#<hull>...] <text>`**: check the author and each hull, then submit
/// `mind_quirk_claim`. Prints nothing: the receipt is the provider's.
pub(crate) fn file(ctx: &mut Ctx<'_>, text: &str) -> Result<(), CmdError> {
    let (hulls, said) = parse(text).ok_or_else(|| CmdError::msg(USAGE))?;
    let actor = ctx.actor;
    let (action, key) = {
        let w: &World = &*ctx.w;
        let host = InspectionHost::of(w).ok_or_else(|| CmdError::msg(words::CLAIMS_OFF))?;
        let claim = host
            .claims
            .as_ref()
            .ok_or_else(|| CmdError::msg(words::CLAIMS_OFF))?;
        // The observer rule first: a hull the actor's own sensors never earned is the one
        // sentence, whoever asks (only an IC field's own-sensor classification earns one:
        // `earned_contacts`, doc 23 cut S.4b-T).
        if hulls.iter().any(|hull| !earned(w, actor, *hull)) {
            return Err(CmdError::msg(crate::surface::hull::NO_BOX));
        }
        if !holds_charter(w, actor) {
            return Err(CmdError::msg(words::INSPECT_REFUSED));
        }
        if ctx.authoritative_pending(actor) || ctx.authoritative_queue_full() {
            return Err(CmdError::msg(words::BUSY));
        }
        let at = w.world_ic_now().at;
        let key = crate::minds::market_commands::next_key(
            &host.attempts,
            "claim",
            &actor.0.to_string(),
            at,
        )?;
        (claim.action.clone(), key)
    };
    ctx.submit_authoritative(
        action,
        PerceivedRef::SelfActor,
        Vec::new(),
        json!({"hulls": hull_list(&hulls), "text": said}),
        key,
    );
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Parameters {
    /// The hulls as one canonical list, `77,91` (an offer's parameters are scalars).
    hulls: String,
    text: String,
}

/// The wire's hull list: ascending decimal numbers joined by commas.
fn hull_list(hulls: &[ObjId]) -> String {
    hulls
        .iter()
        .map(|hull| hull.0.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

/// The longest hull list the wire admits.
const HULL_LIST_BYTES: usize = MAX_CLAIM_HULLS * 11;

/// `mind_quirk_claim`'s provider: the author's charter title and each hull's earning again in
/// the prepare, then the Roll's record through the writer; the receipt confirms nothing.
pub(super) struct Provider {
    action: DefinitionRef,
    outcome: DefinitionRef,
    writer: RegisteredInspectionWriter,
    refusal: ActionError,
}
impl Provider {
    pub(super) fn new(
        action: DefinitionRef,
        outcome: DefinitionRef,
        writer: RegisteredInspectionWriter,
    ) -> Result<Self, ModError> {
        Ok(Self {
            action,
            outcome,
            writer,
            refusal: ActionError::new(
                ActionErrorCode::ProviderRejected,
                false,
                mush_core::action::SUBMISSION_REFUSED_LINE,
            )
            .map_err(|e| ModError::msg(e.to_string()))?,
        })
    }
    /// The hulls and words, when the invocation is the actor's own, canonical and bounded.
    fn request(&self, inv: &ActionInvocation) -> Result<(Vec<ObjId>, String), ActionError> {
        if inv.principal() != &(PrincipalId::Object { id: inv.actor() })
            || inv.subject() != &PerceivedRef::SelfActor
            || !inv.targets().is_empty()
        {
            return Err(self.refusal.clone());
        }
        let p: Parameters =
            serde_json::from_value(inv.parameters().clone()).map_err(|_| self.refusal.clone())?;
        let hulls = p
            .hulls
            .split(',')
            .map(|word| crate::port::families::strict_decimal_u32(word).filter(|n| *n != 0))
            .collect::<Option<Vec<u32>>>()
            .ok_or_else(|| self.refusal.clone())?;
        if hulls.is_empty()
            || hulls.len() > MAX_CLAIM_HULLS
            || hulls
                .windows(2)
                .any(|pair| matches!(pair, [a, b] if a >= b))
            || !valid_text(&p.text)
        {
            return Err(self.refusal.clone());
        }
        Ok((hulls.into_iter().map(ObjId).collect(), p.text))
    }
    fn definitions(&self) -> Result<Vec<DefinitionRef>, ActionError> {
        let authority = self.writer.authority().map_err(|_| self.refusal.clone())?;
        Ok(vec![self.action.clone(), self.outcome.clone(), authority])
    }
}
impl ActionProvider for Provider {
    fn validate_invocation(&self, inv: &ActionInvocation) -> Result<(), ActionError> {
        self.request(inv).map(|_| ())
    }
    fn prepare_authoritative_with_context(
        &self,
        inv: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
        ctx: &mut PrepareCtx<'_>,
    ) -> Result<AuthoritativeActionOutput, ActionError> {
        let (hulls, said) = self.request(inv)?;
        let author = inv.actor();
        {
            let world = ctx.world().map_err(|_| self.refusal.clone())?;
            if !holds_charter(world, author)
                || hulls.iter().any(|hull| !earned(world, author, *hull))
            {
                return Err(self.refusal.clone());
            }
        }
        let record = self
            .writer
            .file_quirk_claim(ctx, txn, &self.action, author, hulls, said.into())
            .map_err(|_| self.refusal.clone())?;
        let result = mush_core::mechanics::RulesValue::new(
            self.outcome.clone(),
            json!({"claim": record.claim, "author": author.0, "seq": record.seq}),
        )
        .map_err(|_| self.refusal.clone())?;
        Ok(AuthoritativeActionOutput::new(result)
            .with_definitions(self.definitions()?)
            .with_eligible_witnesses(vec![inv.principal().clone()]))
    }
    /// Offered to a holder of a mind charter title, where the claim composes (the command's
    /// submission resolves this offer). Which hulls the holder earned is the prepare's.
    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<mush_core::ProviderOfferDraft>, ActionError> {
        if action != &self.action
            || query.truth_subject() != &(mush_core::SubjectKey::Object { id: query.actor() })
            || query.projected_subject() != &PerceivedRef::SelfActor
            || !holds_charter(&engine.world, query.actor())
        {
            return Ok(None);
        }
        let fail = || self.refusal.clone();
        let definitions = engine.registry.definitions().ok_or_else(fail)?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(mush_core::RevisionSet::default())
            .map_err(|_| fail())?;
        for r in self.definitions()? {
            guards.add_definition(r, definitions).map_err(|_| fail())?;
        }
        let text =
            |id: &str, maximum_bytes| -> Result<mush_core::ProjectedParameter, ActionError> {
                Ok(mush_core::ProjectedParameter {
                    id: mush_core::mechanics::CanonicalId::new(id).map_err(|_| fail())?,
                    required: true,
                    kind: mush_core::ProjectedParameterKind::Text { maximum_bytes },
                })
            };
        Ok(Some(mush_core::ProviderOfferDraft {
            fixed_parameters: json!({}),
            parameters: mush_core::ProjectedParameterSchema::new(vec![
                text("hulls", HULL_LIST_BYTES)?,
                text(
                    "text",
                    mush_roll::commerce::registry_inspection::MAX_CLAIM_TEXT,
                )?,
            ])
            .map_err(|_| fail())?,
            targets: mush_core::ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: mush_core::ActionPresentation::new(
                "mind-quirk-claim",
                "File a quirk claim",
                "Tell the Registry which hulls your own sensors saw behaving like your mind. It confirms nothing.",
            )
            .map_err(|_| fail())?,
            guards: guards.finish().map_err(|_| fail())?,
        }))
    }
    /// **The receipt**: "Your claim is filed.", for the author alone; it names no hull, no
    /// mind and no finding.
    fn project_receipt(
        &self,
        action: &DefinitionRef,
        outcome: &mush_core::mechanics::RulesValue,
        viewer: &PrincipalId,
    ) -> Result<Option<mush_proto::ProjectedActionResult>, ActionError> {
        if action != &self.action || outcome.schema() != &self.outcome {
            return Err(self.refusal.clone());
        }
        let data = outcome.data();
        let author = data
            .get("author")
            .and_then(serde_json::Value::as_u64)
            .and_then(|id| u32::try_from(id).ok())
            .map(ObjId)
            .ok_or_else(|| self.refusal.clone())?;
        if viewer != &(PrincipalId::Object { id: author }) {
            return Err(self.refusal.clone());
        }
        mush_proto::ProjectedActionResult::new(CLAIM_ACTION, words::CLAIM_FILED, data.clone())
            .map(Some)
            .map_err(|_| self.refusal.clone())
    }
}

/// The claims one read prints, newest first.
pub const CLAIMS_SHOWN: usize = 16;
/// The deepest page one read may request, keeping top-K heap RAM bounded (`<= 256` claims).
const MAX_CLAIM_PAGES: usize = 16;
/// One page of `scan_action_state` (`mush_core::action_state::MAX_STATE_FAMILY_SCAN_RECORDS`).
const SCAN_PAGE: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ReadQuery {
    author: Option<ObjId>,
    page: usize,
}

fn parse_read_query(raw: &str) -> Option<ReadQuery> {
    let mut author = None;
    let mut page = None;
    let mut count = 0usize;
    for token in raw.split_whitespace() {
        count += 1;
        if count > 2 {
            return None;
        }
        if let Some(rest) = token
            .strip_prefix("page=")
            .or_else(|| token.strip_prefix("PAGE="))
        {
            if page.is_some() {
                return None;
            }
            let n = crate::port::families::strict_decimal_u32(rest)
                .and_then(|v| usize::try_from(v).ok())
                .filter(|v| (1..=MAX_CLAIM_PAGES).contains(v))?;
            page = Some(n);
            continue;
        }
        let author_raw = token
            .strip_prefix("author=")
            .or_else(|| token.strip_prefix("AUTHOR="))
            .unwrap_or(token);
        if let Some(num) = author_raw.strip_prefix('#') {
            if author.is_some() {
                return None;
            }
            let id = crate::port::families::strict_decimal_u32(num).filter(|n| *n != 0)?;
            author = Some(ObjId(id));
            continue;
        }
        return None;
    }
    Some(ReadQuery {
        author,
        page: page.unwrap_or(1),
    })
}

type ClaimKey = (i64, ObjId, u64);

struct RankedClaim {
    key: ClaimKey,
    claim: ClaimRecord,
}

impl PartialEq for RankedClaim {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl Eq for RankedClaim {}

impl PartialOrd for RankedClaim {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for RankedClaim {
    fn cmp(&self, other: &Self) -> Ordering {
        self.key.cmp(&other.key)
    }
}

/// Bounded top-K recency accumulator: holds at most `limit` newest claims in RAM while
/// scanning arbitrarily many authors without prefix truncation.
struct TopClaims {
    limit: usize,
    total: usize,
    heap: BinaryHeap<Reverse<RankedClaim>>,
}

impl TopClaims {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            total: 0,
            heap: BinaryHeap::with_capacity(limit.saturating_add(1)),
        }
    }

    fn push(&mut self, claim: &ClaimRecord) {
        self.total = self.total.saturating_add(1);
        if self.limit == 0 {
            return;
        }
        let key = (claim.at_ic.at, claim.author, claim.seq);
        if self.heap.len() < self.limit {
            self.heap.push(Reverse(RankedClaim {
                key,
                claim: claim.clone(),
            }));
        } else if self.heap.peek().is_some_and(|oldest| key > oldest.0.key) {
            let _ = self.heap.pop();
            self.heap.push(Reverse(RankedClaim {
                key,
                claim: claim.clone(),
            }));
        }
    }

    fn into_page(self, page: usize) -> (Vec<ClaimRecord>, usize) {
        let mut ranked: Vec<RankedClaim> = self
            .heap
            .into_vec()
            .into_iter()
            .map(|Reverse(item)| item)
            .collect();
        ranked.sort_by_key(|item| Reverse(item.key));
        let skip = page.saturating_sub(1).saturating_mul(CLAIMS_SHOWN);
        let shown = ranked
            .into_iter()
            .skip(skip)
            .take(CLAIMS_SHOWN)
            .map(|item| item.claim)
            .collect();
        (shown, self.total)
    }
}

/// **`@registry/claims [#<author>|author=#<author>] [page=<1..=16>]`**: the newest
/// [`CLAIMS_SHOWN`] claims filed with the Registry, for an operator authorized now.
/// Scans all authors into a bounded top-K heap (never prefix-truncating low `ObjId`s
/// before recency sorting) and supports direct O(32) single-author lookups.
pub(crate) fn read(ctx: &mut Ctx<'_>, usage: &str) -> Result<(), CmdError> {
    let query = parse_read_query(ctx.args.raw().trim()).ok_or_else(|| CmdError::msg(usage))?;
    let actor = ctx.actor;
    let writer = InspectionHost::of(ctx.w)
        .ok_or_else(|| CmdError::msg(words::INSPECTION_OFF))?
        .writer
        .clone();
    if !writer.claims_offered(ctx.w, actor) {
        return Err(CmdError::msg(words::INSPECT_REFUSED));
    }
    let family = writer
        .claims_family()
        .map_err(|_| CmdError::msg(words::INSPECTION_OFF))?;
    let mut top = TopClaims::new(query.page.saturating_mul(CLAIMS_SHOWN));
    if let Some(author) = query.author {
        for slot in mush_roll::commerce::registry_inspection::claims::claim_slots(author) {
            let maybe = ctx
                .action_state(&family, &slot)
                .map_err(|_| CmdError::msg(words::INSPECTION_OFF))?;
            if let Some(claim) = maybe {
                top.push(claim);
            }
        }
    } else {
        let mut after: Option<String> = None;
        loop {
            let page = ctx
                .scan_action_state(&family, after.as_deref(), SCAN_PAGE)
                .map_err(|_| CmdError::msg(words::INSPECTION_OFF))?;
            for entry in page.entries() {
                top.push(entry.value());
            }
            let next = page.next_after().map(|cursor| cursor.to_string());
            if next.is_none() || next.as_deref() == after.as_deref() {
                break;
            }
            after = next;
        }
    }
    let (shown, total) = top.into_page(query.page);
    if shown.is_empty() {
        ctx.out.info(actor, words::NO_CLAIMS);
        return Ok(());
    }
    let mut lines = Vec::new();
    for claim in &shown {
        lines.extend(claim_lines(ctx.w, claim));
    }
    let shown_end = query
        .page
        .saturating_sub(1)
        .saturating_mul(CLAIMS_SHOWN)
        .saturating_add(shown.len());
    if total > shown_end {
        lines.push(fill(
            words::MORE_CLAIMS,
            &[("count", &(total - shown_end).to_string())],
        ));
    }
    for line in lines {
        ctx.out.info(actor, line);
    }
    Ok(())
}

/// One claim as the operator reads it: who, which of their claims, when and which hulls, then
/// the author's words as theirs, each line inside the 120-column floor.
pub(crate) fn claim_lines(w: &mush_core::World, claim: &ClaimRecord) -> Vec<String> {
    let name = w
        .get(claim.author)
        .map_or_else(|| "someone".to_owned(), |o| o.name.to_string());
    let hulls = claim
        .hulls
        .iter()
        .map(|hull: &ObjId| format!("#{}", hull.0))
        .collect::<Vec<_>>()
        .join(", ");
    let mut lines = crate::port::site_map::wrap(&fill(
        words::CLAIM_LINE,
        &[
            ("author", &name),
            ("number", &claim.author.0.to_string()),
            ("seq", &claim.seq.to_string()),
            ("at", &crate::port::licence::ic_stamp(claim.at_ic.at)),
            ("hulls", &hulls),
        ],
    ));
    lines.extend(crate::port::site_map::wrap(&fill(
        words::CLAIM_WORDS,
        &[("text", claim.text.as_str())],
    )));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::{ClockRef, ClockStamp};
    use mush_roll::commerce::registry_inspection::RegistryInspectionPolicy;
    use smol_str::SmolStr;

    fn sample_policy() -> RegistryInspectionPolicy {
        RegistryInspectionPolicy::new(
            mush_org::SystemPrincipalId::new("registry").expect("issuer"),
            mush_org::OrgId(1),
            mush_org::OrgScopeId(1),
            vec![ObjId(10)],
        )
        .expect("policy")
    }

    fn sample_claim(
        policy: &RegistryInspectionPolicy,
        author: ObjId,
        seq: u64,
        at: i64,
    ) -> ClaimRecord {
        ClaimRecord {
            format: 1,
            claim: mush_core::CommitToken::from_bytes([seq as u8; 16]),
            action: policy.claim_action().expect("claim action"),
            authority: policy.clone(),
            author,
            seq,
            hulls: vec![ObjId(77)],
            text: SmolStr::new(format!("claim by #{} seq {seq}", author.0)),
            at_ic: ClockStamp {
                clock: ClockRef::WorldIc { generation: 1 },
                at,
            },
        }
    }

    #[test]
    fn p2_tsc_18_top_claims_is_not_blinded_by_1024_low_objid_claims() {
        let policy = sample_policy();
        let mut top = TopClaims::new(CLAIMS_SHOWN);
        // 32 low-ObjId authors * 32 claims = 1,024 older rows (previously filled MAX_READ).
        for author_id in 1..=32u32 {
            for seq in 1..=32u64 {
                let c = sample_claim(&policy, ObjId(author_id), seq, 1_000 + seq as i64);
                top.push(&c);
                assert!(
                    top.heap.len() <= CLAIMS_SHOWN,
                    "heap RAM must stay bounded at CLAIMS_SHOWN"
                );
            }
        }
        // 33rd higher-ObjId author files a newer claim after row 1,024.
        let newer = sample_claim(&policy, ObjId(999), 1, 9_999);
        top.push(&newer);
        assert!(top.heap.len() <= CLAIMS_SHOWN);

        let (shown, total) = top.into_page(1);
        assert_eq!(total, 1_025);
        assert_eq!(shown.len(), CLAIMS_SHOWN);
        assert_eq!(
            shown[0].author,
            ObjId(999),
            "33rd higher-ObjId author's newest claim must appear first"
        );
        assert_eq!(shown[0].at_ic.at, 9_999);
    }

    #[test]
    fn p2_tsc_18_pagination_and_query_parser_are_bounded() {
        let policy = sample_policy();
        let mut top = TopClaims::new(2 * CLAIMS_SHOWN);
        for i in 1..=40u64 {
            let c = sample_claim(&policy, ObjId(50), i, 100 + i as i64);
            top.push(&c);
        }
        let (page2, total) = top.into_page(2);
        assert_eq!(total, 40);
        assert_eq!(page2.len(), CLAIMS_SHOWN);
        assert_eq!(page2[0].seq, 24);
        assert_eq!(page2[CLAIMS_SHOWN - 1].seq, 9);

        assert_eq!(
            parse_read_query(""),
            Some(ReadQuery {
                author: None,
                page: 1
            })
        );
        assert_eq!(
            parse_read_query("#42"),
            Some(ReadQuery {
                author: Some(ObjId(42)),
                page: 1
            })
        );
        assert_eq!(
            parse_read_query("author=#42 page=2"),
            Some(ReadQuery {
                author: Some(ObjId(42)),
                page: 2
            })
        );
        for bad in [
            "#0",
            "page=0",
            "page=17",
            "page=01",
            "#1 #2",
            "page=1 page=2",
            "unknown",
            "#1 page=2 extra",
        ] {
            assert_eq!(parse_read_query(bad), None, "{bad}");
        }
    }
}
