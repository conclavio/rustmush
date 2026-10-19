use super::*;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExchangeObjectDigest {
    pub object: ObjId,
    pub before: DefinitionDigest,
    pub after: DefinitionDigest,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExchangeRecord {
    format: u16,
    pub(crate) order: u64,
    operation: CommitToken,
    action: DefinitionRef,
    participants: [DefinitionRef; 2],
    requester: ObjId,
    hull: ObjId,
    unit_revision: u64,
    incoming: ObjId,
    source: ObjId,
    outgoing: Option<ObjId>,
    return_to: Option<ObjId>,
    at_utc: i64,
    objects: Vec<ExchangeObjectDigest>,
}
impl ExchangeRecord {
    pub(super) fn new(
        f: &ExchangeFacts,
        action: DefinitionRef,
        participants: [DefinitionRef; 2],
        operation: CommitToken,
        at_utc: i64,
        objects: Vec<ExchangeObjectDigest>,
    ) -> Self {
        Self {
            format: 1,
            order: f.order,
            operation,
            action,
            participants,
            requester: f.requester,
            hull: f.hull,
            unit_revision: f.unit_revision,
            incoming: f.incoming,
            source: f.source,
            outgoing: f.outgoing,
            return_to: f.return_to,
            at_utc,
            objects,
        }
    }
    pub(crate) fn validate(&self) -> Result<(), ParticipantError> {
        exact_action(&self.action)?;
        check_participants(&self.participants)?;
        let displaced = match (self.outgoing, self.return_to) {
            (Some(o), Some(d)) => Some((o, d)),
            (None, None) => None,
            _ => return Err(refused()),
        };
        ExchangeFacts::new(
            0,
            self.order,
            self.requester,
            self.hull,
            self.unit_revision,
            self.incoming,
            self.source,
            displaced,
        )?;
        let expected: BTreeSet<_> = [
            Some(self.hull),
            Some(self.incoming),
            Some(self.source),
            self.outgoing,
            self.return_to,
        ]
        .into_iter()
        .flatten()
        .collect();
        if self.format != 1
            || self.objects.len() != expected.len()
            || self.objects.len() > MAX_EXCHANGE_OBJECTS
            || self.objects.iter().map(|o| o.object).collect::<Vec<_>>()
                != expected.into_iter().collect::<Vec<_>>()
        {
            return Err(refused());
        }
        bounded(self)?;
        Ok(())
    }
}
/// Opaque result from this exact preparation, or its verified permanent record.
/// Public facts do not authorize an install without the actual registered owners.
#[derive(Debug)]
pub struct ExchangedCoreComponent {
    pub(crate) row: ExchangeRecord,
}
impl ExchangedCoreComponent {
    pub const fn order(&self) -> u64 {
        self.row.order
    }
    pub const fn commit(&self) -> CommitToken {
        self.row.operation
    }
    pub fn action(&self) -> &DefinitionRef {
        &self.row.action
    }
    pub const fn requester(&self) -> ObjId {
        self.row.requester
    }
    pub const fn hull(&self) -> ObjId {
        self.row.hull
    }
    pub const fn unit_revision(&self) -> u64 {
        self.row.unit_revision
    }
    pub const fn incoming(&self) -> ObjId {
        self.row.incoming
    }
    pub const fn source(&self) -> ObjId {
        self.row.source
    }
    pub const fn displaced(&self) -> Option<(ObjId, ObjId)> {
        match (self.row.outgoing, self.row.return_to) {
            (Some(o), Some(d)) => Some((o, d)),
            _ => None,
        }
    }
    pub const fn at_utc(&self) -> i64 {
        self.row.at_utc
    }
    pub fn object_digests(&self) -> &[ExchangeObjectDigest] {
        &self.row.objects
    }
    pub fn digest(&self) -> Result<DefinitionDigest, ParticipantError> {
        self.row.validate()?;
        DefinitionDigest::for_canonical_value(
            "industry.core_component_exchange.receipt.v1",
            &bounded(&self.row)?,
        )
        .map_err(|_| ParticipantError::Encoding)
    }
}
