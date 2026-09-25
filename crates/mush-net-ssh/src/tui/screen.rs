//! The screen model (interface plan I2, D-I-7, D-I-17, D-I-18, D-I-19):
//! numbered screens of titled panes, each with a legend of single-key
//! bindings, a drop order for small terminals, and — for the scene screen
//! — the room's one-letter exits as keys.
//!
//! A binding is [`KeyAction::Local`] (a client toggle), a
//! [`KeyAction::Prefill`] (enter input mode with a line ready; nothing is
//! sent until Enter), or a [`KeyAction::Run`], and `Run` is confined to the
//! read-only verbs in [`RUN_ALLOWLIST`]: [`validate_screens`] refuses
//! anything else, so a bare key can never fire, spend or send. The one
//! carve-out — walking an exit on the scene screen — is not a table entry
//! at all: the app builds it from each `Room` event (D-I-19).
//!
//! Screens are data. The built-ins live in [`builtin_screens`]; a ruleset
//! screen is one more [`Screen`] value passed to the app, not a rewrite.

use serde_json::Value;

use super::panels::{ARENA_MAP_FEED, COCKPIT_FEED, CONTACTS_FEED, MAP_FEED, UNIT_FEED};
use super::render::{PaneRows, RenderCtx};

/// The most screens a TUI can register (the hotkey digits `0`–`9`).
///
/// **16b cut Q.4** took the tenth: nine screens spent `1`–`9`, and the cockpit
/// doc 16 §5 asks for had nowhere to go. `0` is the digit a keyboard puts after
/// `9`, and the tab strip reads `1 … 9 0` for that reason.
pub const MAX_SCREENS: usize = 10;
/// The most panes one screen may declare.
pub const MAX_PANES_PER_SCREEN: usize = 8;

/// **The width every screen is laid out for** (16c `Q-AB1`, 16b cut AB.1).
///
/// The owner ruled on 2026-09-24 that 120 columns is the narrowest terminal a
/// client surface is built for (160 to 180 is typical, 240 and more is
/// reasonable), and that nothing is built to 80 again. Every screen's panes fit
/// side by side at this width; narrower than it `TuiApp::draw` paints no screen
/// at all, only one sentence naming the floor and the terminal's own width
/// (`render::floor_line`) above the input box. The floor is a WIDTH: a short
/// terminal keeps dropping panes in each screen's declared `degrade` order, as
/// 16b cut B.3 built it.
pub const FLOOR_COLS: u16 = 120;

/// **The terminal height from which the cockpit is drawn whole** for the
/// shipped tank (16b cut AB.1): seven rows of chrome with the vitals gauge
/// (tab strip, status line, gauge, the three-row input box, legend) and the
/// Left column's `9 + 14` — the status pane's seven rows and the damage pane's
/// twelve, each with its border. Below it the cockpit's `degrade` order is live
/// and `damage` goes first. Read by the screen test and named in comments;
/// `TuiApp::draw` never reads it, because there is no row floor.
pub const COCKPIT_ROWS: u16 = 30;

/// The letters the scene screen never binds itself, so a room's exits can
/// always use them (D-I-19). Other screens may bind them freely.
pub const RESERVED_EXIT_KEYS: [char; 7] = ['n', 'e', 's', 'w', 'u', 'd', 'o'];

/// The keys every screen inherits and may not rebind (§7 of the plan):
/// the digits, the mode triggers, the palette, help, and the pane-jump
/// letters.
pub const GLOBAL_KEYS: [char; 16] = [
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', '"', ':', ';', '\\', '/', '?',
];
/// `g` / `G` jump a pane to its top and bottom in nav mode.
pub const JUMP_KEYS: [char; 2] = ['g', 'G'];

/// Verbs a bare key may send (D-I-18). Every entry reads state and changes
/// nothing; the probe in [`validate_screens`] holds every `Run` to this
/// list by its first word.
///
/// Four are bound today — `look`, `inventory`, `+roll/last`, `+contacts` —
/// and the rest are reserved for the screens a ruleset declares. Look-back
/// LB-16 took two off it: `@mail <n>` MARKS THE MESSAGE READ (`mush-mail`)
/// and `+orders` writes work orders, so neither belonged on a list whose
/// whole promise is that a bare key mutates nothing. The list is one of two
/// hand-kept copies (`web/src/lib/stores/screens.svelte.ts` is the other);
/// both are pinned to the committed fixture `web/src/lib/run-allowlist.json`
/// — asserted below and by a vitest — until K8.5's read-only classification
/// can replace the hand-keeping.
/// **16b cut Q.4** added the cockpit's seven, which are the seven of the twelve
/// cockpit verbs that only read: `VIEW`, `CONTACTS`, `STATUS`, `WEAPONSTATUS`,
/// `SCAN`, `RANGE` and `BEARING` all answer off the last frame the arena
/// published and stage no intent (`tacsim::match_::cockpit::register` says so
/// in as many words). The other five — `STARTUP`, `HEADING`, `SPEED`, `LOCK`,
/// `FIRE` — each stage one, so none of them is here and none of them is a bare
/// key: the cockpit screen only ever PREFILLS them. **16b cut B.3** unbound
/// `v` (`VIEW`'s grid never reached the TUI's screen); `view` stays listed,
/// because the list is the web's too and a screen may yet bind it.
pub const RUN_ALLOWLIST: &[&str] = &[
    "look",
    "l",
    "who",
    "inventory",
    "i",
    "+sheet",
    "+contacts",
    "+intel",
    "+actions",
    "+scene",
    "+recap",
    "+roll/last",
    "help",
    "view",
    "contacts",
    "status",
    "weaponstatus",
    "scan",
    "range",
    "bearing",
];

/// The body of a declared-but-unproduced pane (interface plan I8), two rows
/// that fit the narrowest pane. Each names the PRODUCER that is missing,
/// never a cut number, so the pane says what would fill it and a reader can
/// tell "nothing yet" from "broken". The pane's title is its plain name.
/// (The grid pane's producer landed with S4: the `atlas.map` frame.)
pub const AWAITING_WEAPONS: &str = "awaiting a weapons-cycle\nprojection (plan \u{a7}9)";
/// The comms pane's declaration.
pub const AWAITING_COMMS: &str = "awaiting a station comms\nprojection (plan \u{a7}9)";

/// A screen's hotkey digit, `0..=9` (16b cut Q.4 took `0` for the cockpit).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ScreenId(pub u8);

/// The cockpit screen's digit (16b cut Q.4; doc 16 §5).
pub const COCKPIT_SCREEN: ScreenId = ScreenId(0);

/// A pane's stable name within its screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PaneId(pub &'static str);

/// How a pane's frame is styled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BorderRole {
    /// The default frame.
    Neutral,
    /// A unit or subject pane (cyan).
    Unit,
    /// In-character content (green).
    Ic,
    /// Out-of-character content (yellow).
    Ooc,
    /// Something demanding attention (red).
    Danger,
}

/// Where a pane sits: the main column, or a stacked left / right column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneSlot {
    /// The wide column that takes whatever is left.
    Main,
    /// A fixed-width column on the left.
    Left,
    /// A fixed-width column on the right.
    Right,
}

