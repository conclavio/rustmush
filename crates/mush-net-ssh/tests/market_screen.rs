//! **The market screen** (17g cut C3b; the C3 plan §2.4, §5): the TUI's third lettered
//! screen, `m`, gated on `mind.market` — the counter's listings on the left and the actor's own
//! holdings on the right, drawn from the two keyframes 17g C3a's queries answer. Its keys open
//! forms and send nothing; Enter inside a form sends exactly one typed `MindRequest::Market`
//! (never a `+minds` line) under an id from the desk's own range, `1 << 40` up to
//! `MAX_VIEW_WIRE_INTEGER`; the replies (`Staged`, `Shown`, `Error`) drive the form and the
//! status line with the server's own bytes.
//!
//! The keyframes here are written in exactly the JSON shapes `tacsim::minds::market_commands::
//! query` builds (`market_keyframe`, `listing_row`, `own_strip`, `holdings_keyframe`,
//! `owner_row`, `paid_row`), with fields the projection never sends planted beside them: the
//! decoder must ignore them and the screen must never paint them.

use mush_ansi::AnsiTheme;
use mush_net_ssh::tui::panels::PanelStore;
use mush_net_ssh::tui::render::{
    HOLDINGS_COLUMN_STARTS, MARKET_COLUMN_STARTS, RenderCtx, render_holdings_rows,
    render_market_rows,
};
use mush_net_ssh::tui::screen::{
    CHART_SCREEN, FLOOR_COLS, MARKET_SCREEN, QUARRY_SCREEN, builtin_screens, validate_screens,
};
use mush_net_ssh::tui::{InputResult, TuiApp, TuiManifest, frame_text};
use mush_proto::{
    ClientCaps, ClientKind, ColorDepth, InvokeRequest, InvokeResult, MAX_VIEW_WIRE_INTEGER,
    MarketAction, MarketListingForm, MindRequest, MindResponse, ObjRef, OutputEvent,
    PanelFrameKind,
};
use serde_json::{Value, json};

/// The clock every test reads (Unix seconds).
const NOW: i64 = 14 * 3600 + 30 * 60;
/// The desk's first id (`tui::market::FIRST_ID`).
const FIRST_ID: u64 = 1 << 40;
/// A publication token, an account id and a digest, planted where the projection never puts
/// them.
const TOKEN: &str = "f00dcafef00dcafef00dcafef00dcafef00dcafef00dcafef00dcafef00dcafe";
const ACCOUNT: &str = "acct-7f3a9e";
const DIGEST: &str = "sha256:9b2d4e6f8a0c1e3f5a7b9c1d3e5f7a9b";

fn caps(width: u16, height: u16) -> ClientCaps {
    ClientCaps {
        kind: ClientKind::Ssh,
        color: ColorDepth::Ansi16,
        width,
        height,
        utf8: true,
        structured: false,
    }
}

fn app_with(panels: &[&str]) -> TuiApp {
    let mut app = TuiApp::new(caps(120, 34));
    app.set_clock(|| NOW);
    app.set_manifest(TuiManifest {
        panels: panels.iter().map(|p| (*p).to_owned()).collect(),
        character: Some("Nick".into()),
        room: None,
    });
    app
}

fn keyframe(feed: &str, data: Value) -> OutputEvent {
    OutputEvent::Panel {
        feed: feed.into(),
        kind: PanelFrameKind::Keyframe,
        data,
    }
}

/// A manifest that publishes the market.
const MARKET: [&str; 3] = ["mind.market", "mind.holdings", "mail"];

/// One page of two listings: `0a1b2c3d` (transferable, with a live quote of the actor's) and
/// `99aa77bb` (not resaleable, derived, with the actor's purchase waiting), and the next page's
/// cursor.
fn market_page() -> Value {
    json!({
        "schema": { "id": "mind.market", "version": 1 },
        "open": true,
        "notice": null,
        "rows": [
            {
                "code": "0a1b2c3d", "name": "Drift Kestrel", "class": "II", "form": "open",
                "transferable": true, "term_seconds": 2_592_000, "grace_seconds": 604_800,
                "price_minor": "125000", "price_text": "1250cr", "levy_permille": 50,
                "venue_permille": 20, "parent_royalty_permille": 0, "derived": false,
                "terms_line": "One seat for 30 IC days plus 7 days of Grace; the licence can be resold.",
                "split_line": "Price 1250cr: Registry 5%, venue 2%, charter owner the rest.",
                "publication": TOKEN, "seller_account": ACCOUNT, "digest": DIGEST
            },
            {
                "code": "99aa77bb", "name": "Long Watch", "class": "III", "form": "open-resale",
                "transferable": false, "term_seconds": 2_592_000, "grace_seconds": 604_800,
                "price_minor": "9950", "price_text": "99.50cr", "levy_permille": 50,
                "venue_permille": 0, "parent_royalty_permille": 100, "derived": true,
                "terms_line": "One seat for 30 IC days plus 7 days of Grace; the licence cannot be resold.",
                "split_line": "Price 99.50cr: Registry 5%, parents 10%, charter owner the rest.",
                "protocol": TOKEN
            }
        ],
        "after": "99aa77bb",
        "own": {
            "quotes": [ { "code": "0a1b2c3d", "price_minor": "125000", "price_text": "1250cr",
                          "expires_in_seconds": 240, "expired": false, "operation": TOKEN } ],
            "waiting": [ { "code": "99aa77bb", "purchase": TOKEN } ]
        },
        "account": ACCOUNT
    })
}

/// The owner section: filed mind 7, its charter for sale; an unknown action word the decoder
/// drops.
fn owner_page() -> Value {
    json!({
        "schema": { "id": "mind.holdings", "version": 1 },
        "open": true,
        "notice": null,
        "section": "owner",
        "rows": [
            { "kind": "owner", "filed": 7, "name": "Drift Kestrel", "dependent": false,
              "phase": { "state": "active", "days_left": 30 }, "selling": false,
              "actions": ["publish", "withdraw", "charter", "someday"],
              "charter": 4_242, "owner_licence": 17, "digest": DIGEST }
        ],
        "after": null
    })
}

