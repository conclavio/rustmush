//! Who issues the licence a renewal continues: the one parameter of the renewal twin.
//!
//! [`Issuer`] is sealed. Its supertrait lives in a module private to this file, so the two
//! types named here, [`Personal`] and [`House`], are the only ones that can ever implement
//! it, and each can be implemented once: `renewal.rs` and `house_renewal.rs`. The compiler
//! resolves an issuer at every call; there is no `dyn Issuer`, no table and no registration.
use super::*;

mod sealed {
    /// The seal. Nothing outside `issuer.rs` can name this trait, so nothing outside it can
    /// make a type an [`Issuer`](super::Issuer).
    pub trait Sealed {}
    impl Sealed for super::Personal {}
    impl Sealed for super::House {}
}

/// A licence a player sold: the personal renewal families (`*_mind_renewal_*`).
#[derive(Clone, Copy, Debug)]
pub(in crate::action_state) struct Personal;

/// A licence an organisation issued: the House renewal families (`*_house_mind_renewal_*`).
#[derive(Clone, Copy, Debug)]
pub(in crate::action_state) struct House;

/// What differs between the personal and the House renewal in the kernel.
///
/// The constants are identity (doc 46 §4.4): every one is persisted or digested, and holds
/// the value its family has always had. The methods are the birth proof (§4.3 items 4, 5
/// and 8): which closed purchase audits a renewal needs, which economics family its grant
/// belongs to, and how the licence's immutable birth is proven. Money, the payee, the
/// issuer's current authority and the player's own stop rules are the owners' obligations
/// and are not here.
///
/// A third implementation does not compile. From outside the crate the trait cannot be
/// named at all:
///
/// ```compile_fail
/// use mush_core::action_state::terminal_capacity::renewal_twin::Issuer;
/// struct Guild;
/// impl Issuer for Guild {}
/// ```
///
/// From inside it, `impl Issuer for Guild` fails on the supertrait (`Guild: Sealed` is not
/// satisfied) and `impl sealed::Sealed for Guild` fails because `sealed` is private to this
/// file. The test `the_issuer_seal_admits_exactly_two` holds the source to that.
pub(in crate::action_state::terminal_capacity) trait Issuer:
    sealed::Sealed + Sized + 'static
{
    /// The five action ids, in transition order: draft, fund, settle, refund, close.
    const ACTIONS: [&'static str; 5];
    /// The service's own close of an unfunded offer.
    const SERVICE_ACTION: &'static str;
    /// The `protocol` name in the five actions' payloads.
    const PROTOCOL: &'static str;
    /// The `protocol` name in the service action's payload.
    const SERVICE_PROTOCOL: &'static str;
    /// The market's operation family.
    const MARKET: &'static str;
    /// The Roll's operation family.
    const ROLL: &'static str;
    /// The Roll's head family, one row a licence.
    const HEAD: &'static str;
    /// The market's receipt family.
    const RECEIPT: &'static str;
    /// The Roll's term-economics family the grant row is written to.
    const ECONOMICS: &'static str;
    /// The other issuer's two operation families (Roll, market): an operation that has a
    /// row in either belongs to the other recipe.
    const OTHER_ISSUER: [&'static str; 2];
    /// Digest domain of the predecessor term the binding names.
    const PREDECESSOR_DOMAIN: &'static str;
    /// Digest domain of the Economy offer.
    const OFFER_DOMAIN: &'static str;
    /// Digest domain of the grant's renewal template.
    const TEMPLATE_DOMAIN: &'static str;
    /// Digest domain of the capacity fingerprint.
    const CAPACITY_DOMAIN: &'static str;

    /// This issuer's field of the registry. The registry's fields stay a closed, hand-edited
    /// list; this only says which one is whose.
    fn protocol(registry: &StateFamilyRegistry) -> Option<&Protocol<Self>>;

    /// The closed purchase audits a renewal of this issuer cannot be declared without.
    fn birth_audits(registry: &StateFamilyRegistry) -> Result<(), ActionStateError>;

    /// The economics family of the licence's birth: the digest domain of a term's
    /// economics row.
    fn economics_domain(economics: &Value) -> Result<&'static str, ActionStateError>;
    /// The digest domain of the grant.
    fn grant_domain(grant: &Value) -> Result<&'static str, ActionStateError>;
    /// The grant's format, which is the format of the economics row a Grant writes.
    fn grant_version(grant: &Value) -> Result<u64, ActionStateError>;

    /// The birth check made before the predecessor term is read. The personal issuer
    /// proves a Follow binding's immutable acquisition here; the House proves its whole
    /// birth later, in [`Issuer::birth_route`].
    fn follow_birth(view: &View<'_>, binding: &Value) -> Result<(), ActionStateError>;

    /// Whether the offer's royalty chain differs from the grant's. Only a personal licence
    /// has a royalty chain; the House refuses an offer that carries one, in
    /// [`Issuer::birth_route`].
    fn royalty_chain_differs(basis: &Value, grant: &Value) -> Result<bool, ActionStateError>;

    /// The birth check made after the predecessor and the disclosed terms agree. The House
    /// proves its exact allocation, issuer and native shape here; the personal issuer has
    /// proven its birth already.
    fn birth_route(
        view: &View<'_>,
        binding: &Value,
        basis: &Value,
        terms: &Value,
    ) -> Result<(), ActionStateError>;

    /// At a Draft, Fund or Grant: the birth's purchase recipe must still be one the
    /// registry declares. Only the House asks; a personal Follow birth is held to it in
    /// [`Issuer::follow_birth`].
    fn birth_product(after: &View<'_>, binding: &Value) -> Result<(), ActionStateError>;
}
