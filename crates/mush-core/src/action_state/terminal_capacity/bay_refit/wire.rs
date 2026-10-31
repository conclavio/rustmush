//! The bay order row as the kernel reads it. Industry owns the full record;
//! these are the fields the protocol joins.
use super::*;
pub(super) struct Row {
    pub(super) raw: Value,
    pub(super) order: u64,
    pub(super) accepted: CommitToken,
    pub(super) action: DefinitionRef,
    pub(super) intent: Value,
    pub(super) offer: Value,
    pub(super) policy: Value,
    pub(super) fingerprint: DefinitionDigest,
}
/// The row's outcome once terminal: an installed refit or a cancellation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Outcome {
    Installed,
    Cancelled,
}
impl Row {
    pub(super) fn parse(raw: Value, order: u64) -> Result<Self, ActionStateError> {
        strict(
            &raw,
            &[
                "format",
                "sequence",
                "accepted",
                "action",
                "intent",
                "funding",
                "capacity",
                "progress",
                "terminal",
                "installation",
            ],
        )?;
        let intent = field(&raw, "intent")?.clone();
        strict(
            &intent,
            &["order", "offer", "payer", "accepted_at", "expires_at"],
        )?;
        let offer = field(&intent, "offer")?.clone();
        let policy = field(&offer, "policy")?.clone();
        let accepted: CommitToken = parsed(field(&raw, "accepted")?)?;
        let action: DefinitionRef = parsed(field(&raw, "action")?)?;
        let start: ClockStamp = parsed(field(&intent, "accepted_at")?)?;
        let expiry: ClockStamp = parsed(field(&intent, "expires_at")?)?;
        if number(&raw, "format")? != 1
            || !(1..=8192).contains(&number(&raw, "sequence")?)
            || number(&intent, "order")? != order
            || action != action_reference(&policy, 0)?
            || field(field(&raw, "funding")?, "commit")? != &json!(accepted)
            || field(field(&raw, "funding")?, "action")? != &json!(action)
            || !matches!(start.clock, crate::ClockRef::WorldIc { generation } if generation > 0)
            || start.clock != expiry.clock
            || start.at.checked_add(RESERVATION_SECONDS) != Some(expiry.at)
        {
            return Err(refused("bay refit immutable admission identity"));
        }
        canonical_json_bytes_with_limit(&intent, 34816).map_err(bad)?;
        canonical_json_bytes_with_limit(field(&raw, "funding")?, 16384).map_err(bad)?;
        canonical_json_bytes_with_limit(field(&raw, "progress")?, 32768).map_err(bad)?;
        let fingerprint = digest(
            "industry.bay_refit.capacity.v1",
            &json!({"format":1,"sequence":field(&raw,"sequence")?,"accepted":accepted,"action":action,"intent":intent,"funding":field(&raw,"funding")?,"capacity":field(&raw,"capacity")?}),
        )?;
        let row = Self {
            raw,
            order,
            accepted,
            action,
            intent,
            offer,
            policy,
            fingerprint,
        };
        row.validate_offer()?;
        match (row.terminal()?, row.installation()?) {
            (None, None) => {}
            (Some(t), installation) => {
                canonical_json_bytes_with_limit(t, 16384).map_err(bad)?;
                let index = match (row.outcome()?, installation) {
                    (Some(Outcome::Installed), Some(i)) => {
                        row.validate_installation(t, i)?;
                        2
                    }
                    (Some(Outcome::Cancelled), None) => 3,
                    _ => return Err(refused("bay refit terminal pair")),
                };
                if field(t, "commit")? == &json!(row.accepted)
                    || field(t, "action")? != &json!(action_reference(&row.policy, index)?)
                    || number(t, "journal_seq")?
                        <= number(field(&row.raw, "funding")?, "journal_seq")?
                {
                    return Err(refused("bay refit terminal action/sequence"));
                }
            }
            _ => return Err(refused("bay refit terminal pair")),
        }
        row.validate_progress()?;
        Ok(row)
    }
    /// What comes out and what goes in agree with the parts Inventory holds
    /// and takes back, and the bay is one the policy admits.
    fn validate_offer(&self) -> Result<(), ActionStateError> {
        let remove = optional(&self.offer, "remove")?;
        let install = optional(&self.offer, "install")?;
        let part = self.part()?;
        let returned = self.returned()?;
        if (remove.is_none() && install.is_none())
            || part.is_some() != install.is_some()
            || returned.is_some() != remove.is_some()
            || part.is_some_and(|p| returned == Some(p))
            || !protocol::facilities(&self.policy)?.contains(&self.facility()?)
            || [self.requester()?, self.facility()?].contains(&self.hull()?)
            || number(&self.offer, "unit_revision")? == u64::MAX
        {
            return Err(refused("bay refit offer parts"));
        }
        self.stores()?;
        Ok(())
    }
    /// The installed refit joins the offer exactly: the hull, its revision,
    /// the roots before and after, and what came out and went in.
    fn validate_installation(&self, t: &Value, i: &Value) -> Result<(), ActionStateError> {
        canonical_json_bytes_with_limit(i, 69632).map_err(bad)?;
        strict(i, &["refit", "participants", "at_utc"])?;
        let refit = field(i, "refit")?;
        if field(field(t, "result")?, "refit")?
            != &json!(digest("industry.bay_refit.refit.v1", refit)?)
            || field(i, "at_utc")? != field(t, "posted_at")?
            || field(refit, "object")? != field(&self.offer, "hull")?
            || field(refit, "from_revision")? != field(&self.offer, "unit_revision")?
            || number(refit, "from_revision")?.checked_add(1) != Some(number(refit, "to_revision")?)
            || field(refit, "before")? != field(&self.offer, "before")?
            || field(refit, "after")? != field(&self.offer, "target")?
            || field(refit, "remove")? != field(&self.offer, "remove")?
            || field(refit, "install")? != field(&self.offer, "install")?
            || optional(refit, "removed")?.is_some() != self.returned()?.is_some()
            || optional(refit, "inserted")?.is_some() != self.part()?.is_some()
        {
            return Err(refused("bay refit actual refit join"));
        }
        Ok(())
    }
    pub(super) fn terminal(&self) -> Result<Option<&Value>, ActionStateError> {
        optional(&self.raw, "terminal")
    }
    pub(super) fn installation(&self) -> Result<Option<&Value>, ActionStateError> {
        optional(&self.raw, "installation")
    }
    pub(super) fn outcome(&self) -> Result<Option<Outcome>, ActionStateError> {
        let Some(t) = self.terminal()? else {
            return Ok(None);
        };
        match field(field(t, "result")?, "outcome")?.as_str() {
            Some("installed") => Ok(Some(Outcome::Installed)),
            Some("cancelled") => Ok(Some(Outcome::Cancelled)),
            _ => Err(refused("bay refit terminal result")),
        }
    }
    pub(super) fn terminal_commit(&self) -> Result<Value, ActionStateError> {
        Ok(field(
            self.terminal()?
                .ok_or_else(|| refused("bay refit terminal"))?,
            "commit",
        )?
        .clone())
    }
    pub(super) fn requester(&self) -> Result<ObjId, ActionStateError> {
        parsed(field(&self.offer, "requester")?)
    }
    pub(super) fn hull(&self) -> Result<ObjId, ActionStateError> {
        parsed(field(&self.offer, "hull")?)
    }
    pub(super) fn facility(&self) -> Result<ObjId, ActionStateError> {
        parsed(field(&self.offer, "facility")?)
    }
    /// The bay's stores: the Inventory endpoint the part comes from and the
    /// part taken out goes back to.
    pub(super) fn stores(&self) -> Result<u64, ActionStateError> {
        number(&self.offer, "stores")
    }
    /// The lot held for the part fitted, when one is.
    pub(super) fn part(&self) -> Result<Option<u64>, ActionStateError> {
        optional(&self.offer, "part")?.map(parsed).transpose()
    }
    /// The lot the part taken out becomes, when one is.
    pub(super) fn returned(&self) -> Result<Option<u64>, ActionStateError> {
        optional(&self.offer, "returned")?.map(parsed).transpose()
    }
    pub(super) fn resource(&self) -> Result<ResourceRef, ActionStateError> {
        let owner = owner_id("industry")?;
        Ok(ResourceRef::new(
            owner.clone(),
            SubjectKey::Scoped {
                owner,
                class: CanonicalId::new("industry.facility").map_err(bad)?,
                id: IdempotencyKeyText::parse_canonical(format!("f{}", self.facility()?.0))
                    .map_err(bad)?,
            },
            CanonicalId::new("slots").map_err(bad)?,
        ))
    }
    /// The slot's holder: the bay work order, in its own key domain so a
    /// bay order never shares a holder with a core refit's.
    pub(super) fn holder(&self) -> Result<crate::ScopedIdempotencyKey, ActionStateError> {
        let owner = owner_id("industry")?;
        let id = crate::IdempotencyKeyText::parse_canonical(format!("bay{:016x}", self.order))
            .map_err(bad)?;
        Ok(crate::ScopedIdempotencyKey::new(
            crate::IdempotencyScope {
                principal: crate::PrincipalId::Scoped {
                    owner: owner.clone(),
                    class: CanonicalId::new("workorder").map_err(bad)?,
                    id,
                },
                owner,
                namespace: CanonicalId::new("industry").map_err(bad)?,
            },
            0,
            crate::IdempotencyKey::Deterministic(
                crate::CanonicalHash::for_canonical_value(
                    "industry.bay_holder.v1",
                    1,
                    &json!({"order":self.order}),
                    1024,
                )
                .map_err(bad)?,
            ),
        ))
    }
    pub(super) fn schedule(&self) -> Result<ScheduleId, ActionStateError> {
        ScheduleId::derive(
            &owner_id("industry")?,
            &CanonicalId::new(format!("bay.{:016x}", self.order)).map_err(bad)?,
        )
        .map_err(bad)
    }