/// A renderer for a panel keyframe: the feed's JSON to rows.
pub type PanelRenderer = fn(&Value, &RenderCtx<'_>) -> PaneRows;

/// What a pane shows.
#[derive(Clone, Copy, Debug)]
pub enum PaneContent {
    /// The scrollback ring with its speaker gutter.
    Scrollback,
    /// The WHO rail.
    Who,
    /// A panel feed's latest keyframe, rendered by `render`.
    Panel {
        /// The feed name in the panel store.
        feed: &'static str,
        /// Keyframe JSON to rows.
        render: PanelRenderer,
    },
    /// A semantic view's latest keyframe (`OutputEvent::View`).
    View {
        /// The view name.
        view: &'static str,
        /// Keyframe JSON to rows.
        render: PanelRenderer,
    },
    /// The first `.sheet` keyframe in the store, walked generically.
    Sheet,
    /// The comsys channel list: the in-character channels (`ic`) or the
    /// out-of-character ones (interface plan I7).
    Channels {
        /// Which half of the list this pane shows.
        ic: bool,
    },
    /// Live and recalled messages of the highlighted channel.
    Backlog,
    /// The `.roll` feeds' ring.
    Rolls,
    /// Every `.vitals` frame as widgets (interface plan I6): tracks as boxes
    /// in threes with the level, pools as gauges, readouts with a condition
    /// mark.
    Vitals,
    /// A pane whose producer does not exist yet (interface plan I8): a
    /// declared, titled placeholder that names what is missing and degrades
    /// before every live pane. A `Panel` on an absent feed would instead
    /// wait on `(loading...)` forever, which is a lie.
    Placeholder(&'static str),
}

/// One pane of a screen.
#[derive(Clone, Copy, Debug)]
pub struct PaneSpec {
    /// The pane's name (unique within the screen).
    pub id: PaneId,
    /// Frame title.
    pub title: &'static str,
    /// Frame styling.
    pub role: BorderRole,
    /// Which column the pane lives in.
    pub slot: PaneSlot,
    /// What it shows.
    pub content: PaneContent,
    /// Below this width the pane is dropped (see [`Screen::degrade`]).
    pub min_w: u16,
    /// Below this height the pane is dropped.
    pub min_h: u16,
    /// Whether Up/Down move a highlighted row (a row pane) rather than
    /// scrolling text.
    pub selectable: bool,
}

/// A client-side action a key may perform.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalAction {
    /// Re-query the pane's feed.
    Refresh,
    /// Request backlog for the highlighted channel.
    Backlog,
    /// Empty the scrollback ring.
    ClearScrollback,
}

/// What a bound key does (D-I-18).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAction {
    /// A client toggle: focus, filter, refresh, overlay.
    Local(LocalAction),
    /// Enter input mode with this line ready; nothing is sent.
    Prefill(&'static str),
    /// Enter input mode with this prefix followed by the focused pane's
    /// highlighted row key (a mail index, a contact id); nothing is sent.
    PrefillRow(&'static str),
    /// Send now — read-only verbs only.
    Run(&'static str),
}

/// One legend entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyBinding {
    /// The bare key in nav mode (Alt+key in input mode).
    pub key: char,
    /// The legend word.
    pub label: &'static str,
    /// What it does.
    pub action: KeyAction,
}

/// One numbered screen.
#[derive(Clone, Debug)]
pub struct Screen {
    /// The hotkey digit.
    pub id: ScreenId,
    /// The tab-strip name.
    pub name: &'static str,
    /// Panes in focus order (the first is focused on entry).
    pub panes: Vec<PaneSpec>,
    /// The screen's own bindings, printed on the legend.
    pub keys: Vec<KeyBinding>,
    /// Panes to drop, in order, when the terminal is too small.
    pub degrade: Vec<PaneId>,
    /// Whether the room's one-letter exits are keys here (D-I-19; the
    /// scene screen).
    pub exits_are_keys: bool,
    /// Queries to submit when the screen is entered (`(name, args)`).
    pub on_enter: Vec<(&'static str, Value)>,
    /// Whether the Right column takes half the surplus width instead of
    /// staying at its panes' `max(min_w)` (interface plan I8).
    ///
    /// A rail — the WHO list, the channel list — is designed at a fixed
    /// width and reads worse stretched. A table is not: the tactical
    /// screens' before/after and journal panes carry whole rows, and at a
    /// fixed 34 they clipped their one job at every terminal size. Since
    /// 16b cut B.3 `draw_pane` wraps every panel row at its pane's edge, so a
    /// narrow Right column no longer loses words — it spends rows — and the
    /// share is what keeps a table row on ONE line where the terminal has the
    /// width. The screen declares which it is; `visible_panes` never reads
    /// this, so no survivor set depends on it.
    pub right_shares_surplus: bool,
    /// **The panel feed this screen needs to exist at all** (16b cut Q.4),
    /// mirroring the web's per-screen `gate`.
    ///
    /// `None` is a screen every viewer has. `Some(feed)` is a screen that
    /// exists only for a viewer whose server publishes `feed` AND who holds a
    /// frame of it (`TuiApp::screen_available`): its tab is not drawn and its
    /// digit is inert otherwise, so a server with no arena never grows a tenth
    /// tab and a bystander on one never sees a cockpit they do not sit in.
    pub gate: Option<&'static str>,
}

impl Screen {
    /// The binding for `key`, if the screen has one.
    #[must_use]
    pub fn binding(&self, key: char) -> Option<&KeyBinding> {
        self.keys.iter().find(|b| b.key == key)
    }

    /// The pane named `id`.
    #[must_use]
    pub fn pane(&self, id: PaneId) -> Option<&PaneSpec> {
        self.panes.iter().find(|p| p.id == id)
    }
}

/// The ten built-in screens: 1 scene, 2 sheet, 3 mail, 4 chan, 5 rolls,
/// 6 intel, 7 actions, 8 damage, 9 unit, 0 cockpit.
///
/// Screens 8 and 9 read one feed, `tactics.unit`, and name nothing in it:
/// every label, id and word on them comes out of the frame, so a ruleset that
/// publishes the feed gets both screens without a client change.
///
/// **The cockpit is LAST** (16b cut Q.4) and not first: [`super::app::TuiApp`]
/// opens on `screens.first()`, which must stay the scene, and a tab strip
/// reading `1 … 9 0` is a keyboard's order rather than a number's. It is the
/// one gated screen — [`Screen::gate`] — so a world with no arena never grows
/// a tenth tab.
#[must_use]
pub fn builtin_screens() -> Vec<Screen> {
    use super::render::{
        render_actions_rows, render_cockpit_contacts_rows, render_cockpit_damage_rows,
        render_cockpit_map_rows, render_cockpit_notices_rows, render_cockpit_status_rows,
        render_cockpit_weapons_rows, render_intel_rows, render_mail_rows, render_map_rows,
        render_scene_rows, render_unit_components_rows, render_unit_delta_rows,
        render_unit_journal_rows, render_unit_plant_rows, render_unit_sections_compact_rows,
        render_unit_sections_rows, render_view_rows,
    };
    let unit_query = || vec![(UNIT_FEED, Value::Object(serde_json::Map::new()))];
    vec![
        Screen {
            id: ScreenId(1),
            name: "scene",
            panes: vec![
                PaneSpec {
                    id: PaneId("scrollback"),
                    title: "rustmush",
                    role: BorderRole::Ic,
                    slot: PaneSlot::Main,
                    content: PaneContent::Scrollback,
                    // A reading width: below it the side panes go. At the
                    // 120-column floor the Right column's rails sit beside it.
                    min_w: 60,
                    min_h: 3,
                    selectable: false,
                },
                PaneSpec {
                    id: PaneId("who"),
                    title: "WHO",
                    role: BorderRole::Ooc,
                    slot: PaneSlot::Right,
                    content: PaneContent::Who,
                    min_w: 24,
                    min_h: 4,
                    selectable: false,
                },
                PaneSpec {
                    id: PaneId("scene"),
                    title: "scene",
                    role: BorderRole::Neutral,
                    slot: PaneSlot::Right,
                    content: PaneContent::Panel {
                        feed: "scenes",
                        render: render_scene_rows,
                    },
                    min_w: 24,
                    min_h: 4,
                    selectable: false,
                },
                PaneSpec {
                    id: PaneId("actions"),
                    title: "actions",
                    role: BorderRole::Neutral,
                    slot: PaneSlot::Right,
                    content: PaneContent::View {
                        view: "legal.actions",
                        render: render_actions_rows,
                    },
                    min_w: 24,
                    min_h: 4,
                    selectable: true,
                },
            ],
            keys: vec![
                KeyBinding {
                    key: 'l',
                    label: "look",
                    action: KeyAction::Run("look"),
                },
                KeyBinding {
                    key: 'i',
                    label: "inventory",
                    action: KeyAction::Run("inventory"),
                },
                KeyBinding {
                    key: 'p',
                    label: "page",
                    action: KeyAction::Prefill("page "),
                },
                KeyBinding {
                    key: 'r',
                    label: "roll",
                    action: KeyAction::Prefill("+roll "),
                },
                KeyBinding {
                    key: 'a',
                    label: "act",
                    action: KeyAction::PrefillRow("+act "),
                },
            ],
            degrade: vec![PaneId("actions"), PaneId("scene"), PaneId("who")],
            exits_are_keys: true,
            on_enter: vec![("legal.actions", Value::Object(serde_json::Map::new()))],
            // The WHO rail and its neighbours are rails, designed at their
            // own width: Main takes the surplus, as it always has.
            right_shares_surplus: false,
            gate: None,
        },
        Screen {
            id: ScreenId(2),
            name: "sheet",
            panes: vec![PaneSpec {
                id: PaneId("sheet"),
                title: "sheet",
                role: BorderRole::Unit,
                slot: PaneSlot::Main,
                content: PaneContent::Sheet,
                min_w: 20,
                min_h: 3,
                selectable: false,
            }],
            keys: vec![KeyBinding {
                key: 'r',
                label: "refresh",
                action: KeyAction::Local(LocalAction::Refresh),
            }],
            degrade: vec![],
            exits_are_keys: false,
            on_enter: vec![],
            right_shares_surplus: false,
            gate: None,
        },
        Screen {
            id: ScreenId(3),
            name: "mail",
            panes: vec![PaneSpec {
                id: PaneId("mail"),
                title: "@mail",
                role: BorderRole::Ooc,
                slot: PaneSlot::Main,
                content: PaneContent::Panel {
                    feed: "mail",
                    render: render_mail_rows,
                },
                min_w: 20,
                min_h: 3,
                selectable: true,
            }],
            keys: vec![
                KeyBinding {
                    key: 'r',
                    label: "refresh",
                    action: KeyAction::Local(LocalAction::Refresh),
                },
                KeyBinding {
                    key: 's',
                    label: "send",
                    action: KeyAction::Prefill("@mail/send "),
                },
                KeyBinding {
                    key: 'd',
                    label: "delete",
                    action: KeyAction::PrefillRow("@mail/delete "),
                },
            ],
            degrade: vec![],
            exits_are_keys: false,
            on_enter: vec![("mail.list", Value::Object(serde_json::Map::new()))],
            right_shares_surplus: false,
            gate: None,
        },
        Screen {
            id: ScreenId(4),
            name: "chan",
            panes: vec![
                PaneSpec {
                    id: PaneId("ic"),
                    title: "IC",
                    role: BorderRole::Ic,
                    slot: PaneSlot::Left,
                    content: PaneContent::Channels { ic: true },
                    min_w: 26,
                    min_h: 3,
                    selectable: true,
                },
                PaneSpec {
                    id: PaneId("ooc"),
                    title: "OOC",
                    role: BorderRole::Ooc,
                    slot: PaneSlot::Left,
                    content: PaneContent::Channels { ic: false },
                    min_w: 26,
                    min_h: 3,
                    selectable: true,
                },
                PaneSpec {
                    id: PaneId("backlog"),
                    title: "channel",
                    role: BorderRole::Neutral,
                    slot: PaneSlot::Main,
                    content: PaneContent::Backlog,
                    min_w: 20,
                    min_h: 3,
                    selectable: false,
                },
            ],
            keys: vec![
                KeyBinding {
                    key: 'r',
                    label: "refresh",
                    action: KeyAction::Local(LocalAction::Refresh),
                },
                KeyBinding {
                    key: 'b',
                    label: "backlog",
                    action: KeyAction::Local(LocalAction::Backlog),
                },
            ],
            degrade: vec![PaneId("ic"), PaneId("ooc")],
            exits_are_keys: false,
            on_enter: vec![("comsys.state", Value::Object(serde_json::Map::new()))],
            right_shares_surplus: false,
            gate: None,
        },
        Screen {
            id: ScreenId(5),
            name: "rolls",
            panes: vec![
                PaneSpec {
                    id: PaneId("rolls"),
                    title: "rolls",
                    role: BorderRole::Unit,
                    slot: PaneSlot::Main,
                    content: PaneContent::Rolls,
                    min_w: 20,
                    min_h: 3,
                    selectable: false,
                },
                PaneSpec {
                    id: PaneId("vitals"),
                    title: "vitals",
                    role: BorderRole::Unit,
                    slot: PaneSlot::Left,
                    content: PaneContent::Vitals,
                    min_w: 44,
                    min_h: 3,
                    selectable: false,
                },
            ],
            keys: vec![
                KeyBinding {
                    key: 'r',
                    label: "roll",
                    action: KeyAction::Prefill("+roll "),
                },
                KeyBinding {
                    key: 'h',
                    label: "history",
                    action: KeyAction::Run("+roll/last"),
                },
            ],
            degrade: vec![PaneId("vitals")],
            exits_are_keys: false,
            on_enter: vec![],
            right_shares_surplus: false,
            gate: None,
        },
        Screen {
            id: ScreenId(6),
            name: "intel",
            panes: vec![PaneSpec {
                id: PaneId("intel"),
                title: "+intel",
                role: BorderRole::Neutral,
                slot: PaneSlot::Main,
                content: PaneContent::Panel {
                    feed: "intel",
                    render: render_intel_rows,
                },
                min_w: 20,
                min_h: 3,
                selectable: false,
            }],
            keys: vec![KeyBinding {
                key: 'r',
                label: "refresh",
                action: KeyAction::Local(LocalAction::Refresh),
            }],
            degrade: vec![],
            exits_are_keys: false,
            on_enter: vec![("intel.list", Value::Object(serde_json::Map::new()))],
            right_shares_surplus: false,
            gate: None,
        },
        Screen {
            id: ScreenId(7),
            name: "actions",
            panes: vec![PaneSpec {
                id: PaneId("actions"),
                title: "+actions",
                role: BorderRole::Neutral,
                slot: PaneSlot::Main,
                content: PaneContent::View {
                    view: "legal.actions",
                    render: render_actions_rows,
                },
                min_w: 20,
                min_h: 3,
                selectable: true,
            }],
            keys: vec![
                KeyBinding {
                    key: 'r',
                    label: "refresh",
                    action: KeyAction::Local(LocalAction::Refresh),
                },
                KeyBinding {
                    key: 'a',
                    label: "act",
                    action: KeyAction::PrefillRow("+act "),
                },
            ],
            degrade: vec![],
            exits_are_keys: false,
            on_enter: vec![("legal.actions", Value::Object(serde_json::Map::new()))],
            right_shares_surplus: false,
            gate: None,
        },
        // 8 damage — the after-action report (interface plan I8): the unit as
        // the ruleset judged it, the newest `@unit/damage` before/after table
        // beside it, and the journal those tables come from.
        Screen {
            id: ScreenId(8),
            name: "damage",
            panes: vec![
                PaneSpec {
                    id: PaneId("sections"),
                    title: "sections",
                    role: BorderRole::Unit,
                    slot: PaneSlot::Main,
                    content: PaneContent::Panel {
                        feed: UNIT_FEED,
                        render: render_unit_sections_rows,
                    },
                    min_w: 40,
                    min_h: 6,
                    selectable: false,
                },
                PaneSpec {
                    id: PaneId("delta"),
                    title: "before / after",
                    role: BorderRole::Danger,
                    slot: PaneSlot::Right,
                    content: PaneContent::Panel {
                        feed: UNIT_FEED,
                        render: render_unit_delta_rows,
                    },
                    min_w: 34,
                    min_h: 5,
                    selectable: false,
                },
                PaneSpec {
                    id: PaneId("journal"),
                    title: "journal",
                    role: BorderRole::Neutral,
                    slot: PaneSlot::Right,
                    content: PaneContent::Panel {
                        feed: UNIT_FEED,
                        render: render_unit_journal_rows,
                    },
                    min_w: 34,
                    min_h: 4,
                    selectable: false,
                },
            ],
            keys: vec![
                // Refresh replays `on_enter`, which IS the keyframe
                // re-request; `@unit` is staff-gated and mutating, so it is
                // never a bare key — only a line waiting on Enter (D-I-18).
                KeyBinding {
                    key: 'r',
                    label: "refresh",
                    action: KeyAction::Local(LocalAction::Refresh),
                },
                KeyBinding {
                    key: 'd',
                    label: "damage",
                    action: KeyAction::Prefill("@unit/damage "),
                },
                KeyBinding {
                    key: 'u',
                    label: "show unit",
                    action: KeyAction::Prefill("@unit/show "),
                },
            ],
            degrade: vec![PaneId("journal"), PaneId("delta")],
            exits_are_keys: false,
            on_enter: unit_query(),
            // The before/after table is this screen's whole point and its
            // rows are whole lines: it takes the width the section map, which
            // is one short line per part, does not need.
            right_shares_surplus: true,
            gate: None,
        },
        // 9 unit — the unit at a glance. Four panes read the feed; the other
        // three are declared placeholders naming the projection each waits on
        // (plan §9), and they degrade before any live pane does.
        Screen {
            id: ScreenId(9),
            name: "unit",
            panes: vec![
                PaneSpec {
                    id: PaneId("sections"),
                    title: "sections",
                    role: BorderRole::Unit,
                    slot: PaneSlot::Main,
                    content: PaneContent::Panel {
                        feed: UNIT_FEED,
                        render: render_unit_sections_compact_rows,
                    },
                    min_w: 40,
                    min_h: 4,
                    selectable: false,
                },
                PaneSpec {
                    id: PaneId("components"),
                    title: "components",
                    role: BorderRole::Neutral,
                    slot: PaneSlot::Main,
                    content: PaneContent::Panel {
                        feed: UNIT_FEED,
                        render: render_unit_components_rows,
                    },
                    min_w: 40,
                    min_h: 4,
                    selectable: false,
                },
                PaneSpec {
                    id: PaneId("plant"),
                    title: "plant",
                    role: BorderRole::Unit,
                    slot: PaneSlot::Left,
                    content: PaneContent::Panel {
                        feed: UNIT_FEED,
                        render: render_unit_plant_rows,
                    },
                    min_w: 24,
                    min_h: 4,
                    selectable: false,
                },
                // S4: the geometry projection the grid pane awaited — the
                // `atlas.map` frame of the window the pilot stands on.
                PaneSpec {
                    id: PaneId("grid"),
                    title: "grid",
                    role: BorderRole::Neutral,
                    slot: PaneSlot::Left,
                    content: PaneContent::Panel {
                        feed: MAP_FEED,
                        render: render_map_rows,
                    },
                    min_w: 24,
                    min_h: 3,
                    selectable: false,
                },
                // The contacts table (D-I8-1): the I9 generic-view renderer
                // over the `tactics.contacts` keyframe; a row's Enter prefills
                // `+contact <id>`. Subscribed on entry only when the manifest
                // carries the feed (`screen_queries`), so a server without
                // the propagation layer never prints an unknown-query line.
                PaneSpec {
                    id: PaneId("contacts"),
                    title: "contacts",
                    role: BorderRole::Neutral,
                    slot: PaneSlot::Right,
                    content: PaneContent::Panel {
                        feed: CONTACTS_FEED,
                        render: render_view_rows,
                    },
                    min_w: 34,
                    min_h: 4,
                    selectable: true,
                },
                PaneSpec {
                    id: PaneId("weapons"),
                    title: "weapons",
                    role: BorderRole::Neutral,
                    slot: PaneSlot::Right,
                    content: PaneContent::Placeholder(AWAITING_WEAPONS),
                    min_w: 20,
                    min_h: 2,
                    selectable: false,
                },
                PaneSpec {
                    id: PaneId("comms"),
                    title: "comms",
                    role: BorderRole::Neutral,
                    slot: PaneSlot::Right,
                    content: PaneContent::Placeholder(AWAITING_COMMS),
                    min_w: 20,
                    min_h: 2,
                    selectable: false,
                },
            ],
            keys: vec![
                KeyBinding {
                    key: 'r',
                    label: "refresh",
                    action: KeyAction::Local(LocalAction::Refresh),
                },
                KeyBinding {
                    key: 'c',
                    label: "contacts",
                    action: KeyAction::Run("+contacts"),
                },
                KeyBinding {
                    key: 'd',
                    label: "damage",
                    action: KeyAction::Prefill("@unit/damage "),
                },
            ],
            degrade: vec![
                PaneId("comms"),
                PaneId("weapons"),
                PaneId("grid"),
                PaneId("contacts"),
                PaneId("plant"),
                PaneId("components"),
            ],
            exits_are_keys: false,
            on_enter: unit_query(),
            // The contacts table's own header is 49 columns wide; the compact
            // section map is one short line per section.
            right_shares_surplus: true,
            gate: None,
        },
        // 0 cockpit — doc 16 §5's full-screen surface (16b cut Q.4): the map
        // the pilot is flying over, what the platform is doing, what is left of
        // it, what it can shoot with and what it holds. Six panes over the one
        // `arena.cockpit` feed, which the arena pushes every pulse, plus the
        // `arena.map` terrain the map pane composes onto — pulled once, because
        // the ground does not move.
        //
        // The screen names nothing the frame does not: a contact is its
        // designator, its class word and its plot (doc 16 §1, §3.6).
        //
        // **16b cut AB.1** (16c `Q-AB1`, superseding cut B.3's eighty-column
        // layout, `B-10`): the widths are what the 120-column floor carries —
        // the Left column 34 (status over damage), the map 36, the Right column
        // 46 (weapons, contacts, notices) — and `34 + 36 + 46` is 116, so no
        // pane is dropped for WIDTH at the floor and the four columns over go
        // half to the map and half to the Right column (`right_shares_surplus`,
        // below). The Left column holds 32 characters inside its borders, where
        // no status row wraps; the Right column `44 + (w − 116) / 2`, 46 at
        // 120 and 66 at 160, where no weapon or contact row wraps. Every panel
        // row still wraps at its pane's edge (`TuiApp::draw_pane`, cut B.3), so
        // what is longer than its pane — a contact's SCAN row — costs a row,
        // never words. For HEIGHT the Left column wants `9 + 14` body rows and
        // the Right `6 + 6 + 5`, so the cockpit is whole from `COCKPIT_ROWS`
        // (30 terminal rows with the vitals gauge); below that the `degrade`
        // order is live and `damage` goes first, as cut B.3 built it.
        Screen {
            id: COCKPIT_SCREEN,
            name: "cockpit",
            panes: vec![
                PaneSpec {
                    id: PaneId("map"),
                    title: "map",
                    role: BorderRole::Unit,
                    slot: PaneSlot::Main,
                    content: PaneContent::Panel {
                        feed: COCKPIT_FEED,
                        render: render_cockpit_map_rows,
                    },
                    min_w: 36,
                    min_h: 6,
                    selectable: false,
                },
                // `min_w` 34 and `min_h` 7 (16b cut AB.1): the fullest block
                // the frame can send is seven rows, and at the pane's 32 inner
                // columns not one of them wraps:
                //   `Callsign` and an eight-letter callsign            17
                //   `power starting  HELD`                              20
                //   the speed row at a three-digit kph, `  DESTROYED`   32
                //     (`43 kph  heading 045°  DESTROYED` is 31;
                //     `IMMOBILE` is one less)
                //   `heat 100/100  OVERHEATED`                          24
                //   `receiver 100/100  100%`                            22
                //   `scope hostile`                                     13
                //   `Lock` an eight-letter designator, past a
                //     kilometre at 100%                                 27
                // The lock is the last row and the one a gunner reads before
                // FIRE, so the pane is never shorter than the block. Cut B.3's
                // `min_h` 10 was the same seven rows with three of them wrapped
                // in 20 columns.
                PaneSpec {
                    id: PaneId("status"),
                    title: "status",
                    role: BorderRole::Unit,
                    slot: PaneSlot::Left,
                    content: PaneContent::Panel {
                        feed: COCKPIT_FEED,
                        render: render_cockpit_status_rows,
                    },
                    min_w: 34,
                    min_h: 7,
                    selectable: false,
                },
                // `min_h` 12 (16b cut AB.1) is the shipped line tank whole at
                // the pane's 32 inner columns: `render_cockpit_damage_rows`
                // packs a section's facet cells onto rows under its condition
                // row, and the tank's hull (five facets) takes four rows, its
                // turret three, each track two and the engine one. Every
                // section of the stock roster is therefore on screen from
                // `COCKPIT_ROWS`, where cut B.3's 20 columns showed three of
                // the five at 120 by 34.
                PaneSpec {
                    id: PaneId("damage"),
                    title: "damage",
                    role: BorderRole::Danger,
                    slot: PaneSlot::Left,
                    content: PaneContent::Panel {
                        feed: COCKPIT_FEED,
                        render: render_cockpit_damage_rows,
                    },
                    min_w: 34,
                    min_h: 12,
                    selectable: false,
                },
                // The Right column's panes each declare its 46 (16b cut AB.1):
                // `draw_body` takes a column's `max(min_w)`. A contact row is at
                // most 43 characters at the widest shipped words (`unknown`,
                // `effective`), a four-digit range (the arena's diagonal is
                // 3,620 m) and a three-digit age, so at 44 inside it never
                // wraps; a weapon row (`machine_gun  12 s  1000/1000  100%`, 34)
                // has room to spare.
                PaneSpec {
                    id: PaneId("weapons"),
                    title: "weapons",
                    role: BorderRole::Danger,
                    slot: PaneSlot::Right,
                    content: PaneContent::Panel {
                        feed: COCKPIT_FEED,
                        render: render_cockpit_weapons_rows,
                    },
                    min_w: 46,
                    min_h: 4,
                    selectable: false,
                },
                PaneSpec {
                    id: PaneId("contacts"),
                    title: "contacts",
                    role: BorderRole::Neutral,
                    slot: PaneSlot::Right,
                    content: PaneContent::Panel {
                        feed: COCKPIT_FEED,
                        render: render_cockpit_contacts_rows,
                    },
                    min_w: 46,
                    min_h: 4,
                    selectable: true,
                },
                // **16b cut B.3** (`B-64`, 16c Q20): the frame's notices and
                // the pilot's own replies, composed from FIELDS and never from
                // a notice's `text` — `PanelStore::cockpit_notices`, a ring of
                // `COCKPIT_NOTICES` lines that starts each match clean. It used
                // to be the whole scrollback, which let channel chatter push a
                // hit off the pane; the scrollback on screen 1 still keeps every
                // sentence, and Q20 lets two screens word one event. A pilot on
                // this screen reads `Locking BA.` here, because every `Sys` line
                // the pilot is sent while seated rings too (not one answering
                // the app's own background queries).
                //
                // **16b cut AB.1** (16c `Q-AB3`) moved it from the Left column
                // to the foot of the Right one, nearest the box the pilot types
                // in: under a twelve-row damage pane it would have kept two
                // inner rows at 120 by 34, where the Right column's even split
                // gives it seven there (at 46 columns) and twelve at 160 by 48
                // (at 66) — room for a read key's multi-line answer and for most
                // of cut Y.4's catch-up report. It is not selectable: Tab
                // reaches it last now, and contacts is still the one pane a
                // highlight and `Enter` belong to.
                PaneSpec {
                    id: PaneId("notices"),
                    title: "notices",
                    role: BorderRole::Ic,
                    slot: PaneSlot::Right,
                    content: PaneContent::Panel {
                        feed: COCKPIT_FEED,
                        render: render_cockpit_notices_rows,
                    },
                    min_w: 46,
                    min_h: 3,
                    selectable: false,
                },
            ],
            // Every `Run` here is one of the seven cockpit verbs that only
            // read; the five that stage an intent are PREFILLS, so no bare key
            // ever steers, fires or locks — the player still presses Enter.
            keys: vec![
                KeyBinding {
                    key: 'k',
                    label: "lock",
                    action: KeyAction::Prefill("lock "),
                },
                KeyBinding {
                    key: 'f',
                    label: "fire",
                    action: KeyAction::Prefill("fire "),
                },
                KeyBinding {
                    key: 'h',
                    label: "heading",
                    action: KeyAction::Prefill("heading "),
                },
                // `m` for the help's own words — "make way at that speed" —
                // because `s` is the scan-a-row key below and `p` is pull.
                KeyBinding {
                    key: 'm',
                    label: "speed",
                    action: KeyAction::Prefill("speed "),
                },
                // **16b cut B.2** (`B-59`): `p`, pull — the panel asked for
                // again, and the map too when it has gone dark (the arm is
                // `screen_queries`, which drops a held map's ask). Not `g`,
                // which is the pane-jump key every screen inherits
                // (`JUMP_KEYS`). Placed right after `m`, fifth, inside the ten
                // bindings the legend keeps whole at the 120-column floor
                // (16b cut AB.1, `render::legend_line`). It reads and changes
                // nothing, so it is a `Local` action and not a `Run`.
                KeyBinding {
                    key: 'p',
                    label: "pull",
                    action: KeyAction::Local(LocalAction::Refresh),
                },
                // **16b cut B.3** (`B-63`): no `v`. `VIEW` answers a grid the
                // line client prints, which this screen never showed — the map
                // pane is the TUI's view — so the key ran a verb whose answer
                // went nowhere a pilot on screen 0 could read it. The read keys
                // below stay: their answers ring on the notices pane.
                KeyBinding {
                    key: 'c',
                    label: "contacts",
                    action: KeyAction::Run("contacts"),
                },
                KeyBinding {
                    key: 't',
                    label: "status",
                    action: KeyAction::Run("status"),
                },
                KeyBinding {
                    key: 'w',
                    label: "weapons",
                    action: KeyAction::Run("weaponstatus"),
                },
                KeyBinding {
                    key: 'r',
                    label: "range",
                    action: KeyAction::Run("range"),
                },
                KeyBinding {
                    key: 'b',
                    label: "bearing",
                    action: KeyAction::Run("bearing"),
                },
                KeyBinding {
                    key: 'n',
                    label: "scan",
                    action: KeyAction::Run("scan"),
                },
                KeyBinding {
                    key: 's',
                    label: "scan row",
                    action: KeyAction::PrefillRow("scan "),
                },
            ],
            // The map and the status go last: a cockpit with neither is not a
            // cockpit. Damage first, then the weapons, then the contacts.
            degrade: vec![PaneId("damage"), PaneId("weapons"), PaneId("contacts")],
            exits_are_keys: false,
            on_enter: vec![
                (COCKPIT_FEED, Value::Object(serde_json::Map::new())),
                (ARENA_MAP_FEED, Value::Object(serde_json::Map::new())),
            ],
            // The contacts pane carries whole rows — a designator, a class, a
            // bearing, a band, a range and an age — and a row that has the
            // width is read on one line rather than wrapped onto two (16b cut
            // B.3 wraps every panel row, so the surplus is now comfort, not the
            // difference between a row and a clipped one). At the 120-column
            // floor the surplus is four columns and the Right column holds 46
            // inside, where every contact row is one line already; what the
            // share buys is the SCAN row under a classified contact (49
            // characters for the shipped tank, 53 with every band letter),
            // which is a second line by design and wraps once until 126
            // columns (134 for 53) — the map is not squeezed for it (16b cut
            // AB.1). The Left column never shares the surplus.
            right_shares_surplus: true,
            gate: Some(COCKPIT_FEED),
        },
    ]
}

/// The binding-table probe (D-I-18, D-I-19): every `Run` verb is on the
/// read-only list, no screen rebinds a global or jump key, the screen
/// that gives exits keys never binds a reserved exit letter, ids are
/// unique and within the caps, every `degrade` entry names a pane.
///
/// # Errors
///
/// The first violation found, in words.
pub fn validate_screens(screens: &[Screen]) -> Result<(), String> {
    if screens.len() > MAX_SCREENS {
        return Err(format!("{} screens exceed MAX_SCREENS", screens.len()));
    }
    let mut seen_ids = Vec::new();
    for s in screens {
        if !(0..=9).contains(&s.id.0) {
            return Err(format!("screen {:?} is not a digit 0-9", s.name));
        }
        if seen_ids.contains(&s.id) {
            return Err(format!("screen digit {} is used twice", s.id.0));
        }
        seen_ids.push(s.id);
        if s.panes.len() > MAX_PANES_PER_SCREEN {
            return Err(format!("screen {} exceeds MAX_PANES_PER_SCREEN", s.name));
        }
        let mut pane_ids: Vec<PaneId> = Vec::new();
        for p in &s.panes {
            if pane_ids.contains(&p.id) {
                return Err(format!("screen {}: pane {} declared twice", s.name, p.id.0));
            }
            pane_ids.push(p.id);
        }
        for d in &s.degrade {
            if !pane_ids.contains(d) {
                return Err(format!(
                    "screen {}: degrade names unknown pane {}",
                    s.name, d.0
                ));
            }
        }
        let mut keys: Vec<char> = Vec::new();
        for b in &s.keys {
            if GLOBAL_KEYS.contains(&b.key) || JUMP_KEYS.contains(&b.key) {
                return Err(format!(
                    "screen {}: {:?} rebinds a global key",
                    s.name, b.key
                ));
            }
            if s.exits_are_keys && RESERVED_EXIT_KEYS.contains(&b.key) {
                return Err(format!(
                    "screen {}: {:?} is reserved for exits",
                    s.name, b.key
                ));
            }
            if keys.contains(&b.key) {
                return Err(format!("screen {}: {:?} bound twice", s.name, b.key));
            }
            keys.push(b.key);
            if let KeyAction::Run(cmd) = b.action {
                let verb = cmd.split_whitespace().next().unwrap_or("");
                if !RUN_ALLOWLIST
                    .iter()
                    .any(|allowed| allowed.eq_ignore_ascii_case(verb))
                {
                    return Err(format!(
                        "screen {}: {:?} runs {cmd:?}, not a read-only verb",
                        s.name, b.key
                    ));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_pass_the_probe_and_the_scene_screen_reserves_exit_letters() {
        let screens = builtin_screens();
        validate_screens(&screens).expect("built-ins are valid");
        let scene = screens.iter().find(|s| s.id == ScreenId(1)).unwrap();
        assert!(scene.exits_are_keys);
        for b in &scene.keys {
            assert!(!RESERVED_EXIT_KEYS.contains(&b.key), "{:?}", b.key);
        }
        // Another screen may use those letters.
        let mail = screens.iter().find(|s| s.id == ScreenId(3)).unwrap();
        assert!(mail.binding('s').is_some() && mail.binding('d').is_some());
    }

    /// I8: ten screens is the cap (16b cut Q.4 took `0` for the cockpit), and
    /// the two tactical ones sit inside every bound the probe enforces — pane
    /// count, unique pane ids, a `degrade` order that names only panes it has,
    /// and no `Run` off the read-only list (`@unit` is staff-gated and
    /// mutating: it is only ever a prefill).
    #[test]
    fn ten_screens_validate_and_the_tactical_two_stay_inside_every_cap() {
        let screens = builtin_screens();
        assert_eq!(screens.len(), 10);
        validate_screens(&screens).expect("ten built-ins are valid");
        assert_eq!(
            screens.len(),
            MAX_SCREENS,
            "the hotkey digits are now spent"
        );
        for id in [ScreenId(8), ScreenId(9)] {
            let screen = screens.iter().find(|s| s.id == id).expect("a screen");
            assert!(screen.panes.len() <= MAX_PANES_PER_SCREEN);
            for d in &screen.degrade {
                assert!(
                    screen.pane(*d).is_some(),
                    "{} degrades {}",
                    screen.name,
                    d.0
                );
            }
            assert_eq!(
                screen.on_enter,
                vec![(UNIT_FEED, Value::Object(serde_json::Map::new()))],
                "the TUI asks for the default subject, never a named one"
            );
            assert!(!screen.exits_are_keys);
            for b in &screen.keys {
                assert!(
                    !matches!(b.action, KeyAction::Run(cmd) if cmd.starts_with('@')),
                    "{:?} runs a mutating verb",
                    b.key
                );
            }
        }
        // Every placeholder pane degrades before any live one (plan §3.8).
        let unit = screens.iter().find(|s| s.id == ScreenId(9)).unwrap();
        let placeholders: Vec<PaneId> = unit
            .panes
            .iter()
            .filter(|p| matches!(p.content, PaneContent::Placeholder(_)))
            .map(|p| p.id)
            .collect();
        assert_eq!(
            placeholders.len(),
            2,
            "weapons and comms await; the grid is produced (S4)"
        );
        let grid = unit.pane(PaneId("grid")).expect("grid pane");
        assert!(matches!(grid.content, PaneContent::Panel { feed, .. } if feed == MAP_FEED));
        // The contacts pane is live (D-I8-1): the generic-view renderer over
        // its own feed, selectable so Enter prefills the row's `+contact`.
        let contacts = unit.pane(PaneId("contacts")).expect("contacts pane");
        assert!(
            matches!(contacts.content, PaneContent::Panel { feed, .. } if feed == CONTACTS_FEED)
        );
        assert!(contacts.selectable);
        let live_first = unit
            .degrade
            .iter()
            .position(|d| !placeholders.contains(d))
            .expect("some live pane degrades too");
        assert_eq!(live_first, placeholders.len());
    }

    /// Look-back LB-16: [`RUN_ALLOWLIST`] is one of two hand-kept copies, with
    /// nothing that crossed them. Both are now pinned to one committed
    /// fixture, this side in Rust and the web side in a vitest, so a verb
    /// added to one and not the other fails a build rather than diverging in
    /// silence. `@mail` and `+orders` are off the list because both WRITE.
    #[test]
    fn the_run_allowlist_matches_the_committed_fixture_and_writes_nothing() {
        let fixture: Vec<String> =
            serde_json::from_str(include_str!("../../../../web/src/lib/run-allowlist.json"))
                .expect("web/src/lib/run-allowlist.json is a JSON array of verbs");
        let mut sorted: Vec<String> = RUN_ALLOWLIST.iter().map(|v| (*v).to_string()).collect();
        sorted.sort();
        assert_eq!(sorted.len(), RUN_ALLOWLIST.len(), "a verb is listed twice");
        assert_eq!(sorted, fixture, "the TUI and web copies have drifted");
        for writer in ["@mail", "+orders"] {
            assert!(
                !fixture.contains(&writer.to_string()),
                "{writer} mutates and may not ride a bare key"
            );
        }
        assert!(
            !fixture.iter().any(|v| v.starts_with('@')),
            "no admin verb is read-only enough for a bare key"
        );
        // Every verb a built-in actually binds is on the list; the rest of the
        // list is reserved for the screens a ruleset declares.
        let bound: Vec<&str> = builtin_screens()
            .iter()
            .flat_map(|s| s.keys.iter())
            .filter_map(|b| match b.action {
                KeyAction::Run(cmd) => cmd.split_whitespace().next(),
                _ => None,
            })
            .collect();
        // 16b cut Q.4 bound the cockpit's seven read verbs; the five that stage
        // an arena intent are prefills and are not on the list at all. 16b cut
        // B.3 (`B-63`) unbound `view`, whose grid the TUI never showed; the
        // verb stays on the list, which is the web's too and is reserved.
        assert_eq!(
            bound,
            [
                "look",
                "inventory",
                "+roll/last",
                "+contacts",
                "contacts",
                "status",
                "weaponstatus",
                "range",
                "bearing",
                "scan",
            ]
        );
        for verb in &bound {
            assert!(
                RUN_ALLOWLIST.contains(verb),
                "{verb} is bound but not listed"
            );
        }
        for staged in ["startup", "heading", "speed", "lock", "fire"] {
            assert!(
                !RUN_ALLOWLIST.contains(&staged),
                "{staged} stages an arena intent and may not ride a bare key"
            );
        }
    }

    /// **The cockpit screen** (16b cut Q.4; doc 16 §5): it is LAST so the app
    /// still opens on the scene, it is the one gated screen, its map and status
    /// survive every degrade, and no key on it sends an order.
    #[test]
    fn the_cockpit_screen_is_last_gated_and_orders_nothing_from_a_bare_key() {
        let screens = builtin_screens();
        assert_eq!(
            screens.first().map(|s| s.id),
            Some(ScreenId(1)),
            "the app opens on `screens.first()`, which must stay the scene"
        );
        let cockpit = screens.last().expect("a last screen");
        assert_eq!(cockpit.id, COCKPIT_SCREEN);
        assert_eq!(cockpit.name, "cockpit");
        assert_eq!(cockpit.gate, Some(COCKPIT_FEED));
        assert!(
            screens.iter().filter(|s| s.gate.is_some()).count() == 1,
            "the cockpit is the one gated screen"
        );
        assert_eq!(
            cockpit.on_enter,
            vec![
                (COCKPIT_FEED, Value::Object(serde_json::Map::new())),
                (ARENA_MAP_FEED, Value::Object(serde_json::Map::new())),
            ],
            "the frame and the terrain it is drawn on"
        );
        assert!(cockpit.panes.len() <= MAX_PANES_PER_SCREEN);
        // Damage, then weapons, then contacts; the map and the status are not
        // in the order at all, so nothing can drop them.
        assert_eq!(
            cockpit.degrade,
            vec![PaneId("damage"), PaneId("weapons"), PaneId("contacts")]
        );
        for kept in ["map", "status"] {
            assert!(
                !cockpit.degrade.contains(&PaneId(kept)),
                "{kept} must survive every degrade"
            );
        }
        for d in &cockpit.degrade {
            assert!(cockpit.pane(*d).is_some(), "degrade names {}", d.0);
        }
        // Every pane reads the one feed — the notices pane too since 16b cut
        // B.3 (`B-64`): it is the frame's notices and the pilot's replies,
        // not the scrollback.
        for pane in &cockpit.panes {
            match pane.content {
                PaneContent::Panel { feed, .. } => assert_eq!(feed, COCKPIT_FEED),
                _ => panic!("the cockpit declares only panel panes"),
            }
        }
        let notices = cockpit.pane(PaneId("notices")).expect("a notices pane");
        assert!(!notices.selectable, "the notices pane is read, not picked");
        assert_eq!(notices.role, BorderRole::Ic);
        // 16b cut AB.1 (16c `Q-AB1`, superseding cut B.3's `B-10`): the Left
        // column is status over damage at 34, the Right column weapons,
        // contacts and notices at 46, in that order top to bottom, and the map
        // 36 between them.
        let pane = |id: &'static str| cockpit.pane(PaneId(id)).expect("a cockpit pane");
        assert_eq!(pane("map").slot, PaneSlot::Main);
        assert_eq!(pane("map").min_w, 36);
        for id in ["status", "damage"] {
            assert_eq!(pane(id).slot, PaneSlot::Left, "{id}");
            assert_eq!(pane(id).min_w, 34, "{id}");
        }
        for id in ["weapons", "contacts", "notices"] {
            assert_eq!(pane(id).slot, PaneSlot::Right, "{id}");
            assert_eq!(pane(id).min_w, 46, "{id}");
        }
        let right: Vec<&str> = cockpit
            .panes
            .iter()
            .filter(|p| p.slot == PaneSlot::Right)
            .map(|p| p.id.0)
            .collect();
        assert_eq!(right, ["weapons", "contacts", "notices"]);
        assert_eq!(pane("status").min_h, 7);
        assert_eq!(pane("damage").min_h, 12);
        assert_eq!(pane("notices").min_h, 3);
        // The derivation: the three columns fit side by side at the floor, and
        // from `COCKPIT_ROWS` both side columns hold every pane with its border
        // under the chrome (tab strip, status line, the vitals gauge, the input
        // box and the legend).
        let (left_w, map_w, right_w) = (pane("status").min_w, pane("map").min_w, 46);
        assert!(left_w + map_w + right_w <= FLOOR_COLS);
        let body = COCKPIT_ROWS - (2 + 1 + super::super::app::INPUT_H + 1);
        assert!((pane("status").min_h + 2) + (pane("damage").min_h + 2) <= body);
        assert!(
            (pane("weapons").min_h + 2)
                + (pane("contacts").min_h + 2)
                + (pane("notices").min_h + 2)
                <= body
        );
        assert!(
            cockpit
                .pane(PaneId("contacts"))
                .is_some_and(|p| p.selectable),
            "Enter prefills the highlighted contact's LOCK"
        );
        // Nothing that changes the world rides a bare key: the five staging
        // verbs are prefills, and `s` prefills a row's SCAN.
        for binding in &cockpit.keys {
            match binding.action {
                KeyAction::Run(cmd) => assert!(
                    [
                        "contacts",
                        "status",
                        "weaponstatus",
                        "scan",
                        "range",
                        "bearing"
                    ]
                    .contains(&cmd),
                    "{cmd:?} is not one of the six read verbs the screen binds"
                ),
                KeyAction::Prefill(line) => assert!(
                    ["lock ", "fire ", "heading ", "speed "].contains(&line),
                    "{line:?}"
                ),
                KeyAction::PrefillRow(line) => assert_eq!(line, "scan "),
                // 16b cut B.2: the one local action is `p`, pull, which only
                // asks the server again for what the screen reads.
                KeyAction::Local(action) => assert!(
                    binding.key == 'p' && action == LocalAction::Refresh,
                    "the cockpit's one local action is `p` pull: {:?} {action:?}",
                    binding.key
                ),
            }
        }
        // …and `p` sits right after `m`, so the legend cut at the floor
        // shows it.
        let order: Vec<char> = cockpit.keys.iter().map(|b| b.key).collect();
        let m = order
            .iter()
            .position(|k| *k == 'm')
            .expect("an `m` speed key");
        assert_eq!(order.get(m + 1), Some(&'p'), "{order:?}");
        assert!(
            !JUMP_KEYS.contains(&'p') && !GLOBAL_KEYS.contains(&'p'),
            "`p` is on neither reserved list"
        );
        // 16b cut B.3 (`B-63`): `v` ran a `VIEW` whose grid this screen never
        // showed, so it is gone.
        assert!(cockpit.binding('v').is_none(), "{order:?}");
    }

    /// The id range moved with the tenth screen: `0` is a digit now and `10`
    /// still is not.
    #[test]
    fn the_digit_range_takes_zero_and_stops_at_nine() {
        let mut screens = builtin_screens();
        if let Some(screen) = screens.first_mut() {
            screen.id = ScreenId(10);
        }
        let err = validate_screens(&screens).unwrap_err();
        assert!(err.contains("is not a digit 0-9"), "{err}");
        let mut screens = builtin_screens();
        let eleventh = screens.first().cloned().expect("a screen");
        screens.push(eleventh);
        assert!(
            validate_screens(&screens)
                .unwrap_err()
                .contains("exceed MAX_SCREENS")
        );
    }

    #[test]
    fn a_mutating_run_or_a_reserved_letter_fails_the_probe() {
        let mut screens = builtin_screens();
        // Any mutating verb will do; this one is real, so the fixture cannot
        // outlive the verb it names the way `+fire a` did (16b cut T, D-16-20).
        screens[0].keys.push(KeyBinding {
            key: 'f',
            label: "focus",
            action: KeyAction::Run("+focus #1/pilot"),
        });
        let err = validate_screens(&screens).unwrap_err();
        assert!(err.contains("not a read-only verb"), "{err}");

        let mut screens = builtin_screens();
        screens[0].keys.push(KeyBinding {
            key: 'n',
            label: "new",
            action: KeyAction::Prefill("@dig "),
        });
        let err = validate_screens(&screens).unwrap_err();
        assert!(err.contains("reserved for exits"), "{err}");

        let mut screens = builtin_screens();
        screens[1].keys.push(KeyBinding {
            key: '?',
            label: "help",
            action: KeyAction::Local(LocalAction::Refresh),
        });
        assert!(
            validate_screens(&screens)
                .unwrap_err()
                .contains("global key")
        );

        let mut screens = builtin_screens();
        screens[1].id = ScreenId(1);
        assert!(
            validate_screens(&screens)
                .unwrap_err()
                .contains("used twice")
        );
    }
}