/// The paid section: licence 41 with a renewal offer waiting, and the House Pin.
fn paid_page() -> Value {
    json!({
        "schema": { "id": "mind.holdings", "version": 1 },
        "open": true,
        "notice": null,
        "section": "paid",
        "rows": [
            { "kind": "paid", "licence": 41, "house": false, "name": "Kestrel Line", "form": "open",
              "transferable": true, "phase": { "state": "grace", "days_left": 4 }, "revoked": false,
              "selling": false, "renewal": "offered",
              "actions": ["renew_pay", "renew_decline", "resell"],
              "renewal_operation": TOKEN, "seller": ACCOUNT },
            { "kind": "paid", "licence": 42, "house": true, "name": "House Pin", "form": "sealed",
              "transferable": false, "phase": { "state": "active", "days_left": 9 },
              "revoked": false, "selling": false, "renewal": null, "actions": ["renew"] }
        ],
        "after": 42
    })
}

/// A page that covers the buy form's other defaults: `c0ffee00` with nothing of the actor's,
/// `deadbeef` with the actor's quote expired, and a waiting purchase on `12345678`, which this
/// page does not list (its listing withdrawn since it was paid for).
fn third_page() -> Value {
    json!({
        "schema": { "id": "mind.market", "version": 1 },
        "open": true,
        "notice": null,
        "rows": [
            {
                "code": "c0ffee00", "name": "Quiet Hour", "class": "I", "form": "sealed",
                "transferable": true, "term_seconds": 2_592_000, "grace_seconds": 604_800,
                "price_minor": "5000", "price_text": "50cr", "levy_permille": 50,
                "venue_permille": 0, "parent_royalty_permille": 0, "derived": false,
                "terms_line": "One seat for 30 IC days plus 7 days of Grace; the licence can be resold.",
                "split_line": "Price 50cr: Registry 5%, charter owner the rest."
            },
            {
                "code": "deadbeef", "name": "Old Lamp", "class": "II", "form": "open",
                "transferable": true, "term_seconds": 2_592_000, "grace_seconds": 604_800,
                "price_minor": "7000", "price_text": "70cr", "levy_permille": 50,
                "venue_permille": 0, "parent_royalty_permille": 0, "derived": false,
                "terms_line": "One seat for 30 IC days plus 7 days of Grace; the licence can be resold.",
                "split_line": "Price 70cr: Registry 5%, charter owner the rest."
            }
        ],
        "after": null,
        "own": {
            "quotes": [ { "code": "deadbeef", "price_minor": "7000", "price_text": "70cr",
                          "expires_in_seconds": 0, "expired": true } ],
            "waiting": [ { "code": "12345678" } ]
        }
    })
}

/// Away from a counter (C2's `COUNTER` notice, no rows) with a purchase in escrow.
fn away_page() -> Value {
    json!({
        "schema": { "id": "mind.market", "version": 1 },
        "open": true,
        "notice": "You are not at a market counter.",
        "rows": [],
        "after": null,
        "own": { "quotes": [], "waiting": [ { "code": "12345678" } ] }
    })
}

/// An app on the market screen with both pages held.
fn on_the_market(holdings: Value) -> TuiApp {
    let mut app = app_with(&MARKET);
    let _ = app.feed_input(b"\x1bm");
    assert_eq!(app.active_screen(), MARKET_SCREEN);
    let _ = app.apply_event(&keyframe("mind.market", market_page()));
    let _ = app.apply_event(&keyframe("mind.holdings", holdings));
    app
}

/// The one typed market request a burst sent, with its id.
fn one_market(r: &InputResult) -> (u64, MarketAction) {
    assert!(r.submit.is_empty(), "no command text: {:?}", r.submit);
    let [(id, request)] = r.invokes.as_slice() else {
        panic!("exactly one typed request: {:?}", r.invokes);
    };
    request.validate().expect("the request passes validate");
    let InvokeRequest::Mind(MindRequest::Market { action }) = request else {
        panic!("a market request: {request:?}");
    };
    assert!(
        (FIRST_ID..=MAX_VIEW_WIRE_INTEGER).contains(id),
        "the desk's id range: {id}"
    );
    (*id, action.clone())
}

/// A reply to `id`.
fn reply(app: &mut TuiApp, id: u64, response: MindResponse) -> Vec<(String, Value)> {
    app.apply_event(&OutputEvent::InvokeResult {
        id,
        result: InvokeResult::Mind(response),
    })
}

fn refused(app: &mut TuiApp, id: u64) {
    let _ = reply(
        app,
        id,
        MindResponse::Error {
            code: "refused".into(),
            message: "Nothing changed.".into(),
        },
    );
}

