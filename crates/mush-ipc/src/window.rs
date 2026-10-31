//! The one-step compatibility window (doc 33 §4.1, D-33-1; cut GG.0).
//!
//! The two halves may run one protocol step apart, in one direction: a server
//! at version N serves gateways at N-1 and N, and a gateway speaks only its own
//! version. Each half announces the versions it serves as `[min_proto, proto]`
//! in its Hello ([`ProtoWindow`]); the server records what each link announced
//! ([`LinkVersion`]) and passes every server → gateway frame through that
//! link's [`EmissionGate`], so a frame a gateway cannot decode is never sent to
//! it. The gateway's side is one check: the server's window must contain
//! [`IPC_VERSION`], else the link fails with [`IPC_VERSION_SKEW_MARKER`].
//!
//! The rules a protocol change follows are in [`crate::protocol`]'s module
//! docs ("The compatibility window"); this module is the mechanism.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::protocol::{IPC_MIN_PROTO, IPC_VERSION, ToGateway};

/// The fixed marker either half logs when its peer is outside the window.
/// Stable text an operator (or an alert rule) greps for; quoted by the proofs
/// in `mush-gateway`'s `split_halves` and `mush-server`'s `split_boundary`
/// suites. Do not reword it.
pub const IPC_VERSION_SKEW_MARKER: &str = "IPC-VERSION-SKEW";

/// The protocol versions one peer serves, `[min_proto, proto]`, as its Hello
/// announces them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProtoWindow {
    /// The oldest version served.
    pub min_proto: u32,
    /// The newest version served: the peer's own [`IPC_VERSION`].
    pub proto: u32,
}

impl ProtoWindow {
    /// This build's window: `[IPC_MIN_PROTO, IPC_VERSION]`.
    pub const OURS: ProtoWindow = ProtoWindow {
        min_proto: IPC_MIN_PROTO,
        proto: IPC_VERSION,
    };

    /// The window a Hello announced. A peer from before the window (version 12)
    /// sends no `min_proto`, which the defaulted field reads as `0`: such a
    /// peer serves exactly its own version. A `min_proto` above `proto` is
    /// nonsense and is read the same way.
    #[must_use]
    pub const fn announced(proto: u32, min_proto: u32) -> ProtoWindow {
        let min_proto = if min_proto == 0 || min_proto > proto {
            proto
        } else {
            min_proto
        };
        ProtoWindow { min_proto, proto }
    }

    /// Whether a peer at `version` is served by this window.
    #[must_use]
    pub const fn contains(&self, version: u32) -> bool {
        self.min_proto <= version && version <= self.proto
    }
}

impl fmt::Display for ProtoWindow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.min_proto == self.proto {
            write!(f, "v{}", self.proto)
        } else {
            write!(f, "v{}..=v{}", self.min_proto, self.proto)
        }
    }
}

/// The protocol version a link's peer announced, shared between the link's
/// reader (which records it from the first frame) and its writer (whose
/// [`EmissionGate`] reads it on every frame).
///
/// Until the peer has spoken, the link is treated as the oldest peer this
/// build serves ([`IPC_MIN_PROTO`]): a frame the oldest gateway cannot decode
/// is withheld rather than risked. In practice nothing gated can be produced
/// before the first frame, because a gateway's output is caused by frames it
/// sent; the greeting and the health tick are from before the window.
#[derive(Clone, Debug, Default)]
pub struct LinkVersion(Arc<AtomicU32>);

/// The value [`LinkVersion`] holds before the peer announces a version.
const NOT_ANNOUNCED: u32 = 0;

impl LinkVersion {
    /// A link whose peer has not announced a version yet.
    #[must_use]
    pub fn unknown() -> LinkVersion {
        LinkVersion(Arc::new(AtomicU32::new(NOT_ANNOUNCED)))
    }

    /// A link whose peer's version is already known.
    #[must_use]
    pub fn announced(version: u32) -> LinkVersion {
        let link = LinkVersion::unknown();
        link.record(version);
        link
    }

    /// Record the version the peer announced (or the bootstrap rule assumed).
    pub fn record(&self, version: u32) {
        self.0.store(version, Ordering::SeqCst);
    }

    /// The announced version, if any.
    #[must_use]
    pub fn get(&self) -> Option<u32> {
        match self.0.load(Ordering::SeqCst) {
            NOT_ANNOUNCED => None,
            version => Some(version),
        }
    }

