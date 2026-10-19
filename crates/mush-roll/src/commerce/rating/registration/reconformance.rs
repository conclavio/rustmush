//! Fixed Registry-owned re-conformance profile. These references are data;
//! only actual registered handles and private prepared tickets authorize work.
use super::*;

pub const PROTOCOL: &str = "registry_reconformance_v1";
pub const ACTION_IDS: [&str; 5] = [
    "reconformance_admit",
    "reconformance_grant",
    "reconformance_settle",
    "reconformance_cancel",
    "reconformance_recover",
];

#[derive(Clone)]
pub struct ReconformanceDefinitions {
    pub(crate) manifest: DefinitionRef,
    pub(crate) descriptors: [DefinitionDescriptor; 5],
    pub(crate) actions: [DefinitionRef; 5],
    pub(crate) outcome: DefinitionDescriptor,
}
impl ReconformanceDefinitions {
    pub fn new(manifest: DefinitionRef) -> Result<Self, ModError> {
        let ordinary = RegistryDefinitions::new(manifest.clone())?;
        let refs = vec![
            manifest.clone(),
            reference("roll", commercial_roots_descriptor()?)?,
            reference("tacsim", ordinary.quota.clone())?,
            reference("tacsim", ordinary.artifacts.clone())?,
        ];
        let mut descriptors = Vec::with_capacity(5);
        let mut actions = Vec::with_capacity(5);
        for (id, word) in ACTION_IDS
            .into_iter()
            .zip(["admit", "grant", "settle", "cancel", "recover"])
        {
            let definition = descriptor(
                "action",
                id,
                json!({"format":1,"protocol":PROTOCOL,"transition":word}),
            )?
            .with_references(refs.clone());
            actions.push(reference("roll", definition.clone())?);
            descriptors.push(definition);
        }
        Ok(Self {
            manifest,
            descriptors: descriptors
                .try_into()
                .map_err(|_| error("reconformance action count"))?,
            actions: actions
                .try_into()
                .map_err(|_| error("reconformance action count"))?,
            outcome: descriptor(
                "outcome",
                "reconformance_transition_committed",
                json!({"format":1,"protocol":PROTOCOL}),
            )?,
        })
    }
    pub fn actions(&self) -> &[DefinitionRef; 5] {
        &self.actions
    }
    pub fn manifest(&self) -> &DefinitionRef {
        &self.manifest
    }
    pub(crate) fn definitions(&self) -> impl Iterator<Item = DefinitionDescriptor> + '_ {
        self.descriptors
            .iter()
            .cloned()
            .chain(std::iter::once(self.outcome.clone()))
    }
}