/// **The thirteenth screen sits on the letter `m`** (17g cut C3b): thirteen built-ins
/// validate; Alt+m on a manifest with `mind.market` switches and asks the first listings page
/// and the owner section — the holdings query with the section it requires — and nothing
/// else of the market; Alt+m is inert where the manifest lacks the feed; on the chart and the
/// quarry `m` stays the screen's own key (the flat map, the rig's `mine`).
#[test]
fn thirteen_screens_validate_and_the_market_is_lettered_m() {
    let screens = builtin_screens();
    validate_screens(&screens).expect("the built-ins validate");
    assert_eq!(screens.len(), 13);
    assert_eq!(MARKET_SCREEN.0, b'm');
    let mut app = app_with(&MARKET);
    let r = app.feed_input(b"\x1bm");
    assert_eq!(app.active_screen(), MARKET_SCREEN);
    assert!(r.invokes.is_empty() && r.submit.is_empty());
    let market: Vec<&(String, Value)> = r
        .queries
        .iter()
        .filter(|(n, _)| n.starts_with("mind."))
        .collect();
    assert_eq!(
        market,
        [
            &("mind.market".to_owned(), json!({ "after": null })),
            &(
                "mind.holdings".to_owned(),
                json!({ "section": "owner", "after": null })
            ),
        ]
    );
    let frame = frame_text(&mut app, 160, 34);
    assert!(
        frame.contains(" m market "),
        "the strip names the letter:\n{frame}"
    );

    let mut bare = app_with(&["mail"]);
    let r = bare.feed_input(b"\x1bm");
    assert!(r.queries.is_empty(), "{:?}", r.queries);
    assert_ne!(bare.active_screen(), MARKET_SCREEN);

    let mut chart = app_with(&["sky.chart", "mind.market", "mind.holdings"]);
    let _ = chart.feed_input(b"\x1bc");
    assert_eq!(chart.active_screen(), CHART_SCREEN);
    let r = chart.feed_input(b"\x1bm");
    assert_eq!(chart.active_screen(), CHART_SCREEN, "the chart keeps `m`");
    assert_eq!(r.submit, ["+chart/map"]);
    let mut quarry = app_with(&["site.map", "mind.market", "mind.holdings"]);
    let _ = quarry.feed_input(b"\x1bq");
    assert_eq!(quarry.active_screen(), QUARRY_SCREEN);
    let _ = quarry.feed_input(b"\x1bm");
    assert_eq!(
        quarry.active_screen(),
        QUARRY_SCREEN,
        "the quarry keeps `m`"
    );
    assert_eq!(quarry.input_text(), "+tv mine");
}

/// **The rows fit the floor and leak nothing** (17g cut C3b; the C3 plan §2.4, §2.6): at
/// 120×34 the listings pane (68 columns inside) keeps CODE to PRICE and drops TERM+GRACE and
/// FLAGS whole; at 160×44 (108 inside) it keeps every column; the holdings pane (48 inside at
/// any width) keeps NUMBER to NAME. Every row the renderers give is at most its pane's width —
/// so `draw_pane` never wraps one — and starts its columns where the plan's header puts them.
/// The token, the account id and the digest planted in extra fields are on no frame; neither
/// is any `+minds` text, since no row prefills a line.
#[test]
fn market_rows_at_the_floor_fit_and_leak_nothing() {
    let theme = AnsiTheme::default();
    let store = PanelStore::new();
    let ctx = |pane: Option<(u16, u16)>| RenderCtx {
        theme: &theme,
        depth: ColorDepth::Ansi16,
        utf8: true,
        store: &store,
        selected_channel: None,
        pane,
        picture: None,
    };
    let text = |rows: &mush_net_ssh::tui::render::PaneRows| -> Vec<String> {
        rows.rows.iter().map(|r| r.text()).collect()
    };
    // The listings pane at the floor and at 160.
    for (inner, keeps_term) in [(68_u16, false), (108, true)] {
        let rows = render_market_rows(&market_page(), &ctx(Some((inner, 20))));
        let lines = text(&rows);
        for line in &lines {
            assert!(
                line.chars().count() <= usize::from(inner),
                "{inner}: {line:?}"
            );
        }
        let header = lines.first().expect("a header");
        assert_eq!(header.find("CODE"), Some(MARKET_COLUMN_STARTS[0]));
        assert_eq!(header.find("PRICE"), Some(MARKET_COLUMN_STARTS[4]));
        assert_eq!(header.contains("TERM+GRACE"), keeps_term, "{header:?}");
        assert_eq!(header.contains("FLAGS"), keeps_term, "{header:?}");
        // The own strip under the header, then the rows keyed by code and prefilling nothing.
        assert_eq!(
            lines.get(1).map(String::as_str),
            Some("quote 0a1b2c3d 1250cr 240s")
        );
        assert_eq!(lines.get(2).map(String::as_str), Some("waiting 99aa77bb"));
        let keyed: Vec<(&str, bool)> = rows
            .rows
            .iter()
            .filter(|r| r.selectable)
            .map(|r| (r.row_key.as_deref().unwrap_or(""), r.prefill.is_none()))
            .collect();
        assert_eq!(keyed, [("0a1b2c3d", true), ("99aa77bb", true)]);
        let row = lines.get(3).expect("the first listing");
        assert!(row.starts_with("0a1b2c3d  Drift Kestrel"), "{row:?}");
        assert!(row.contains("1250cr"), "the server's price text: {row:?}");
        if keeps_term {
            assert_eq!(row.find("30+7"), Some(MARKET_COLUMN_STARTS[5]), "{row:?}");
            let derived = lines.get(4).expect("the second listing");
            assert!(derived.ends_with("no resale, derived"), "{derived:?}");
        }
    }
    // The holdings pane.
    for page in [owner_page(), paid_page()] {
        let rows = render_holdings_rows(&page, &ctx(Some((48, 20))));
        let lines = text(&rows);
        for line in &lines {
            assert!(line.chars().count() <= 48, "{line:?}");
        }
        let header = lines.get(1).expect("a header under the section row");
        assert_eq!(header.find("NAME"), Some(HOLDINGS_COLUMN_STARTS[2]));
        assert!(!header.contains("PHASE"), "{header:?}");
    }
    let wide = text(&render_holdings_rows(&paid_page(), &ctx(None)));
    assert_eq!(
        wide.first().map(String::as_str),
        Some("paid section (h: owner)")
    );
    let pin = wide
        .iter()
        .find(|l| l.starts_with("42"))
        .expect("the House Pin row");
    assert_eq!(
        pin.find("House Pin"),
        Some(HOLDINGS_COLUMN_STARTS[1]),
        "{pin:?}"
    );
    assert_eq!(
        pin.find("Active, 9d left"),
        Some(HOLDINGS_COLUMN_STARTS[4]),
        "{pin:?}"
    );
    assert!(pin.ends_with("no resale"), "{pin:?}");
    let offered = wide
        .iter()
        .find(|l| l.starts_with("41"))
        .expect("licence 41");
    assert!(offered.ends_with("renew offer"), "{offered:?}");
    // A bad shape, a closed market.
    let bad = text(&render_market_rows(
        &json!({ "schema": { "id": "mind.market", "version": 2 } }),
        &ctx(None),
    ));
    assert_eq!(bad, ["No such market page."]);
    let closed = text(&render_market_rows(
        &json!({ "schema": { "id": "mind.market", "version": 1 }, "open": false,
                 "notice": "The mind market is not open here.", "rows": [], "after": null,
                 "own": { "quotes": [], "waiting": [] } }),
        &ctx(None),
    ));
    assert_eq!(closed, ["The mind market is not open here."]);
    // Away from the counter with a purchase in escrow (the C3b review): the notice, then the own
    // strip, whose line for a code this page does not list is keyed so `b` reaches it.
    let away = render_market_rows(&away_page(), &ctx(None));
    assert_eq!(
        text(&away),
        ["You are not at a market counter.", "waiting 12345678"]
    );
    let keyed: Vec<&str> = away
        .rows
        .iter()
        .filter(|r| r.selectable)
        .filter_map(|r| r.row_key.as_deref())
        .collect();
    assert_eq!(keyed, ["12345678"]);
    // The frames.
    for (w, h) in [(FLOOR_COLS, 34), (160, 44)] {
        for (holdings, section, holding) in [
            (owner_page(), "owner section (h: paid)", "7         owner"),
            (paid_page(), "paid section (h: owner)", "Kestrel Line"),
        ] {
            let mut app = TuiApp::new(caps(w, h));
            app.set_clock(|| NOW);
            app.set_manifest(TuiManifest {
                panels: MARKET.iter().map(|p| (*p).to_owned()).collect(),
                character: Some("Nick".into()),
                room: None,
            });
            let _ = app.feed_input(b"\x1bm");
            let _ = app.apply_event(&keyframe("mind.market", market_page()));
            let _ = app.apply_event(&keyframe("mind.holdings", holdings));
            let frame = frame_text(&mut app, w, h);
            assert!(
                frame.contains("0a1b2c3d") && frame.contains("1250cr"),
                "{frame}"
            );
            // The holdings pane is drawn beside the listings at the floor too: the pane the
            // `k t l x` keys read from is never one the layout dropped.
            assert!(
                frame.contains(section) && frame.contains(holding),
                "the holdings pane at {w}x{h}:\n{frame}"
            );
            // The legend keeps every form key at the floor — the keys the server's own words
            // (`Accept it with ...`, the offer lines) correspond to — and drops only from the
            // right (`[ first page`, `Enter type` at 120), with `? help` last.
            for entry in [
                "b buy",
                "k renew",
                "t offer",
                "a accept",
                "l publish",
                "x withdraw",
                "h section",
                "r refresh",
                "] next page",
                "? help",
            ] {
                assert!(frame.contains(entry), "{entry} in the legend:\n{frame}");
            }
            // The planted strings whole, and their prefixes: a column cuts a cell to its width
            // (24 at most), so a leaked token would show only as its start.
            mush_contract_tests::assert_projection_text_clean(
                "TUI market screen",
                &frame,
                &[
                    TOKEN,
                    &TOKEN[..12],
                    ACCOUNT,
                    DIGEST,
                    &DIGEST[..14],
                    "4242",
                    "+minds",
                ],
            );
        }
    }
}