    /// The version emission is gated at: the announced one, else the floor of
    /// this build's window.
    #[must_use]
    pub fn effective(&self) -> u32 {
        self.get().unwrap_or(IPC_MIN_PROTO)
    }
}

/// The version at which every variant that existed when the window opened was
/// introduced, as far as the window cares: a 12 peer decodes all of them, and
/// no older peer can be inside any window from here on.
pub const PRE_WINDOW: u32 = 12;

impl ToGateway {
    /// The protocol version that introduced this variant in its current
    /// shape. The emission gate sends a frame to a link only when the link
    /// announced at least this version.
    ///
    /// **Rule for the next bump (doc 33 §4.1):** a variant added at N lists
    /// `N` here. A defaulted field needs no entry (an old decoder ignores a
    /// field it does not know), unless the old peer's behaviour WITHOUT the
    /// field is wrong, in which case the variant's version moves to N and
    /// [`ToGateway::downgrade`] produces the N-1 shape.
    #[must_use]
    pub const fn introduced_at(&self) -> u32 {
        match self {
            ToGateway::SetOutputBrackets { .. }
            | ToGateway::WorldHealth { .. }
            | ToGateway::Deliver { .. }
            | ToGateway::Reply { .. }
            | ToGateway::RequestDisposition { .. }
            | ToGateway::SetFloodExempt { .. }
            | ToGateway::DiscordControl { .. }
            | ToGateway::CloseSession { .. }
            | ToGateway::Detach { .. }
            | ToGateway::CloseWebSessions { .. }
            | ToGateway::Hello { .. }
            | ToGateway::Bye => PRE_WINDOW,
        }
    }

    /// The older form of this frame for a link at `version`, when the link
    /// predates the frame's shape: `Some(older)` where an N-1 gateway should
    /// get something, `None` where the N-1 behaviour is to send nothing.
    ///
    /// Every arm here is a variant introduced inside a window that still has
    /// an N-1 peer to serve; an arm is deleted when its N-1 version leaves the
    /// window. There is none yet: every variant predates the window, so an
    /// older gateway gets the frame itself (see [`ToGateway::for_link`]).
    #[must_use]
    pub fn downgrade(self, version: u32) -> Option<ToGateway> {
        let _ = version;
        // Downgrade arms go here, one per gated variant with an older form:
        //     ToGateway::NewShape { conn, .. } => Some(ToGateway::OldShape { conn }),
        None
    }

    /// The frame to send a link at `version`: the frame itself when the link
    /// understands it, its [`downgrade`](ToGateway::downgrade) otherwise.
    #[must_use]
    pub fn for_link(self, version: u32) -> Option<ToGateway> {
        if self.introduced_at() <= version {
            Some(self)
        } else {
            self.downgrade(version)
        }
    }
}

/// The per-link emission gate: every server → gateway frame passes through
/// the gate of the link it is written to, which withholds (or downgrades) what
/// that link's announced version cannot decode.
///
/// The gate's notion of when a variant appeared is [`ToGateway::introduced_at`]
/// by default. [`EmissionGate::with_introduction_table`] is the test hook: a
/// proof can declare a variant newer than it is and watch the gate hold it
/// back from an older link, so the mechanism is tested while there is nothing
/// real to gate.
#[derive(Clone, Debug)]
pub struct EmissionGate {
    link: LinkVersion,
    introduced_at: fn(&ToGateway) -> u32,
}

impl EmissionGate {
    /// The production gate for one link.
    #[must_use]
    pub fn new(link: LinkVersion) -> EmissionGate {
        EmissionGate {
            link,
            introduced_at: ToGateway::introduced_at,
        }
    }

    /// A gate with its own table of introduction versions. **A test hook**: it
    /// exists so the gate can be proven to withhold a frame from an older link
    /// before any real variant is gated.
    #[doc(hidden)]
    #[must_use]
    pub fn with_introduction_table(
        link: LinkVersion,
        introduced_at: fn(&ToGateway) -> u32,
    ) -> EmissionGate {
        EmissionGate {
            link,
            introduced_at,
        }
    }

    /// The link this gate serves.
    #[must_use]
    pub fn link(&self) -> &LinkVersion {
        &self.link
    }

