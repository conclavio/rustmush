use super::*;
use wire::{integer, nonzero_digest};
impl Row {
    pub(super) fn terminal_pair(
        &self,
        view: &View<'_>,
        native: &Value,
    ) -> Result<(), ActionStateError> {
        let t = required(view, &self.key(TERMINAL)?)?;
        shape(
            &t,
            &[
                "format",
                "request",
                "operation",
                "action",
                "protocol",
                "policy",
                "policy_digest",
                "native_history",
                "close",
            ],
        )?;
        let d = field(native, "decision")?;
        shape(
            d,
            &[
                "format",
                "operation",
                "action",
                "actor",
                "at_ic",
                "posted_at_utc",
                "reason",
            ],
        )?;
        let close = field(&t, "close")?;
        shape(
            close,
            &[
                "open",
                "close",
                "digest",
                "transition",
                "at_ic",
                "zone",
                "kind",
            ],
        )?;
        let reason = field(d, "reason")?;
        let cause = field(reason, "kind")?
            .as_str()
            .ok_or_else(|| refused("coverage terminal cause"))?;
        let pays = cause == "destroyed";
        let current = protocol::Captured::parse(field(&t, "protocol")?)?;
        let operation: CommitToken = parsed(field(d, "operation")?)?;
        let actor: ObjId = parsed(field(d, "actor")?)?;
        let terms = self.terms()?;
        let decision: ClockStamp = parsed(field(d, "at_ic")?)?;
        let at: ClockStamp = parsed(field(close, "at_ic")?)?;
        let start: ClockStamp = parsed(field(terms, "starts_at")?)?;
        let end: ClockStamp = parsed(field(terms, "ends_at")?)?;
        let expected_actor: ObjId = parsed(field(
            terms,
            if cause == "cancelled" {
                "buyer"
            } else {
                "adjudicator"
            },
        )?)?;
        if number(&t, "format")? != 1
            || number(d, "format")? != 1
            || actor != expected_actor
            || operation == self.binding
            || field(&t, "operation")? != &json!(operation)
            || field(&t, "policy")? != &json!(self.binding)
            || field(&t, "action")? != field(d, "action")?
            || field(d, "action")? != &json!(current.action(if pays { 1 } else { 2 })?)
            || field(&t, "policy_digest")? != &json!(self.policy_digest()?)
            || field(&t, "native_history")? != &json!(self.history_digest()?)
            || field(close, "open")? != field(self.basis()?, "ic_open")?
            || number(close, "transition")? == 0
            || field(close, "close")? == field(close, "open")?
            || field(close, "close")? == &json!(operation)
            || integer(d, "posted_at_utc")? < integer(self.consent()?, "accepted_at_utc")?
            || !matches!(decision.clock,crate::ClockRef::WorldIc{generation} if generation!=0)
            || !matches!(at.clock,crate::ClockRef::WorldIc{generation} if generation!=0)
            || (decision.clock == at.clock && decision.at < at.at)
            || (decision.clock == start.clock && decision.at < start.at)
            || (decision.clock != start.clock && !["cancelled", "clean_close"].contains(&cause))
        {
            return Err(refused("coverage exact terminal affinity"));
        }
        nonzero_digest(field(&t, "request")?)?;
        nonzero_digest(field(close, "digest")?)?;
        let zone: DefinitionRef = parsed(field(close, "zone")?)?;
        let close_kind = field(field(close, "kind")?, "kind")?
            .as_str()
            .ok_or_else(|| refused("coverage close kind"))?;
        if close_kind == "destroyed" {
            shape(field(close, "kind")?, &["kind"])?;
        } else if [
            "returned_to_base",
            "powered_down_in_base",
            "recovered_to_base",
        ]
        .contains(&close_kind)
        {
            shape(field(close, "kind")?, &["kind", "base"])?;
            let base: ObjId = parsed(field(field(close, "kind")?, "base")?)?;
            if base.0 == 0 || base == self.hull {
                return Err(refused("coverage close base"));
            }
        } else {
            return Err(refused("coverage close kind"));
        }
        let covered = close_kind == "destroyed"
            && at.clock == start.clock
            && at.at >= start.at
            && at.at < end.at
            && &json!(zone) == field(self.basis()?, "zone")?;
        let (event, event_digest) = if pays {
            shape(
                reason,
                &[
                    "kind",
                    "loss",
                    "loss_digest",
                    "occurred_at",
                    "zone",
                    "transition",
                ],
            )?;
            if !covered
                || field(reason, "occurred_at")? != field(close, "at_ic")?
                || field(reason, "zone")? != field(close, "zone")?
                || decision.clock != at.clock
                || at
                    .at
                    .checked_add(integer(terms, "claim_delay_seconds")?)
                    .is_none_or(|due| decision.at < due)
            {
                return Err(refused("coverage destruction window"));
            }
            (field(reason, "loss")?, field(reason, "loss_digest")?)
        } else if cause == "voided" {
            // Doc 17h cut M8d D5: a void releases a covered loss the product's exclusion
            // refused, naming the Registry inspection that recorded the excluded fact.
            shape(
                reason,
                &["kind", "close", "close_digest", "transition", "inspection"],
            )?;
            let inspection: CommitToken = parsed(field(reason, "inspection")?)?;
            if !covered || inspection == operation || &json!(inspection) == field(reason, "close")?
            {
                return Err(refused("coverage void requires a covered loss"));
            }
            (field(reason, "close")?, field(reason, "close_digest")?)
        } else {
            if !["clean_close", "cancelled", "expired_clean"].contains(&cause)
                || covered
                || (cause == "expired_clean" && decision.at < end.at)
            {
                return Err(refused("coverage release cannot erase covered loss"));
            }
            shape(reason, &["kind", "close", "close_digest", "transition"])?;
            (field(reason, "close")?, field(reason, "close_digest")?)
        };
        if event != field(close, "close")?
            || event_digest != field(close, "digest")?
            || field(reason, "transition")? != field(close, "transition")?
        {
            return Err(refused("coverage actual close identity"));
        }
        let receipt = json!({"format":1,"operation":operation,"policy":self.binding,"history":self.history_digest()?,"transaction":field(native,"transaction")?,"journal_seq":field(native,"journal_seq")?,"state":self.state()?});
        if required(view, &self.key(RECEIPT)?)? != receipt {
            return Err(refused("coverage native terminal receipt"));
        }
        Ok(())
    }
}