/// **A bare key sends nothing; Enter in a form sends one valid typed request** (17g cut C3b;
/// D-I-18): `b k t a l x h` on the market screen queue no invoke (a form opens, a sentence
/// shows, or — `h` — a page is asked); Enter in each form sends exactly one
/// `MindRequest::Market` that passes `validate`, naming the selected row's code, licence or
/// filed number and the step the form shows; the buy form's default step follows `own` (a live
/// quote pays, a waiting purchase completes) and the renew form's follows `renewal` (an offer
/// pays); Esc sends nothing; a typed price `validate` refuses sends nothing and says so.
#[test]
fn bare_keys_send_nothing_and_enter_sends_one_valid_market_action() {
    let mut app = on_the_market(owner_page());
    for key in [b"b", b"k", b"t", b"a", b"l", b"x"] {
        let r = app.feed_input(key);
        assert!(
            r.invokes.is_empty() && r.submit.is_empty(),
            "{:?}: {:?}",
            key,
            r.invokes
        );
        let _ = app.feed_input(b"\x1b");
    }
    // `k` on an owner row is not a renewal.
    let _ = app.feed_input(b"k");
    assert!(
        frame_text(&mut app, 120, 34).contains("This licence is not renewable now."),
        "the status line names it"
    );
    // `b` over the first listing: a live quote, so `pay`.
    let _ = app.feed_input(b"b");
    let frame = frame_text(&mut app, 120, 34);
    assert!(
        frame.contains("Buy listing 0a1b2c3d: Drift Kestrel"),
        "{frame}"
    );
    assert!(frame.contains("Your quote: 1250cr, 240s left."), "{frame}");
    assert!(
        frame.contains("Price 1250cr: Registry 5%, venue 2%, charter owner the rest."),
        "{frame}"
    );
    let (id, action) = one_market(&app.feed_input(b"\r"));
    assert_eq!(
        action,
        MarketAction::Pay {
            code: "0a1b2c3d".try_into().expect("a code")
        }
    );
    refused(&mut app, id);
    // The second listing: a waiting purchase, so `complete`; Right steps on to `refund`.
    let _ = app.feed_input(b"\x1b[B");
    let _ = app.feed_input(b"b");
    let (id, action) = one_market(&app.feed_input(b"\r"));
    assert_eq!(
        action,
        MarketAction::Complete {
            code: "99aa77bb".try_into().expect("a code")
        }
    );
    refused(&mut app, id);
    let _ = app.feed_input(b"b");
    let _ = app.feed_input(b"\x1b[C");
    let (id, action) = one_market(&app.feed_input(b"\r"));
    assert_eq!(
        action,
        MarketAction::Refund {
            code: "99aa77bb".try_into().expect("a code")
        }
    );
    refused(&mut app, id);
    // Esc sends nothing.
    let _ = app.feed_input(b"b");
    let r = app.feed_input(b"\x1b");
    assert!(r.invokes.is_empty());
    // `t` over filed mind 7: a charter offer.
    let _ = app.feed_input(b"t");
    let _ = app.feed_input(b"Mireille");
    let _ = app.feed_input(b"\t");
    let _ = app.feed_input(b"12.50");
    let (id, action) = one_market(&app.feed_input(b"\r"));
    assert_eq!(
        action,
        MarketAction::Charter {
            filed: 7,
            player: "Mireille".into(),
            price: "12.50".into()
        }
    );
    refused(&mut app, id);
    // A price `validate` refuses sends nothing and says so.
    let _ = app.feed_input(b"t");
    let _ = app.feed_input(b"Mireille\tlots");
    let r = app.feed_input(b"\r");
    assert!(r.invokes.is_empty(), "{:?}", r.invokes);
    assert!(
        frame_text(&mut app, 120, 34).contains("Request refused: invalid market price"),
        "the refusal is shown"
    );
    let _ = app.feed_input(b"\x1b");
    // `l`: open, royalty 25, not transferable.
    let _ = app.feed_input(b"l");
    let _ = app.feed_input(b"\x1b[C");
    let _ = app.feed_input(b"\t25\t");
    let _ = app.feed_input(b"\x1b[C");
    let (id, action) = one_market(&app.feed_input(b"\r"));
    assert_eq!(
        action,
        MarketAction::Publish {
            filed: 7,
            form: MarketListingForm::Open,
            royalty: Some(25),
            transfer: Some(false)
        }
    );
    refused(&mut app, id);
    // `x`: the confirm, then Enter.
    let _ = app.feed_input(b"x");
    assert!(
        frame_text(&mut app, 160, 34).contains(
            "Withdraw filed mind #7 Drift Kestrel? New sales stop; licences already sold keep working. Enter withdraws; Esc cancels."
        ),
        "the withdrawal's confirm"
    );
    let (id, action) = one_market(&app.feed_input(b"\r"));
    assert_eq!(action, MarketAction::Withdraw { filed: 7 });
    refused(&mut app, id);
    // `a`: a licence number and no price lists the offers.
    let _ = app.feed_input(b"a");
    let _ = app.feed_input(b"\t41");
    let (id, action) = one_market(&app.feed_input(b"\r"));
    assert_eq!(
        action,
        MarketAction::ResellAccept {
            licence: 41,
            price: None
        }
    );
    refused(&mut app, id);
    // `h`: the paid section, asked from its first page; no invoke.
    let r = app.feed_input(b"h");
    assert!(r.invokes.is_empty());
    assert_eq!(
        r.queries,
        [(
            "mind.holdings".to_owned(),
            json!({ "section": "paid", "after": null })
        )]
    );
    let _ = app.apply_event(&keyframe("mind.holdings", paid_page()));
    // `k` over licence 41, whose renewal offer waits: `pay`.
    let _ = app.feed_input(b"k");
    let (id, action) = one_market(&app.feed_input(b"\r"));
    assert_eq!(action, MarketAction::RenewPay { licence: 41 });
    refused(&mut app, id);
    // `t` over licence 41: a resale offer.
    let _ = app.feed_input(b"t");
    let _ = app.feed_input(b"Mireille\t5");
    let (_, action) = one_market(&app.feed_input(b"\r"));
    assert_eq!(
        action,
        MarketAction::Resell {
            licence: 41,
            player: "Mireille".into(),
            price: "5".into()
        }
    );
}

