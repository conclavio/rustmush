//! **The space cast's doctrines, by digest** (doc 44 unit MW.10; Q-44-25 as corrected in doc
//! 44 §4a; doc 35 `D-35-17`).
//!
//! The pack's cast (`content/space/picket_cast.json`) names, for each NPC actor of a space
//! bubble, the doctrine she flies and that doctrine's digest. A bubble seals its actors when it
//! opens, from the store, by that digest; a seal is never changed afterwards, so a bubble that
//! is open keeps what it sealed.
//!
//! # Which doctrine an actor flies
//!
//! 1. the one staff selected for her (`@mind/cast <actor>=…`), a pointer in the world's own
//!    documents (`tacsim:doctrine:cast.<actor>`, [`crate::store::doctrine_key`]);
//! 2. else the one the pack's cast names.
//!
//! Either way the store must hold a readable source under the digest. If it does not, **the
//! actor does not sail**: the boot says so in a line that names the actor and the digest
//! ([`boot`], kept on [`TacsimStore::cast_faults`]), every seal site refuses, and a carrier
//! that would have met her passes her waypoint with no bubble. Nothing falls back to another
//! doctrine: not a selection to the pack's, and not the pack's to the bytes compiled into the
//! binary.
//!
//! # A changed doctrine is a new revision
//!
//! The source store is addressed by digest and never rewritten (`TacsimStore`'s
//! `check_mind_source`). Staff do not edit a doctrine: they publish other
//! bytes with the workshop's verbs (`@mind/new`, `@mind/paste`, `@mind/save`), which stores
//! them under their own digest, and select that digest for an actor here. The old bytes stay
//! under the old digest, which every seal taken before the selection still names.
//!
//! # What this module leaves alone
//!
//! The collector drone's doctrine (`drone_mining`) is not a cast actor and is still sealed with
//! the digest compiled in (doc 44 unit MW.12). The link-loss backup mind reads its own pointer,
//! `tacsim:doctrine:linkdead_return`, on every think ([`crate::space`]'s `backup` module, doc
//! 35 §11 `B-3`); nothing here writes it. A House's policy and a port's arms are compiled
//! (Q-44-32, deferred); when one becomes a doctrine it is named the same way, by a role under
//! `tacsim:doctrine:` and a digest in the store.

use mush_core::{CmdError, Ctx};
use smol_str::SmolStr;

use crate::store::TacsimStore;

/// One NPC actor of the space cast: a key of `content/space/picket_cast.json`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CastActor {
    /// The customs corvette of the Kirkwood picket.
    Corvette,
    /// The raider that lurks beside the Kirkwood picket.
    Raider,
    /// The interdictor of a waypoint interdiction.
    Interdictor,
}

impl CastActor {
    /// Every actor, in the cast file's order.
    pub const ALL: [Self; 3] = [Self::Corvette, Self::Raider, Self::Interdictor];

    /// The actor's key in the cast file, and her name in `@mind/cast`.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Corvette => "corvette",
            Self::Raider => "raider",
            Self::Interdictor => "interdictor",
        }
    }

    /// The actor `word` names, whatever its case.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|actor| actor.word().eq_ignore_ascii_case(word.trim()))
    }

    /// The tail of the actor's selection pointer (`tacsim:doctrine:cast.<actor>`).
    #[must_use]
    pub fn role(self) -> String {
        format!("cast.{}", self.word())
    }

    fn entry(self, book: &crate::space::PicketCastBook) -> &crate::space::PicketCastEntry {
        match self {
            Self::Corvette => &book.corvette,
            Self::Raider => &book.raider,
            Self::Interdictor => &book.interdictor,
        }
    }
}

/// The doctrine an actor's next seal names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CastDoctrine {
    /// Whose it is.
    pub actor: CastActor,
    /// The doctrine's name as the pack's cast writes it: the seal's `mind`.
    pub name: SmolStr,
    /// The digest the seal will name. The store holds its source.
    pub digest: [u8; 32],
    /// The digest the pack's cast names for this actor.
    pub pack_digest: [u8; 32],
    /// Whether `digest` is staff's selection (true) or the pack's (false).
    pub selected: bool,
}

/// Why an actor has no doctrine to seal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CastFault {
    /// The cast file did not load, or names a digest that is not 64 base-16 characters.
    NoCast {
        /// Whose entry was asked for.
        actor: CastActor,
    },
    /// The store holds no readable source under the digest the actor is to fly.
    NotHeld {
        /// Whose doctrine it is.
        actor: CastActor,
        /// The doctrine's name in the pack's cast.
        name: SmolStr,
        /// The digest the store does not hold.
        digest: [u8; 32],
        /// Whether the digest is staff's selection (true) or the pack's (false).
        selected: bool,
    },
}

