use super::*;
use protocol::Captured;
pub(super) struct Row {
    pub(super) raw: Value,
    pub(super) binding: CommitToken,
    pub(super) action: DefinitionRef,
    pub(super) hull: ObjId,
    pub(super) sequence: u64,
    pub(super) previous: Option<CommitToken>,
}
pub(super) fn without(value: &Value, names: &[&str]) -> Result<Value, ActionStateError> {
    let mut o = value
        .as_object()
        .cloned()
        .ok_or_else(|| refused("coverage object"))?;
    for n in names {
        o.remove(*n);
    }
    Ok(Value::Object(o))
}
pub(super) fn integer(v: &Value, n: &str) -> Result<i64, ActionStateError> {
    field(v, n)?
        .as_i64()
        .ok_or_else(|| refused("coverage integer"))
}
pub(super) fn nonzero_digest(v: &Value) -> Result<DefinitionDigest, ActionStateError> {
    let d: DefinitionDigest = parsed(v)?;
    if d.as_bytes() == &[0; 32] {
        return Err(refused("coverage empty digest"));
    }
    Ok(d)
}
impl Row {
    pub(super) fn parse(raw: Value, binding: CommitToken) -> Result<Self, ActionStateError> {
        shape(
            &raw,
            &[
                "format",
                "request",
                "operation",
                "action",
                "protocol",
                "sequence",
                "previous",
                "quote",
                "quote_digest",
                "history",
            ],
        )?;
        let action = parsed(field(&raw, "action")?)?;
        let p = Captured::parse(field(&raw, "protocol")?)?;
        let sequence = number(&raw, "sequence")?;
        let previous: Option<CommitToken> = parsed(field(&raw, "previous")?)?;
        let history = field(&raw, "history")?;
        shape(history, &["intent", "binding", "terminal"])?;
        let intent = field(history, "intent")?;
        shape(intent, &["consent", "contract", "hold", "title"])?;
        let c = field(intent, "consent")?;
        shape(
            c,
            &[
                "format",
                "operation",
                "effect",
                "accepted_at_utc",
                "basis",
                "terms",
            ],
        )?;
        let basis = field(c, "basis")?;
        shape(
            basis,
            &[
                "format",
                "product",
                "product_digest",
                "quote",
                "quote_digest",
                "ic_open",
                "seal_digest",
                "admission_digest",
                "departure_digest",
                "hull",
                "hull_title_revision",
                "hull_revision",
                "hull_schema",
                "zone",
                "qualified_role",
            ],
        )?;
        let hull: ObjId = parsed(field(basis, "hull")?)?;
        let b = field(history, "binding")?;
        shape(
            b,
            &["transaction", "journal_seq", "plan_hash", "hold", "title"],
        )?;
        if number(&raw, "format")? != 1
            || sequence == 0
            || (sequence == 1) != previous.is_none()
            || previous == Some(binding)
            || parsed::<CommitToken>(field(&raw, "operation")?)? != binding
            || action != p.action(0)?
            || number(c, "format")? != 1
            || number(basis, "format")? != 1
            || field(c, "operation")? != &json!(binding)
            || field(c, "effect")? != &json!(action)
            || field(&raw, "quote")? != field(basis, "quote")?
            || field(&raw, "quote_digest")? != field(basis, "quote_digest")?
            || hull.0 == 0
            || number(basis, "hull_title_revision")? == 0
            || field(intent, "hold")? != field(field(b, "hold")?, "id")?
        {
            return Err(refused("coverage immutable native affinity"));
        }
        nonzero_digest(field(&raw, "request")?)?;
        for n in [
            "product_digest",
            "quote_digest",
            "seal_digest",
            "admission_digest",
            "departure_digest",
        ] {
            nonzero_digest(field(basis, n)?)?;
        }
        let _: DefinitionRef = parsed(field(basis, "hull_schema")?)?;
        let _: DefinitionRef = parsed(field(basis, "zone")?)?;
        let _: u64 = number(basis, "hull_revision")?;
        let _: [u8; 32] = parsed(field(b, "plan_hash")?)?;
        let result = Self {
            raw,
            binding,
            action,
            hull,
            sequence,
            previous,
        };
        result.terms_check()?;
        if let Some(t) = result.terminal()? {
            shape(
                t,
                &[
                    "decision",
                    "before_title",
                    "transaction",
                    "journal_seq",
                    "plan_hash",
                    "hold",
                    "title",
                ],
            )?;
            if field(t, "before_title")? != field(result.native_binding()?, "title")? {
                return Err(refused("coverage terminal changed bound title"));
            }
        }
        digest("economy.coverage.history.v1", result.history()?, 65536)?;
        result.policy_digest()?;
        Ok(result)
    }
    pub(super) fn history(&self) -> Result<&Value, ActionStateError> {
        field(&self.raw, "history")
    }
    pub(super) fn intent(&self) -> Result<&Value, ActionStateError> {
        field(self.history()?, "intent")
    }
    pub(super) fn consent(&self) -> Result<&Value, ActionStateError> {
        field(self.intent()?, "consent")
    }
    pub(super) fn terms(&self) -> Result<&Value, ActionStateError> {
        field(self.consent()?, "terms")
    }
    pub(super) fn basis(&self) -> Result<&Value, ActionStateError> {
        field(self.consent()?, "basis")
    }
    pub(super) fn native_binding(&self) -> Result<&Value, ActionStateError> {
        field(self.history()?, "binding")
    }
    pub(super) fn terminal(&self) -> Result<Option<&Value>, ActionStateError> {
        Ok(match field(self.history()?, "terminal")? {
            Value::Null => None,
            v => Some(v),
        })
    }
    pub(super) fn decision(&self) -> Result<Option<&Value>, ActionStateError> {
        self.terminal()?.map(|t| field(t, "decision")).transpose()
    }
    pub(super) fn history_digest(&self) -> Result<DefinitionDigest, ActionStateError> {
        digest("economy.coverage.history.v1", self.history()?, 65536)
    }
    pub(super) fn policy_digest(&self) -> Result<DefinitionDigest, ActionStateError> {
        digest("tacsim.coverage.policy.v1", &self.raw, 81920)
    }
    pub(super) fn key(&self, f: &str) -> Result<Key, ActionStateError> {
        key(
            f,
            if f == HULL {
                format!("{:08x}", self.hull.0)
            } else {
                self.binding.to_hex()
            },
        )
    }
    pub(super) fn immutable(&self) -> Result<Value, ActionStateError> {
        let mut v = self.raw.clone();
        v.get_mut("history")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| refused("coverage history"))?
            .insert("terminal".into(), Value::Null);
        Ok(v)
    }
    pub(super) fn state(&self) -> Result<Value, ActionStateError> {
        Ok(match self.decision()? {
            None => json!({"kind":"held"}),
            Some(d) => {
                json!({"kind":if field(field(d,"reason")?,"kind")?=="destroyed"{"paid"}else{"released"},"terminal":field(d,"operation")?})
            }
        })
    }
    pub(super) fn exposure(&self) -> Result<Value, ActionStateError> {
        let t = self.terms()?;
        Ok(
            json!({"format":1,"policy":self.binding,"hull":self.hull,"treasury":field(t,"treasury")?,"instrument":field(t,"instrument")?,"liability":field(t,"liability")?,"history":self.history_digest()?,"state":self.state()?}),
        )
    }
    pub(super) fn head(&self) -> Result<Value, ActionStateError> {
        Ok(
            json!({"format":1,"hull":self.hull,"sequence":self.sequence,"latest":self.binding,"policy_digest":self.policy_digest()?,"active":self.terminal()?.is_none().then_some(self.binding)}),
        )
    }
    fn terms_check(&self) -> Result<(), ActionStateError> {
        let t = self.terms()?;
        shape(
            t,
            &[
                "format",
                "buyer",
                "insurer",
                "scope",
                "treasury",
                "payer",
                "instrument",
                "adjudicator",
                "insured_value",
                "base_premium",
                "discount",
                "premium",
                "liability",
                "starts_at",
                "ends_at",
                "claim_delay_seconds",
                "quoted_at_utc",
                "expires_at_utc",
            ],
        )?;
        let start: ClockStamp = parsed(field(t, "starts_at")?)?;
        let end: ClockStamp = parsed(field(t, "ends_at")?)?;
        let buyer: ObjId = parsed(field(t, "buyer")?)?;
        let adjudicator: ObjId = parsed(field(t, "adjudicator")?)?;
        let utc = integer(self.consent()?, "accepted_at_utc")?;
        if number(t, "format")? != 1
            || buyer.0 == 0
            || adjudicator.0 == 0
            || buyer == self.hull
            || !matches!(start.clock,crate::ClockRef::WorldIc{generation} if generation!=0)
            || start.clock != end.clock
            || end
                .at
                .checked_sub(start.at)
                .is_none_or(|n| !(1..=7_776_000).contains(&n))
            || !(0..=2_592_000).contains(&integer(t, "claim_delay_seconds")?)
            || utc < integer(t, "quoted_at_utc")?
            || utc >= integer(t, "expires_at_utc")?
        {
            return Err(refused("coverage accepted window"));
        }
        Ok(())
    }
    pub(super) fn sources(&self, view: &View<'_>) -> Result<(Value, Value), ActionStateError> {
        let qid: CommitToken = parsed(field(&self.raw, "quote")?)?;
        let q = required(view, &key(QUOTE, qid.to_hex())?)?;
        shape(
            &q,
            &[
                "format",
                "request",
                "operation",
                "action",
                "protocol",
                "publication",
                "publication_digest",
                "product_digest",
                "risk",
                "terms",
            ],
        )?;
        let pubid: CommitToken = parsed(field(&q, "publication")?)?;
        let publication = required(view, &key(PRODUCT, pubid.to_hex())?)?;
        // A qualified product's publication also carries its approved Conduct
        // terms; a qualified quote must come from one, an ordinary quote not.
        let qualified_quote = !field(field(&q, "risk")?, "qualified")?.is_null();
        const PUBLICATION: [&str; 8] = [
            "format",
            "operation",
            "action",
            "protocol",
            "publisher",
            "authorization",
            "product",
            "posted_at_utc",
        ];
        if qualified_quote {
            let mut names = PUBLICATION.to_vec();
            names.push("conduct");
            shape(&publication, &names)?;
        } else {
            shape(&publication, &PUBLICATION)?;
        }
        let risk = field(&q, "risk")?;
        shape(
            risk,
            &[
                "ic_open",
                "seal_digest",
                "admission_digest",
                "departure_digest",
                "hull",
                "hull_title_revision",
                "hull_revision",
                "hull_schema",
                "zone",
                "qualified",
            ],
        )?;
        let mut basis = without(risk, &["qualified"])?;
        let o = basis
            .as_object_mut()
            .ok_or_else(|| refused("coverage risk"))?;
        // A qualified quote binds the digest of its Conduct selection, exactly as
        // tacsim's `CoverageBasis` records it (`ConductSelection::digest`).
        let qualified = field(risk, "qualified")?;
        let role = if qualified.is_null() {
            Value::Null
        } else {
            json!(digest("tacsim.coverage.qualified_role.v1", qualified, 4096)?)
        };
        for (n, v) in [
            ("format", json!(1)),
            ("product", json!(pubid)),
            (
                "product_digest",
                json!(digest(
                    "tacsim.coverage.publication.v1",
                    &publication,
                    98304
                )?),
            ),
            ("quote", json!(qid)),
            (
                "quote_digest",
                json!(digest("tacsim.coverage.quote.v1", &q, 16384)?),
            ),
            ("qualified_role", role),
        ] {
            o.insert(n.into(), v);
        }
        let qt = field(&q, "terms")?;
        let mut terms = without(qt, &["duration_seconds"])?;
        let start: ClockStamp = parsed(field(self.terms()?, "starts_at")?)?;
        let end = ClockStamp {
            clock: start.clock.clone(),
            at: start
                .at
                .checked_add(integer(qt, "duration_seconds")?)
                .ok_or_else(|| refused("coverage duration overflow"))?,
        };
        let o = terms
            .as_object_mut()
            .ok_or_else(|| refused("coverage quote terms"))?;
        o.insert("starts_at".into(), json!(start));
        o.insert("ends_at".into(), json!(end));
        if number(&q, "format")? != 1
            || number(&publication, "format")? != 1
            || field(&q, "request")? == field(&self.raw, "request")?
            || field(&q, "operation")? != &json!(qid)
            || field(&publication, "operation")? != &json!(pubid)
            || field(&q, "protocol")? != field(&self.raw, "protocol")?
            || field(&publication, "protocol")? != field(&q, "protocol")?
            || field(&q, "publication_digest")? != field(&basis, "product_digest")?
            || field(&self.raw, "quote_digest")? != field(&basis, "quote_digest")?
            || self.basis()? != &basis
            || self.terms()? != &terms
        {
            return Err(refused("coverage frozen quote/publication affinity"));
        }
        Ok((q, publication))
    }
    pub(super) fn retention(&self, view: &View<'_>) -> Result<Value, ActionStateError> {
        let r = required(view, &self.key(RETENTION)?)?;
        shape(
            &r,
            &[
                "format",
                "consent",
                "revision",
                "rating",
                "claim_digest",
                "contract",
                "hold",
                "binding_digest",
                "history_digest",
                "terminal",
            ],
        )?;
        if number(&r, "format")? != 1
            || field(&r, "consent")? != self.consent()?
            || field(&r, "contract")? != field(self.intent()?, "contract")?
            || field(&r, "hold")? != field(field(self.native_binding()?, "hold")?, "id")?
            || field(&r, "binding_digest")?
                != &json!(digest(
                    "roll.coverage.native_binding.v1",
                    self.native_binding()?,
                    65536
                )?)
            || field(&r, "history_digest")? != &json!(self.history_digest()?)
            || field(&r, "terminal")? != self.decision()?.unwrap_or(&Value::Null)
        {
            return Err(refused("coverage native/Roll retention affinity"));
        }
        nonzero_digest(field(&r, "claim_digest")?)?;
        Ok(r)
    }
    pub(super) fn fingerprint(
        &self,
        view: &View<'_>,
    ) -> Result<DefinitionDigest, ActionStateError> {
        let (q, p) = self.sources(view)?;
        let retention = without(&self.retention(view)?, &["history_digest", "terminal"])?;
        digest(
            "core.coverage.completion.v1",
            &json!({"policy":self.immutable()?,"retention":retention,"quote":q,"publication":p}),
            262144,
        )
    }
    pub(super) fn paired(&self, view: &View<'_>) -> Result<(), ActionStateError> {
        self.sources(view)?;
        self.retention(view)?;
        if required(view, &self.key(EXPOSURE)?)? != self.exposure()? {
            return Err(refused("coverage exposure affinity"));
        }
        let h = required(view, &self.key(HULL)?)?;
        // A later binding may replace the head after this policy terminates.
        if field(&h, "latest")? == &json!(self.binding) {
            if h != self.head()? {
                return Err(refused("coverage latest hull head"));
            }
        } else if self.terminal()?.is_none() {
            return Err(refused("coverage active policy lost head"));
        }
        match self.terminal()? {
            None => {
                if get(view, &self.key(TERMINAL)?)?.is_some()
                    || get(view, &self.key(RECEIPT)?)?.is_some()
                {
                    return Err(refused("coverage premature terminal"));
                }
            }
            Some(t) => self.terminal_pair(view, t)?,
        }
        Ok(())
    }
    pub(super) fn future_room(&self, view: &View<'_>) -> Result<(), ActionStateError> {
        let r = self.retention(view)?;
        for (o, f, ceiling) in FAMILIES {
            let d = protocol::descriptor(o, f, ceiling)?;
            let value = match f {
                POLICY => self.raw.clone(),
                RETENTION => r.clone(),
                EXPOSURE => self.exposure()?,
                HULL => self.head()?,
                _ => get(view, &self.key(f)?)?.unwrap_or(Value::Null),
            };
            let envelope = json!({"family":{"owner":o,"id":f},"key":self.key(f)?.encoded,"schema":{"id":d.value_schema(),"version":1},"value":value});
            let actual = canonical_json_bytes_with_limit(&envelope, ceiling)
                .map_err(bad)?
                .len();
            let reserve = match f {
                POLICY => 65536usize
                    .checked_sub(
                        canonical_json_bytes_with_limit(self.history()?, 65536)
                            .map_err(bad)?
                            .len(),
                    )
                    .ok_or_else(|| refused("coverage native history size"))?,
                RETENTION => 4096usize
                    .checked_sub(
                        canonical_json_bytes_with_limit(field(&r, "terminal")?, 4096)
                            .map_err(bad)?
                            .len(),
                    )
                    .ok_or_else(|| refused("coverage decision size"))?,
                _ => 0,
            };
            if actual.checked_add(reserve).is_none_or(|n| n > ceiling) {
                return Err(refused("coverage future complete envelope"));
            }
        }
        Ok(())
    }
}