/// **The replies drive the form** (17g cut C3b; the C3 plan §2.4): a `Staged` reply closes the
/// form, puts its `line` on the status line and asks the two pages the screen is on once (the
/// offer form leaves its closing instruction, built from the typed fields); `Shown` keeps the
/// accept form open with the lines inside it; `Error` closes the form with its `message`; the
/// renewal's `Staged` closes the renew form; a reply on an id the desk does not own changes
/// nothing; after a session boundary a late reply is ignored and C2's "not a refusal" sentence
/// stands. Every id drawn is at least `1 << 40` and at most `MAX_VIEW_WIRE_INTEGER`.
#[test]
fn staged_shown_and_error_replies_drive_the_form() {
    let mut app = on_the_market(owner_page());
    // The page the screen is on: `]` moves the listings to the next one.
    let r = app.feed_input(b"]");
    assert_eq!(
        r.queries,
        [("mind.market".to_owned(), json!({ "after": "99aa77bb" }))]
    );
    let _ = app.apply_event(&keyframe("mind.market", market_page()));
    // An offer, staged.
    let _ = app.feed_input(b"t");
    let _ = app.feed_input(b"Mireille\t12.50");
    let (id, _) = one_market(&app.feed_input(b"\r"));
    assert!(
        frame_text(&mut app, 120, 34).contains("Sent - waiting for the server"),
        "the form says it is out"
    );
    // A second Enter while it is out sends nothing.
    assert!(app.feed_input(b"\r").invokes.is_empty());
    // A foreign id: ignored.
    let none = reply(
        &mut app,
        id + 7,
        MindResponse::Staged {
            line: "FOREIGN".into(),
        },
    );
    assert!(none.is_empty());
    assert!(!frame_text(&mut app, 120, 34).contains("FOREIGN"));
    let line = "Offering filed mind 7 and its charter to Mireille for 12.50cr.";
    let asks = reply(&mut app, id, MindResponse::Staged { line: line.into() });
    assert_eq!(
        asks,
        [
            ("mind.market".to_owned(), json!({ "after": "99aa77bb" })),
            (
                "mind.holdings".to_owned(),
                json!({ "section": "owner", "after": null })
            ),
        ],
        "the two pages it was on, once"
    );
    let frame = frame_text(&mut app, 120, 34);
    assert!(frame.contains(line), "the status line: {frame}");
    assert!(
        frame.contains("They accept with: a, filed 7, price 12.50."),
        "the closing instruction: {frame}"
    );
    assert!(app.feed_input(b"\x1b").invokes.is_empty());
    assert!(!frame_text(&mut app, 120, 34).contains("They accept with"));
    // An accept without a price: the offers are shown in the open form.
    let _ = app.feed_input(b"a");
    let _ = app.feed_input(b"\x1b[C");
    let _ = app.feed_input(b"\t7");
    let (id, action) = one_market(&app.feed_input(b"\r"));
    assert_eq!(
        action,
        MarketAction::CharterAccept {
            filed: 7,
            price: None
        }
    );
    let offer = "Filed mind 7 and its charter, offered to you by Nick for 12.50cr.";
    let how = "Accept it with +minds/charter/accept 7 price=12.50.";
    let asks = reply(
        &mut app,
        id,
        MindResponse::Shown {
            lines: vec![offer.into(), how.into()],
        },
    );
    assert!(asks.is_empty(), "a shown reply asks nothing");
    let frame = frame_text(&mut app, 160, 34);
    assert!(frame.contains("Accept an offer"), "the form stays: {frame}");
    assert!(frame.contains(offer) && frame.contains(how), "{frame}");
    // With a price: an error closes the form with its message.
    let _ = app.feed_input(b"\t12.50");
    let (id, _) = one_market(&app.feed_input(b"\r"));
    let message = "No offer on filed mind 7 names you at 12.50cr.";
    let asks = reply(
        &mut app,
        id,
        MindResponse::Error {
            code: "no_charter_offer_at".into(),
            message: message.into(),
        },
    );
    assert!(asks.is_empty());
    let frame = frame_text(&mut app, 160, 34);
    assert!(frame.contains(message), "{frame}");
    assert!(
        !frame.contains("Accept an offer"),
        "the form closed: {frame}"
    );
    // The renewal hands off and closes.
    let _ = app.feed_input(b"h");
    let _ = app.apply_event(&keyframe("mind.holdings", paid_page()));
    let _ = app.feed_input(b"k");
    let (id, _) = one_market(&app.feed_input(b"\r"));
    let handed = "Handing licence 41 to the renewal service.";
    let asks = reply(
        &mut app,
        id,
        MindResponse::Staged {
            line: handed.into(),
        },
    );
    assert_eq!(
        asks.get(1),
        Some(&(
            "mind.holdings".to_owned(),
            json!({ "section": "paid", "after": null })
        ))
    );
    let frame = frame_text(&mut app, 120, 34);
    assert!(frame.contains(handed), "{frame}");
    assert!(
        !frame.contains("Renew licence 41"),
        "the form closed: {frame}"
    );
    // A session boundary forgets the pending id: the late reply is ignored.
    let _ = app.feed_input(b"b");
    let (id, _) = one_market(&app.feed_input(b"\r"));
    app.set_manifest(TuiManifest {
        panels: MARKET.iter().map(|p| (*p).to_owned()).collect(),
        character: Some("Another".into()),
        room: None,
    });
    let asks = reply(
        &mut app,
        id,
        MindResponse::Staged {
            line: "LATE".into(),
        },
    );
    assert!(asks.is_empty());
    let frame = frame_text(&mut app, 160, 34);
    assert!(!frame.contains("LATE"), "{frame}");
    assert!(
        frame.contains(
            "A request whose result you have not seen is not a refusal; check your holdings before trying again."
        ),
        "{frame}"
    );
    // The next id is still the desk's, never reused.
    let _ = app.apply_event(&keyframe("mind.market", market_page()));
    let _ = app.feed_input(b"b");
    let (next, _) = one_market(&app.feed_input(b"\r"));
    assert!(next > id);
}