impl std::fmt::Display for CastFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoCast { actor } => write!(
                f,
                "the space cast did not load, so its {} has no doctrine and does not sail",
                actor.word()
            ),
            Self::NotHeld {
                actor,
                name,
                digest,
                selected: false,
            } => write!(
                f,
                "the space cast's {actor} flies {name} by digest {digest}, and the store holds no \
                 source under that digest: no {actor} sails until the store holds it or staff \
                 select a held revision (@mind/cast {actor}=<mind>@<revision>)",
                actor = actor.word(),
                digest = crate::arena::digest_text(digest),
            ),
            Self::NotHeld {
                actor,
                name,
                digest,
                selected: true,
            } => write!(
                f,
                "the doctrine selected for the space cast's {actor} (in place of {name}) is digest \
                 {digest}, and the store holds no source under that digest: no {actor} sails \
                 until staff select a held revision or clear the selection (@mind/cast \
                 {actor}=PACK)",
                actor = actor.word(),
                digest = crate::arena::digest_text(digest),
            ),
        }
    }
}

/// The doctrine the pack's cast names for `actor`, and its digest.
///
/// # Errors
///
/// [`CastFault::NoCast`] when the cast file did not load or its digest does not parse.
pub fn pack_doctrine(actor: CastActor) -> Result<(SmolStr, [u8; 32]), CastFault> {
    let book = crate::space::picket_cast_book().ok_or(CastFault::NoCast { actor })?;
    let entry = actor.entry(book);
    let digest = entry.doctrine_digest().ok_or(CastFault::NoCast { actor })?;
    Ok((SmolStr::new(&entry.doctrine), digest))
}

/// **The doctrine `actor`'s next seal names**: staff's selection if the world holds one, else
/// the pack's, and in both cases only if the store holds its source.
///
/// With no store at all (a unit rig with no world) the pack's digest is answered when the
/// binary carries the source it names ([`super::source_lookup`]); a world always has a store.
///
/// # Errors
///
/// [`CastFault`], whose text names the actor and the digest.
pub fn cast_doctrine(
    store: Option<&TacsimStore>,
    actor: CastActor,
) -> Result<CastDoctrine, CastFault> {
    let (name, pack_digest) = pack_doctrine(actor)?;
    let selection = store.and_then(|store| store.doctrine_pointer(&actor.role()));
    let (digest, selected) = selection.map_or((pack_digest, false), |digest| (digest, true));
    let held = match store {
        Some(store) => store.has_mind_source(&digest),
        None => super::source_lookup(None, &digest).is_some(),
    };
    if !held {
        return Err(CastFault::NotHeld {
            actor,
            name,
            digest,
            selected,
        });
    }
    Ok(CastDoctrine {
        actor,
        name,
        digest,
        pack_digest,
        selected,
    })
}

/// **The boot's pass over the space cast** (doc 44 unit MW.10), run once the store has loaded
/// and before the space books are read back.
///
/// It stores the shipped space doctrines under their digests, as the boot stores Standard
/// Doctrine (`tacsim:mindsrc:<sha256>`, written by the first boot that finds no document under
/// the key and never again; a key that is claimed by a document that does not read is left as
/// it is). Then it resolves every actor. An actor whose doctrine the store does not hold is
/// refused: one error line that names her and the digest, kept on
/// [`TacsimStore::cast_faults`]. The boot goes on, as it does past any document it cannot
/// use, and that actor does not sail.
pub fn boot(ctx: &mut Ctx<'_>) {
    let persisted = ctx
        .w
        .module_state::<TacsimStore>()
        .is_some_and(TacsimStore::enabled);
    if persisted {
        for source in super::shipped_space_sources() {
            crate::store::put_mind_source(ctx, source);
        }
    }
    let Some(store) = ctx.w.module_state_mut::<TacsimStore>() else {
        return;
    };
    // A world whose layer is off writes no document; its shipped sources are held in RAM.
    super::ensure_space_mind_sources_installed(store);
    let faults: Vec<String> = CastActor::ALL
        .into_iter()
        .filter_map(|actor| cast_doctrine(Some(store), actor).err())
        .map(|fault| fault.to_string())
        .collect();
    for fault in &faults {
        tracing::error!("tacsim: {fault}");
    }
    store.cast_faults = faults;
}