    /// The frame to write to this link, if any: `msg` when the link's version
    /// is at least the version that introduced it, its downgrade otherwise.
    #[must_use]
    pub fn admit(&self, msg: ToGateway) -> Option<ToGateway> {
        let version = self.link.effective();
        if (self.introduced_at)(&msg) <= version {
            Some(msg)
        } else {
            msg.downgrade(version)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn our_window_is_one_step_wide() {
        assert_eq!(ProtoWindow::OURS.proto, IPC_VERSION);
        assert_eq!(ProtoWindow::OURS.min_proto, IPC_VERSION - 1);
        assert!(ProtoWindow::OURS.contains(IPC_VERSION));
        assert!(ProtoWindow::OURS.contains(IPC_VERSION - 1));
        assert!(!ProtoWindow::OURS.contains(IPC_VERSION - 2));
        assert!(!ProtoWindow::OURS.contains(IPC_VERSION + 1));
    }

    #[test]
    fn a_hello_without_min_proto_serves_its_own_version_only() {
        // A version-12 server's Hello carries no `min_proto`; the defaulted 0
        // must read as "12 only", so a 13 gateway refuses it.
        let twelve = ProtoWindow::announced(12, 0);
        assert_eq!(twelve, ProtoWindow::announced(12, 12));
        assert!(twelve.contains(12));
        assert!(!twelve.contains(13));
        assert_eq!(twelve.to_string(), "v12");
        // Nonsense (a floor above the ceiling) is read the same way.
        assert_eq!(
            ProtoWindow::announced(13, 14),
            ProtoWindow::announced(13, 13)
        );
        assert_eq!(ProtoWindow::announced(13, 12).to_string(), "v12..=v13");
    }

    #[test]
    fn a_link_is_the_window_floor_until_it_speaks() {
        let link = LinkVersion::unknown();
        assert_eq!(link.get(), None);
        assert_eq!(link.effective(), IPC_MIN_PROTO);
        let shared = link.clone();
        shared.record(IPC_VERSION);
        assert_eq!(link.get(), Some(IPC_VERSION), "the clone shares the cell");
        assert_eq!(LinkVersion::announced(12).effective(), 12);
    }

    #[test]
    fn every_current_variant_predates_the_window() {
        // Nothing is gated yet: a 12 link and a 13 link both get every frame.
        for version in [PRE_WINDOW, IPC_VERSION] {
            let gate = EmissionGate::new(LinkVersion::announced(version));
            assert_eq!(gate.admit(ToGateway::Bye), Some(ToGateway::Bye));
            assert_eq!(
                gate.admit(ToGateway::SetFloodExempt {
                    char: 1,
                    exempt: true
                }),
                Some(ToGateway::SetFloodExempt {
                    char: 1,
                    exempt: true
                })
            );
        }
        assert_eq!(ToGateway::Bye.introduced_at(), PRE_WINDOW);
        assert_eq!(ToGateway::Bye.for_link(PRE_WINDOW), Some(ToGateway::Bye));
    }

    /// The test hook at work: declare `Bye` a version-13 frame and the gate
    /// withholds it from a 12 link, from a link that has not spoken, and
    /// hands it to a 13 link.
    #[test]
    fn a_frame_newer_than_the_link_is_withheld() {
        fn bye_is_new(msg: &ToGateway) -> u32 {
            match msg {
                ToGateway::Bye => 13,
                other => other.introduced_at(),
            }
        }
        let twelve = EmissionGate::with_introduction_table(LinkVersion::announced(12), bye_is_new);
        assert_eq!(twelve.admit(ToGateway::Bye), None);
        assert_eq!(
            twelve.admit(ToGateway::SetFloodExempt {
                char: 1,
                exempt: false
            }),
            Some(ToGateway::SetFloodExempt {
                char: 1,
                exempt: false
            }),
            "an ungated frame still passes"
        );
        let silent = EmissionGate::with_introduction_table(LinkVersion::unknown(), bye_is_new);
        assert_eq!(silent.admit(ToGateway::Bye), None, "unknown is the floor");
        let thirteen =
            EmissionGate::with_introduction_table(LinkVersion::announced(13), bye_is_new);
        assert_eq!(thirteen.admit(ToGateway::Bye), Some(ToGateway::Bye));
        // The link cell is shared: once the silent link announces 13, its
        // gate opens without being rebuilt.
        silent.link().record(13);
        assert_eq!(silent.admit(ToGateway::Bye), Some(ToGateway::Bye));
    }
}