/// The frame with no market form open: none of the forms' titles.
fn no_form(frame: &str) -> bool {
    [
        "Buy listing",
        "Renew licence",
        "Offer licence",
        "Offer filed mind",
        "Accept an offer",
        "List filed mind",
        "Withdraw filed mind",
    ]
    .iter()
    .all(|title| !frame.contains(title))
}

/// **Every default of the buy step, and the purchase off the page** (the C3 plan §2.4; the C3b
/// review): with nothing of the actor's on a listing the step is `quote`; with the actor's quote
/// expired it is `quote` again and the form says `Your quote has expired.`; a waiting purchase
/// whose listing this page does not list is reached from its own-strip line and completes — and
/// still refunds — and so is one held while the actor is away from any counter, where the page
/// has no rows at all.
#[test]
fn the_buy_step_follows_every_own_state_and_a_purchase_off_the_page_stays_reachable() {
    let mut app = on_the_market(owner_page());
    let _ = app.apply_event(&keyframe("mind.market", third_page()));
    // The first selectable row is the strip's `waiting 12345678`: no listing, no terms.
    let _ = app.feed_input(b"b");
    let frame = frame_text(&mut app, 160, 34);
    assert!(frame.contains("Buy listing 12345678"), "{frame}");
    let (id, action) = one_market(&app.feed_input(b"\r"));
    assert_eq!(
        action,
        MarketAction::Complete {
            code: "12345678".try_into().expect("a code")
        }
    );
    refused(&mut app, id);
    let _ = app.feed_input(b"b");
    let _ = app.feed_input(b"\x1b[C");
    let (id, action) = one_market(&app.feed_input(b"\r"));
    assert_eq!(
        action,
        MarketAction::Refund {
            code: "12345678".try_into().expect("a code")
        }
    );
    refused(&mut app, id);
    // `c0ffee00`: nothing of the actor's, so `quote`, and no quote line.
    let _ = app.feed_input(b"\x1b[B");
    let _ = app.feed_input(b"b");
    let frame = frame_text(&mut app, 160, 34);
    assert!(
        frame.contains("Buy listing c0ffee00: Quiet Hour"),
        "{frame}"
    );
    assert!(!frame.contains("Your quote"), "{frame}");
    let (id, action) = one_market(&app.feed_input(b"\r"));
    assert_eq!(
        action,
        MarketAction::Quote {
            code: "c0ffee00".try_into().expect("a code")
        }
    );
    refused(&mut app, id);
    // `deadbeef`: the actor's quote expired, so `quote` again, and the form says so.
    let _ = app.feed_input(b"\x1b[B");
    let _ = app.feed_input(b"b");
    let frame = frame_text(&mut app, 160, 34);
    assert!(frame.contains("Buy listing deadbeef: Old Lamp"), "{frame}");
    assert!(frame.contains("Your quote has expired."), "{frame}");
    let (id, action) = one_market(&app.feed_input(b"\r"));
    assert_eq!(
        action,
        MarketAction::Quote {
            code: "deadbeef".try_into().expect("a code")
        }
    );
    refused(&mut app, id);
    // Away from any counter: the notice, the strip, and `b` on the purchase.
    let _ = app.apply_event(&keyframe("mind.market", away_page()));
    let frame = frame_text(&mut app, 120, 34);
    assert!(
        frame.contains("You are not at a market counter.") && frame.contains("waiting 12345678"),
        "{frame}"
    );
    let _ = app.feed_input(b"b");
    let (_, action) = one_market(&app.feed_input(b"\r"));
    assert_eq!(
        action,
        MarketAction::Complete {
            code: "12345678".try_into().expect("a code")
        }
    );
}

