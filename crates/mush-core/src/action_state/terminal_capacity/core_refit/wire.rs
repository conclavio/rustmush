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
            || action != action_reference(&policy, field(&offer, "component")?, 0)?
            || field(field(&raw, "funding")?, "commit")? != &json!(accepted)
            || field(field(&raw, "funding")?, "action")? != &json!(action)
            || start.clock != expiry.clock
            || start.at.checked_add(604800) != Some(expiry.at)
        {
            return Err(refused("core refit immutable admission identity"));
        }
        canonical_json_bytes_with_limit(&intent, 34816).map_err(bad)?;
        canonical_json_bytes_with_limit(field(&raw, "funding")?, 16384).map_err(bad)?;
        canonical_json_bytes_with_limit(field(&raw, "progress")?, 32768).map_err(bad)?;
        let fingerprint = digest(
            "core.industry_refit.capacity.v1",
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
        match (row.terminal()?, row.installation()?) {
            (None, None) => {}
            (Some(t), installation) => {
                canonical_json_bytes_with_limit(t, 16384).map_err(bad)?;
                let outcome = field(field(t, "result")?, "outcome")?
                    .as_str()
                    .ok_or_else(|| refused("core refit terminal result"))?;
                let index = match (outcome, installation) {
                    ("installed", Some(i)) => {
                        canonical_json_bytes_with_limit(i, 69632).map_err(bad)?;
                        strict(i, &["replacement", "core_receipt", "at_utc"])?;
                        let replacement = field(i, "replacement")?;
                        if field(field(t, "result")?, "replacement")?
                            != &json!(digest("industry.core_refit.replacement.v1", replacement)?)
                            || field(i, "at_utc")? != field(t, "posted_at")?
                            || field(replacement, "object")? != field(&row.offer, "hull")?
                            || field(replacement, "from_revision")?
                                != field(&row.offer, "unit_revision")?
                            || number(replacement, "from_revision")?.checked_add(1)
                                != Some(number(replacement, "to_revision")?)
                            || field(replacement, "before")? != field(&row.offer, "before")?
                            || field(replacement, "component")? != field(&row.offer, "component")?
                            || field(replacement, "displaced")? != field(&row.offer, "displaced")?
                            || field(
                                field(field(replacement, "inserted")?, "installed")?,
                                "asset",
                            )? != field(&row.offer, "incoming")?
                        {
                            return Err(refused("core refit actual replacement join"));
                        }
                        2
                    }
                    ("cancelled", None) => 3,
                    _ => return Err(refused("core refit terminal pair")),
                };
                if field(t, "commit")? == &json!(row.accepted)
                    || field(t, "action")?
                        != &json!(action_reference(
                            &row.policy,
                            field(&row.offer, "component")?,
                            index
                        )?)
                    || number(t, "journal_seq")?
                        <= number(field(&row.raw, "funding")?, "journal_seq")?
                {
                    return Err(refused("core refit terminal action/sequence"));
                }
            }
            _ => return Err(refused("core refit terminal pair")),
        }
        row.validate_progress()?;
        Ok(row)
    }
    pub(super) fn terminal(&self) -> Result<Option<&Value>, ActionStateError> {
        optional(&self.raw, "terminal")
    }
    pub(super) fn installation(&self) -> Result<Option<&Value>, ActionStateError> {
        optional(&self.raw, "installation")
    }
    pub(super) fn requester(&self) -> Result<ObjId, ActionStateError> {
        parsed(field(&self.offer, "requester")?)
    }
    pub(super) fn incoming(&self) -> Result<ObjId, ActionStateError> {
        parsed(field(&self.offer, "incoming")?)
    }
    pub(super) fn hull(&self) -> Result<ObjId, ActionStateError> {
        parsed(field(&self.offer, "hull")?)
    }
    pub(super) fn displaced(&self) -> Result<Option<ObjId>, ActionStateError> {
        parsed(
            field(field(&self.offer, "displaced")?, "installed")?
                .get("asset")
                .unwrap_or(&Value::Null),
        )
    }
    pub(super) fn resource(&self) -> Result<ResourceRef, ActionStateError> {
        let factory: ObjId = parsed(field(&self.policy, "factory")?)?;
        let owner = owner_id()?;
        Ok(ResourceRef::new(
            owner.clone(),
            SubjectKey::Scoped {
                owner,
                class: CanonicalId::new("industry.facility").map_err(bad)?,
                id: IdempotencyKeyText::parse_canonical(format!("f{}", factory.0)).map_err(bad)?,
            },
            CanonicalId::new("slots").map_err(bad)?,
        ))
    }
    pub(super) fn holder(&self) -> Result<crate::ScopedIdempotencyKey, ActionStateError> {
        let owner = owner_id()?;
        let id = crate::IdempotencyKeyText::parse_canonical(format!("wo{:016x}", self.order))
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
                    "industry.wo_holder.v1",
                    1,
                    &json!({"order":self.order}),
                    1024,
                )
                .map_err(bad)?,
            ),
        ))
    }

    fn validate_progress(&self) -> Result<(), ActionStateError> {
        let p = field(&self.raw, "progress")?;
        strict(p, &["order", "production", "cursor", "advances"])?;
        let total = number(field(&self.offer, "recipe")?, "work_units")?;
        if !(1..=167).contains(&total) || number(p, "order")? != self.order {
            return Err(refused("core refit production identity"));
        }
        let start: ClockStamp = parsed(field(&self.intent, "accepted_at")?)?;
        let expiry: ClockStamp = parsed(field(&self.intent, "expires_at")?)?;
        let id = ScheduleId::derive(
            &owner_id()?,
            &CanonicalId::new(format!("refit.{:016x}", self.order)).map_err(bad)?,
        )
        .map_err(bad)?;
        let mut cursor = crate::DurableSchedule::new(crate::DurableScheduleDraft {
            id,
            owner: owner_id()?,
            clock: start.clock.clone(),
            next_due: ClockStamp {
                clock: start.clock.clone(),
                at: start
                    .at
                    .checked_add(3600)
                    .ok_or_else(|| refused("core refit clock overflow"))?,
            },
            cadence: Some(3600),
            generation: 1,
            next_ordinal: 0,
            restart: crate::ScheduleRestartPolicy::Resume,
            catch_up_order: crate::ScheduleCatchUpOrder::OldestFirst,
            max_catch_up: 256,
        })
        .map_err(bad)?;
        let proofs = field(p, "advances")?
            .as_array()
            .ok_or_else(|| refused("core refit occurrence history"))?;
        if proofs.len() > 167 {
            return Err(refused("core refit progress bound"));
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
                return Err(refused("core refit actual progress clock/identity"));
            }
            let cause = match field(proof, "cause")?.as_str() {
                Some("live") => crate::ScheduleDriveCause::Live,
                Some("restart") => crate::ScheduleDriveCause::Restart,
                _ => return Err(refused("core refit drive cause")),
            };
            let crate::ScheduleEvaluation::Advance(planned) = cursor
                .plan_due(cursor.revision(), &stamp, cause)
                .map_err(bad)?
            else {
                return Err(refused("core refit occurrence not due"));
            };
            if planned.occurrences().is_empty()
                || planned.skipped().is_some()
                || number(proof, "occurrences")? != planned.occurrences().len() as u64
            {
                return Err(refused("core refit retained occurrence differs"));
            }
            units = units
                .checked_add(planned.occurrences().len() as u64)
                .ok_or_else(|| refused("core refit progress overflow"))?
                .min(total);
            last = stamp.at;
            cursor = planned.post_image().clone();
        }
        if field(p, "cursor")? != &json!(cursor)
            || field(p, "production")?
                != &json!({"schedule":id,"started":start,"clock_policy":"core.refit","total_units":total,"progress_units":units})
            || (self.installation()?.is_some() && units != total)
        {
            return Err(refused("core refit complete native progress"));
        }
        Ok(())
    }

    pub(super) fn keys(&self) -> Result<Vec<(&'static str, Key)>, ActionStateError> {
        let capacity = ReservationBookId::derive(&self.resource()?)
            .map_err(bad)?
            .canonical_hash()
            .to_hex();
        let schedule = ScheduleId::derive(
            &owner_id()?,
            &CanonicalId::new(format!("refit.{:016x}", self.order)).map_err(bad)?,
        )
        .map_err(bad)?
        .canonical_hash()
        .to_hex();
        let mut keys = vec![
            (ORDER, order_key(self.order)?),
            (ASSET, key(ASSET, self.incoming()?.0.to_string())?),
            (HULL, key(HULL, self.hull()?.0.to_string())?),
            (EXCHANGE, key(EXCHANGE, format!("{:016x}", self.order))?),
            (SLOTS, key(SLOTS, capacity.clone())?),
            (BOOK, key(BOOK, capacity)?),
            (SCHEDULE, key(SCHEDULE, schedule.clone())?),
            (JOB, key(JOB, schedule)?),
        ];
        if let Some(old) = self.displaced()? {
            keys.push((ASSET, key(ASSET, old.0.to_string())?));
        }
        Ok(keys)
    }
    pub(super) fn key(&self, family: &str) -> Result<Key, ActionStateError> {
        self.keys()?
            .into_iter()
            .find_map(|(f, k)| (f == family).then_some(k))
            .ok_or_else(|| refused("core refit target key"))
    }
}
