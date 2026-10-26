//! Exact original-author compatible updates; every new file retains its birth lineage.
pub(crate) mod audit;
pub(crate) mod derived;
pub(crate) mod provider;
pub(crate) mod publication;
pub(crate) mod records;
pub(crate) mod registration;
use super::records::PublicationError;
use mush_core::mechanics::DefinitionRef;
pub(crate) const ACTION: &str = "update_original_mind";
pub(crate) const DERIVED_ACTION: &str = "update_derived_mind";
pub(crate) fn check_action(action: &DefinitionRef) -> Result<(), PublicationError> {
    if action.key().owner.as_str() != "roll"
        || action.key().kind.as_str() != "action"
        || action.key().id.as_str() != ACTION
        || action.version() != 1
    {
        return Err(PublicationError::Invalid);
    }
    Ok(())
}

#[cfg(test)]
mod tests;

/// Select only the closed shape requested by the immutable operation format.
pub(crate) fn check_derived_action(action: &DefinitionRef) -> Result<(), PublicationError> {
    if action.key().owner.as_str() != "roll"
        || action.key().kind.as_str() != "action"
        || action.key().id.as_str() != DERIVED_ACTION
        || action.version() != 1
    {
        return Err(PublicationError::Invalid);
    }
    Ok(())
}