    fn validate_progress(&self) -> Result<(), ActionStateError> {
        let p = field(&self.raw, "progress")?;
        strict(p, &["order", "production", "cursor", "advances"])?;
        let total = number(field(&self.offer, "recipe")?, "work_units")?;
        if !(1..=MAX_WORK_UNITS).contains(&total) || number(p, "order")? != self.order {
            return Err(refused("bay refit production identity"));
        }
        let start: ClockStamp = parsed(field(&self.intent, "accepted_at")?)?;
        let expiry: ClockStamp = parsed(field(&self.intent, "expires_at")?)?;
        let id = self.schedule()?;
        let mut cursor = crate::DurableSchedule::new(crate::DurableScheduleDraft {
            id,
            owner: owner_id("industry")?,
            clock: start.clock.clone(),
            next_due: ClockStamp {
                clock: start.clock.clone(),
                at: start
                    .at
                    .checked_add(CADENCE)
                    .ok_or_else(|| refused("bay refit clock overflow"))?,
            },
            cadence: Some(CADENCE),
            generation: 1,
            next_ordinal: 0,
            restart: crate::ScheduleRestartPolicy::Resume,
            catch_up_order: crate::ScheduleCatchUpOrder::OldestFirst,
            max_catch_up: 256,
        })
        .map_err(bad)?;
        let proofs = field(p, "advances")?
            .as_array()
            .ok_or_else(|| refused("bay refit occurrence history"))?;
        if proofs.len() as u64 > MAX_WORK_UNITS {
            return Err(refused("bay refit progress bound"));
        }
        let mut units = 0u64;
        let mut commits = BTreeSet::new();
        let mut last = start.at;
        for proof in proofs {
            strict(proof, &["commit", "observed", "cause", "occurrences"])?;
            let commit: CommitToken = parsed(field(proof, "commit")?)?;
            let stamp: ClockStamp = parsed(field(proof, "observed")?)?;
            if units == total
                || !commits.insert(commit)
                || commit == self.accepted
                || stamp.clock != start.clock
                || stamp.at < last
                || stamp.at >= expiry.at
            {
                return Err(refused("bay refit actual progress clock/identity"));
            }
            let cause = cause(proof)?;
            let crate::ScheduleEvaluation::Advance(planned) = cursor
                .plan_due(cursor.revision(), &stamp, cause)
                .map_err(bad)?
            else {
                return Err(refused("bay refit occurrence not due"));
            };
            if planned.occurrences().is_empty()
                || planned.skipped().is_some()
                || number(proof, "occurrences")? != planned.occurrences().len() as u64
            {
                return Err(refused("bay refit retained occurrence differs"));
            }
            units = units
                .checked_add(planned.occurrences().len() as u64)
                .ok_or_else(|| refused("bay refit progress overflow"))?
                .min(total);
            last = stamp.at;
            cursor = planned.post_image().clone();
        }
        if field(p, "cursor")? != &json!(cursor)
            || field(p, "production")?
                != &json!({"schedule":id,"started":start,"clock_policy":"bay.refit","total_units":total,"progress_units":units})
            || (self.installation()?.is_some() && units != total)
        {
            return Err(refused("bay refit complete native progress"));
        }
        Ok(())
    }