/// **The pages the screen is on** (the C3 plan §2.4): `]` and `[` page the FOCUSED pane — the
/// listings, or after Tab the holdings, whose cursor is the section's own — and `r` on the market
/// screen asks again exactly the listings page and the holdings section and page the screen is
/// on, never `on_enter`'s first pages; `]` on a last page asks nothing and says so.
#[test]
fn paging_and_refresh_ask_the_pages_the_screen_is_on() {
    let mut app = on_the_market(owner_page());
    let r = app.feed_input(b"]");
    assert_eq!(
        r.queries,
        [("mind.market".to_owned(), json!({ "after": "99aa77bb" }))]
    );
    let r = app.feed_input(b"h");
    assert_eq!(
        r.queries,
        [(
            "mind.holdings".to_owned(),
            json!({ "section": "paid", "after": null })
        )]
    );
    let _ = app.apply_event(&keyframe("mind.holdings", paid_page()));
    let r = app.feed_input(b"r");
    assert!(r.invokes.is_empty() && r.submit.is_empty());
    assert_eq!(
        r.queries,
        [
            ("mind.market".to_owned(), json!({ "after": "99aa77bb" })),
            (
                "mind.holdings".to_owned(),
                json!({ "section": "paid", "after": null })
            ),
        ],
        "`r` asks the pages the screen is on"
    );
    let r = app.feed_input(b"[");
    assert_eq!(
        r.queries,
        [("mind.market".to_owned(), json!({ "after": null }))]
    );
    // The holdings pane: its own cursor, its own section.
    let _ = app.feed_input(b"\t");
    let r = app.feed_input(b"]");
    assert_eq!(
        r.queries,
        [(
            "mind.holdings".to_owned(),
            json!({ "section": "paid", "after": 42 })
        )]
    );
    let r = app.feed_input(b"r");
    assert_eq!(
        r.queries,
        [
            ("mind.market".to_owned(), json!({ "after": null })),
            (
                "mind.holdings".to_owned(),
                json!({ "section": "paid", "after": 42 })
            ),
        ]
    );
    let r = app.feed_input(b"[");
    assert_eq!(
        r.queries,
        [(
            "mind.holdings".to_owned(),
            json!({ "section": "paid", "after": null })
        )]
    );
    // The last page.
    let mut last = paid_page();
    last["after"] = Value::Null;
    let _ = app.apply_event(&keyframe("mind.holdings", last));
    let r = app.feed_input(b"]");
    assert!(r.queries.is_empty(), "{:?}", r.queries);
    assert!(frame_text(&mut app, 120, 34).contains("This is the last page."));
}