/// `@mind/cast`'s usage.
pub(crate) const USAGE: &str = "@mind/cast (the space cast: each actor's doctrine and its digest)\n@mind/cast <corvette|raider|interdictor>=<owned mind>@<revision> | <64-character digest> | PACK";

fn refuse(text: impl Into<String>) -> CmdError {
    CmdError::msg(text.into())
}

/// **`@mind/cast`** (staff; doc 44 unit MW.10).
///
/// Bare, it lists each actor of the space cast: the doctrine she flies, its digest, and
/// whether that is the pack's or staff's selection, or the refusal if the store does not hold
/// it. With `<actor>=<what>` it selects the doctrine her next seal names:
///
/// * `<owned mind>@<revision>`: a revision the asker saved in the workshop (`@mind/save`
///   prints its number and its digest);
/// * a digest of 64 base-16 characters: any source the store holds, whoever published it;
/// * `PACK`: clear the selection, so she flies what the pack's cast names.
///
/// The selection is one pointer document, `tacsim:doctrine:cast.<actor>`. It is read when a
/// bubble opens and at no other time: a bubble that is open keeps the seal it took.
///
/// # Errors
///
/// A refusal in words: not staff, an unknown actor, a revision that is not saved, a digest the
/// store does not hold, or a source that does not compile as a mind.
pub(crate) fn command(ctx: &mut Ctx<'_>, text: &str) -> Result<(), CmdError> {
    if !(mush_core::perm::has_power(ctx.w, ctx.actor, mush_core::Powers::SIM_ADMIN)
        || mush_core::perm::has_role(ctx.w, ctx.actor, mush_core::Role::Royalty))
    {
        return Err(refuse("Permission denied."));
    }
    let text = text.trim();
    if text.is_empty() {
        for line in listing(ctx.w.module_state::<TacsimStore>()) {
            ctx.out.info(ctx.actor, line);
        }
        return Ok(());
    }
    let (actor, what) = text.split_once('=').ok_or_else(|| refuse(USAGE))?;
    let actor = CastActor::parse(actor).ok_or_else(|| {
        refuse(format!(
            "No such actor in the space cast: {}. They are corvette, raider and interdictor.",
            actor.trim()
        ))
    })?;
    let what = what.trim();
    if what.eq_ignore_ascii_case("PACK") {
        let had = ctx
            .w
            .module_state::<TacsimStore>()
            .and_then(|store| store.doctrine_pointer(&actor.role()))
            .is_some();
        crate::store::put_doctrine_pointer(ctx, &actor.role(), None);
        let now = cast_doctrine(ctx.w.module_state::<TacsimStore>(), actor);
        let line = match (had, now) {
            (_, Err(fault)) => format!("Selection cleared, and {fault}."),
            (true, Ok(doctrine)) => format!(
                "Selection cleared. The next {} to sail flies the pack's {} ({}); one already out \
                 keeps the doctrine she sailed with.",
                actor.word(),
                doctrine.name,
                crate::arena::digest_text(&doctrine.digest)
            ),
            (false, Ok(doctrine)) => format!(
                "The {} already flies the pack's {} ({}).",
                actor.word(),
                doctrine.name,
                crate::arena::digest_text(&doctrine.digest)
            ),
        };
        ctx.out.info(ctx.actor, line);
        return Ok(());
    }
    let digest = selected_digest(ctx, what)?;
    let store = ctx
        .w
        .module_state::<TacsimStore>()
        .ok_or_else(|| refuse("The mind source store is not available."))?;
    let Some(source) = store.mind_source(&digest) else {
        return Err(refuse(format!(
            "The store holds no source under {}. Publish it first: @mind/new, @mind/paste, \
             @mind/save.",
            crate::arena::digest_text(&digest)
        )));
    };
    // The check a seal site would fail on at the first think: the bytes compile as a mind.
    if let Err(error) = crate::minds::RhaiRunner::new(
        &crate::config::ArenaConfig::default(),
        source,
        mush_mind::Digest(digest),
    ) {
        return Err(refuse(format!(
            "That source does not compile as a mind, so it is not selected: {error}"
        )));
    }
    crate::store::put_doctrine_pointer(ctx, &actor.role(), Some(digest));
    tracing::info!(
        actor = actor.word(),
        by = ctx.actor.0,
        digest = %crate::arena::digest_text(&digest),
        "tacsim: a doctrine was selected for a space cast actor"
    );
    ctx.out.info(
        ctx.actor,
        format!(
            "Selected. The next {} to sail flies {}; one already out keeps the doctrine she \
             sailed with. @mind/cast {}=PACK clears the selection.",
            actor.word(),
            crate::arena::digest_text(&digest),
            actor.word()
        ),
    );
    Ok(())
}