    /// Every key the row names, with its target mode: the order, the hull
    /// head, the slot, the schedule, and the Inventory part, its lot, the
    /// returned lot and the stores' census when the refit moves a part.
    pub(super) fn targets(&self) -> Result<Vec<(&'static str, Key, SlotMode)>, ActionStateError> {
        let capacity = ReservationBookId::derive(&self.resource()?)
            .map_err(bad)?
            .canonical_hash()
            .to_hex();
        let schedule = self.schedule()?.canonical_hash().to_hex();
        let update = SlotMode::UpdateExisting;
        let mut keys = vec![
            (ORDER, order_key(self.order)?, update),
            (HULL, key(HULL, self.hull()?.0.to_string())?, update),
            (SLOTS, key(SLOTS, capacity.clone())?, update),
            (BOOK, key(BOOK, capacity)?, update),
            (SCHEDULE, key(SCHEDULE, schedule.clone())?, update),
            (JOB, key(JOB, schedule)?, update),
        ];
        if self.part()?.is_some() {
            keys.push((PART, self.key(PART)?, update));
            keys.push((LOT, self.lot_key(self.part()?)?, update));
        }
        if let Some(lot) = self.returned()? {
            keys.push((LOT, key(LOT, format!("{lot:016x}"))?, SlotMode::Create));
        }
        if self.part()?.is_some() || self.returned()?.is_some() {
            keys.push((CENSUS, self.key(CENSUS)?, update));
        }
        Ok(keys)
    }
    fn lot_key(&self, lot: Option<u64>) -> Result<Key, ActionStateError> {
        let lot = lot.ok_or_else(|| refused("bay refit lot"))?;
        key(LOT, format!("{lot:016x}"))
    }
    /// The one key of `family` the row names (not the lots, of which there
    /// may be two; see [`Row::part_lot`] and [`Row::returned_lot`]).
    pub(super) fn key(&self, family: &str) -> Result<Key, ActionStateError> {
        match family {
            PART => key(PART, format!("{:016x}", self.order)),
            CENSUS => key(CENSUS, format!("v{}", self.stores()?)),
            PART_HEAD => self
                .lot_key(self.part()?)
                .and_then(|k| key(PART_HEAD, k.encoded.into())),
            LOT => Err(refused("bay refit ambiguous lot key")),
            _ => self
                .targets()?
                .into_iter()
                .find_map(|(f, k, _)| (f == family).then_some(k))
                .ok_or_else(|| refused("bay refit target key")),
        }
    }
    pub(super) fn part_lot(&self) -> Result<Option<Key>, ActionStateError> {
        self.part()?
            .map(|l| key(LOT, format!("{l:016x}")))
            .transpose()
    }
    pub(super) fn returned_lot(&self) -> Result<Option<Key>, ActionStateError> {
        self.returned()?
            .map(|l| key(LOT, format!("{l:016x}")))
            .transpose()
    }
}
pub(super) fn cause(proof: &Value) -> Result<crate::ScheduleDriveCause, ActionStateError> {
    match field(proof, "cause")?.as_str() {
        Some("live") => Ok(crate::ScheduleDriveCause::Live),
        Some("restart") => Ok(crate::ScheduleDriveCause::Restart),
        _ => Err(refused("bay refit drive cause")),
    }
}