/// **A page of the wrong shape opens nothing, and a row without the action is refused** (the
/// C3 plan §2.4, §3.2): a malformed `mind.market` keyframe paints `No such market page.` and
/// `b` opens no form; a malformed `mind.holdings` keyframe does the same for `t`, `l` and `x`;
/// on the paid section a paid licence refuses `l` and `x` with their sentences, and the House
/// Pin — no `resell` — refuses `t` with `This holding cannot be offered now.`; none of them
/// sends.
#[test]
fn a_bad_page_opens_no_form_and_a_row_without_the_action_is_refused() {
    let bad_market = json!({ "schema": { "id": "mind.market", "version": 2 }, "open": true });
    let bad_holdings = json!({ "schema": { "id": "mind.holdings", "version": 2 }, "open": true });
    let mut app = on_the_market(owner_page());
    let _ = app.apply_event(&keyframe("mind.market", bad_market));
    let r = app.feed_input(b"b");
    assert!(r.invokes.is_empty());
    let frame = frame_text(&mut app, 120, 34);
    assert!(no_form(&frame), "{frame}");
    assert!(frame.contains("No such market page."), "{frame}");
    let _ = app.apply_event(&keyframe("mind.holdings", bad_holdings));
    for key in [b"t", b"l", b"x"] {
        let r = app.feed_input(key);
        assert!(r.invokes.is_empty());
        let frame = frame_text(&mut app, 120, 34);
        assert!(no_form(&frame), "{key:?}:\n{frame}");
    }

    let mut app = on_the_market(paid_page());
    let _ = app.feed_input(b"\t");
    // Licence 41 (paid, renewable, resaleable): neither listable nor withdrawable.
    for (key, sentence) in [
        (b"l", "This filed mind cannot be listed now."),
        (b"x", "This filed mind cannot be withdrawn now."),
    ] {
        let r = app.feed_input(key);
        assert!(r.invokes.is_empty() && r.submit.is_empty());
        let frame = frame_text(&mut app, 160, 34);
        assert!(no_form(&frame), "{frame}");
        assert!(frame.contains(sentence), "{sentence}:\n{frame}");
    }
    // The House Pin: no `resell`, so no offer.
    let _ = app.feed_input(b"\x1b[B");
    let r = app.feed_input(b"t");
    assert!(r.invokes.is_empty() && r.submit.is_empty());
    let frame = frame_text(&mut app, 160, 34);
    assert!(no_form(&frame), "{frame}");
    assert!(
        frame.contains("This holding cannot be offered now."),
        "{frame}"
    );
}

/// **One request at a time, and Esc never reads as a cancel** (the C3 plan §2.4; the C3b
/// review): a form opened after another form's refusal does not paint that refusal as its own;
/// while a request is out its form says Esc does not cancel it, and once the window is closed
/// every form key answers the one-pending sentence and opens nothing; the reply settles it and
/// the keys open forms again.
#[test]
fn one_pending_request_holds_the_form_keys_and_esc_does_not_cancel() {
    let mut app = on_the_market(owner_page());
    let _ = app.feed_input(b"b");
    let (id, _) = one_market(&app.feed_input(b"\r"));
    refused(&mut app, id);
    // The next form: the status line keeps the refusal, the form does not repeat it.
    let _ = app.feed_input(b"\x1b[B");
    let _ = app.feed_input(b"b");
    let frame = frame_text(&mut app, 160, 34);
    assert!(frame.contains("Buy listing 99aa77bb"), "{frame}");
    assert_eq!(
        frame.matches("Nothing changed.").count(),
        1,
        "the status line only:\n{frame}"
    );
    // Sent: the form says Esc closes the window and does not cancel.
    let (id, _) = one_market(&app.feed_input(b"\r"));
    let frame = frame_text(&mut app, 160, 34);
    assert!(frame.contains("Sent - waiting for the server"), "{frame}");
    assert!(
        frame.contains("Sent; Esc closes this window but does not cancel the request."),
        "{frame}"
    );
    assert!(!frame.contains("Esc cancels"), "{frame}");
    let _ = app.feed_input(b"\x1b");
    for key in [b"b", b"k", b"t", b"a", b"l", b"x"] {
        let r = app.feed_input(key);
        assert!(r.invokes.is_empty() && r.submit.is_empty(), "{key:?}");
        let frame = frame_text(&mut app, 160, 34);
        assert!(no_form(&frame), "{key:?}:\n{frame}");
        assert!(
            frame.contains("An operation is pending; wait for its result."),
            "{key:?}:\n{frame}"
        );
    }
    // The answer settles it.
    refused(&mut app, id);
    let _ = app.feed_input(b"b");
    assert!(frame_text(&mut app, 160, 34).contains("Buy listing"));
}

/// **Every session boundary forgets the pending request** (the C3 plan §2.4): a character
/// change, a world's `hello`, a `SessionInfo` (an attachment boundary even when the labels
/// match) and a `Resumed` each forget the desk's pending id — the late reply is ignored and
/// C2's "not a refusal" sentence stands — and the next request is sent at once under a fresh id.
#[test]
fn every_session_boundary_forgets_the_pending_request() {
    for boundary in ["character", "hello", "session", "resumed"] {
        let mut app = on_the_market(owner_page());
        let _ = app.feed_input(b"b");
        let (id, _) = one_market(&app.feed_input(b"\r"));
        match boundary {
            "character" => app.set_manifest(TuiManifest {
                panels: MARKET.iter().map(|p| (*p).to_owned()).collect(),
                character: Some("Another".into()),
                room: None,
            }),
            "hello" => {
                let _ = app.apply_event(&OutputEvent::Hello {
                    proto: 1,
                    server: "rustmush test".into(),
                    features: vec![],
                    panels: MARKET.iter().map(|p| (*p).to_owned()).collect(),
                    asset_pack: None,
                });
            }
            "session" => {
                let _ = app.apply_event(&OutputEvent::SessionInfo {
                    character: ObjRef::new("#9", "Nick"),
                    account: "nick".into(),
                    connections: 1,
                });
            }
            _ => {
                let _ = app.apply_event(&OutputEvent::Resumed { replayed: true });
            }
        }
        assert_eq!(app.active_screen(), MARKET_SCREEN, "{boundary}");
        let asks = reply(
            &mut app,
            id,
            MindResponse::Staged {
                line: "LATE".into(),
            },
        );
        assert!(asks.is_empty(), "{boundary}");
        let frame = frame_text(&mut app, 160, 34);
        assert!(!frame.contains("LATE"), "{boundary}:\n{frame}");
        assert!(
            frame.contains(
                "A request whose result you have not seen is not a refusal; check your holdings before trying again."
            ),
            "{boundary}:\n{frame}"
        );
        let _ = app.apply_event(&keyframe("mind.market", market_page()));
        let _ = app.feed_input(b"b");
        let (next, _) = one_market(&app.feed_input(b"\r"));
        assert!(next > id, "{boundary}: a fresh id");
    }
}