/// The digest `what` names: a digest written out, or a revision the asker saved.
fn selected_digest(ctx: &Ctx<'_>, what: &str) -> Result<[u8; 32], CmdError> {
    if what.len() == 64
        && let Some(digest) = crate::arena::digest_from_text(&what.to_ascii_lowercase())
    {
        return Ok(digest);
    }
    let reference =
        crate::minds::workshop::MindRef::parse(what).map_err(|error| refuse(error.to_string()))?;
    let roll = ctx
        .w
        .module_state::<mush_roll::RollStore>()
        .ok_or_else(|| refuse("The workshop is not available."))?;
    let revision =
        crate::minds::workshop::saved_revision(roll, ctx.actor, &reference).map_err(|_| {
            refuse(format!(
                "{what} is not a revision you have saved. Name one as <mind>@<revision>; \
                 @mind/save prints the number and the digest."
            ))
        })?;
    Ok(*revision.source().as_bytes())
}

/// The bare verb's lines.
fn listing(store: Option<&TacsimStore>) -> Vec<String> {
    let mut lines = vec![
        "Space cast. A bubble seals each actor's doctrine when it opens and keeps it until it \
         closes."
            .to_owned(),
    ];
    for actor in CastActor::ALL {
        lines.push(match cast_doctrine(store, actor) {
            Ok(doctrine) if doctrine.selected => format!(
                "  {}: {} — selected by staff (the pack's {} is {})",
                actor.word(),
                crate::arena::digest_text(&doctrine.digest),
                doctrine.name,
                crate::arena::digest_text(&doctrine.pack_digest)
            ),
            Ok(doctrine) => format!(
                "  {}: {} {} — the pack's",
                actor.word(),
                doctrine.name,
                crate::arena::digest_text(&doctrine.digest)
            ),
            Err(fault) => format!("  {}: REFUSED — {fault}", actor.word()),
        });
    }
    lines
}

#[cfg(test)]
mod tests {
    use sha2::{Digest as _, Sha256};

    use super::*;

    fn sha(source: &str) -> [u8; 32] {
        Sha256::digest(source.as_bytes()).into()
    }

    /// The pack's cast names each doctrine by the digest of the source the tree ships for
    /// it. When a shipped doctrine's bytes change (an ABI move rewrites the `api:` in its
    /// header), this fails and prints the line to write into `picket_cast.json`.
    #[test]
    fn the_packs_cast_names_each_shipped_doctrine_by_its_digest() {
        for (actor, name, source) in [
            (
                CastActor::Corvette,
                "kirkwood_picket",
                crate::minds::content::kirkwood_picket_source(),
            ),
            (
                CastActor::Raider,
                "belt_raider",
                crate::minds::content::belt_raider_source(),
            ),
            (
                CastActor::Interdictor,
                "interdictor_patrol",
                crate::minds::content::interdictor_patrol_source(),
            ),
        ] {
            let (cast_name, cast_digest) = pack_doctrine(actor).expect("the cast loads");
            assert_eq!(cast_name, name);
            assert_eq!(
                crate::arena::digest_text(&cast_digest),
                crate::arena::digest_text(&sha(source)),
                "content/space/picket_cast.json: {}'s \"doctrine_sha256\" is not the SHA-256 of \
                 content/minds/{name}.rhai; write the right-hand value there",
                actor.word()
            );
        }
    }

    #[test]
    fn an_actor_is_named_by_its_cast_key_in_any_case() {
        for actor in CastActor::ALL {
            assert_eq!(CastActor::parse(actor.word()), Some(actor));
            assert_eq!(
                CastActor::parse(&format!(" {} ", actor.word().to_uppercase())),
                Some(actor)
            );
        }
        assert_eq!(CastActor::parse("escort"), None);
        assert_eq!(CastActor::Interdictor.role(), "cast.interdictor");
    }

    /// A store that holds the shipped sources resolves every actor to the pack's digest; a
    /// selection the store holds wins; a selection it does not hold is a refusal that names
    /// the actor and the digest, and is not answered with the pack's doctrine.
    #[test]
    fn a_selection_wins_and_an_unheld_one_is_refused_by_name() {
        let mut store = TacsimStore::default();
        crate::minds::content::ensure_space_mind_sources_installed(&mut store);
        let pack = cast_doctrine(Some(&store), CastActor::Interdictor).expect("the pack's");
        assert!(!pack.selected);
        assert_eq!(pack.digest, pack.pack_digest);
        assert_eq!(pack.name, "interdictor_patrol");

        let revision = "fn think(v, c) { hold(); }\n";
        let digest = crate::store::install_mind_source(&mut store, revision).expect("stored");
        store
            .house_doctrines
            .insert(SmolStr::new(CastActor::Interdictor.role()), digest);
        let selected = cast_doctrine(Some(&store), CastActor::Interdictor).expect("selected");
        assert!(selected.selected);
        assert_eq!(selected.digest, digest);
        assert_eq!(selected.pack_digest, pack.digest);
        // The others are untouched.
        assert!(
            !cast_doctrine(Some(&store), CastActor::Raider)
                .expect("raider")
                .selected
        );

        let absent = [0x5a; 32];
        store
            .house_doctrines
            .insert(SmolStr::new(CastActor::Interdictor.role()), absent);
        let fault = cast_doctrine(Some(&store), CastActor::Interdictor).expect_err("unheld");
        assert_eq!(
            fault,
            CastFault::NotHeld {
                actor: CastActor::Interdictor,
                name: SmolStr::new("interdictor_patrol"),
                digest: absent,
                selected: true,
            }
        );
        let line = fault.to_string();
        assert!(line.contains("interdictor"), "{line}");
        assert!(line.contains(&"5a".repeat(32)), "{line}");
    }

    /// The pack's own digest is refused the same way when the store does not hold it: a store
    /// with no sources has nothing to seal, though the binary carries the bytes.
    #[test]
    fn the_packs_digest_is_refused_when_the_store_does_not_hold_it() {
        let store = TacsimStore::default();
        for actor in CastActor::ALL {
            let fault = cast_doctrine(Some(&store), actor).expect_err("an empty store");
            let (_, digest) = pack_doctrine(actor).expect("the cast loads");
            let line = fault.to_string();
            assert!(line.contains(actor.word()), "{line}");
            assert!(line.contains(&crate::arena::digest_text(&digest)), "{line}");
            assert!(
                matches!(
                    fault,
                    CastFault::NotHeld {
                        selected: false,
                        ..
                    }
                ),
                "{fault:?}"
            );
        }
        // A rig with no world at all reads the bytes the binary carries.
        assert!(cast_doctrine(None, CastActor::Corvette).is_ok());
    }

    /// Other bytes under a digest are refused. Published through the store they are simply
    /// another digest, and the first document is untouched. Written as a document under the
    /// first digest's key they do not load, the key stays claimed, and the store will not
    /// take even the right bytes under it again: it never rewrites a key.
    #[test]
    fn the_store_refuses_other_bytes_under_a_digest() {
        let shipped = crate::minds::content::interdictor_patrol_source();
        let other = shipped.replace("speed(1000)", "hold()");
        assert_ne!(other, shipped);

        let mut store = TacsimStore::default();
        let digest = crate::store::install_mind_source(&mut store, shipped).expect("stored");
        assert_eq!(digest, sha(shipped));
        let second = crate::store::install_mind_source(&mut store, &other).expect("stored");
        assert_ne!(second, digest, "other bytes are another digest");
        assert_eq!(store.mind_source(&digest), Some(shipped));

        // A document under the first digest's key that carries the other bytes.
        let tampered = vec![(
            crate::store::mindsrc_key(&digest),
            serde_json::json!({
                "sha256": crate::arena::digest_text(&digest),
                "abi": crate::minds::MIND_ABI,
                "bytes": other,
            }),
        )];
        let mut loaded = crate::store::load(&tampered, crate::TacsimConfig::default());
        assert!(
            !loaded.has_mind_source(&digest),
            "bytes that do not hash to the key are not held"
        );
        assert!(
            crate::store::install_mind_source(&mut loaded, shipped).is_none(),
            "and the key stays claimed: the right bytes are not written over it"
        );
        assert!(!loaded.has_mind_source(&digest));
        // So the cast, which names that digest, is refused by name.
        crate::minds::content::ensure_space_mind_sources_installed(&mut loaded);
        let fault = cast_doctrine(Some(&loaded), CastActor::Interdictor).expect_err("unheld");
        let line = fault.to_string();
        assert!(line.contains("interdictor"), "{line}");
        assert!(line.contains(&crate::arena::digest_text(&digest)), "{line}");
        assert!(cast_doctrine(Some(&loaded), CastActor::Corvette).is_ok());
    }
}
