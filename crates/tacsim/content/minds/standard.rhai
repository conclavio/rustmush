// Standard Doctrine — the combat role's stock mind (doc 17 §11, §17; 17b cut M2.5). name: Standard Doctrine; role: combat; api: 3; stock: 2; rated: class I (1,500 ops a think). Ported arm for arm from crates/tacsim/src/arena/stock_mind.rs: T1 is the oracle (Gate A).
//
// `decide_on_card`, re-expressed over the TacSim API (17b cuts M2.3 and
// M2.4): the SAME order, edges, clocks and fifteen memory keys as the native
// stock mind, every score, trace, picker and reach reader a registered kernel
// over the one free function T1 calls (no second kernel). Gate A is T1 == T2
// exactly: every think's intent list and memory bytes equal
// (`tests/support`'s `TwinRunner`).
//
// # The shape (17b §5's feas F9: the binding limits bind the whole script)
//
// Every function body is parsed at expression depth 24 with a level per
// block, `if`, `else if`, loop and `switch`; at most 32 functions; at most
// 64 variables in a call's scope. `decide_on_card`'s state arm nests about
// eight deep, so the port is `switch` dispatch on the state and on the
// objective with one helper `fn` per state arm and per T1 helper. A helper
// that writes the memory is called on `this` (`this.walk(..)`); a helper that
// decides answers its decision. The think's working values ride one map,
// `p` (T1's locals and its `Plan`), handed to each helper and handed back:
//
//   here      the hull's cell (`view.cell`), `()` off the window
//   reached   the goal the motor said it reached (T1's `Drive`), or `()`
//   held      the goal the motor holds as far as the mind knows
//   goal      `motor.goal` as the think writes it
//   stop      the hull stands this think: `hold()`
//   heading   the facing a stand turns onto, millidegrees, or `()`
//   lock      the lock to take, a contact INT, or `()`
//   stood     the cell of the row a stand stood on, or `()`
//   arm_ceiling  the ceiling the slots arm on (`engage_orders` records it
//             past its gates), `()` for none — the slots disarm
//   fire      the citation `[weapon, slot]` planned beside a fresh lock, or `()`
//   view_order, order, objective   the view's order, the OBEYED order and
//             its objective word (`()` once ended or ignored)
//   health, dry, ranges            the think's own readings
//   leash_on, charge, off_leash, leash_cm, station_cm, leashed, station_on
//                                  the supporter's leash (16b cut Z.2)
//
// # The state codes (`ReflexState::code`, pinned by `tests/content.rs`
// against `ReflexState::ALL`)
//
//   state codes: 0 startup, 1 patrol, 2 approach, 3 engage, 4 withdraw, 5 disabled
//
// # The memory (D-UM-30): the fifteen keys of `StockMemory`, as T1 writes them
//
//   state INT code; target the row's ContactId as its INT (`contact_int`);
//   goal, hunt [x, y]; waypoint, from_waypoint INT node; last_plot
//   [x, y, t_ms]; unheld_since_ms, quiet_since_ms, stand_until_ms,
//   hold_retry_ms, ignore_until_ms INT ms; rallied, no_line, ordered bool
//   (`ordered`, 17b cut M4.3: the Withdraw is an order's, restored at its
//   end; a released seat's seeded Withdraw and a dry or hurt one are not);
//   `()` for every key that holds nothing.
//
// # RANGE_NONE
//
// T1 reads a row it does not hold as `u64::MAX` centimetres
// (`row.map_or(u64::MAX, row_range_cm)`); a Rhai INT is an `i64`, so the
// port's sentinel is `i64::MAX`, 9223372036854775807 (`range_cm`). Every
// comparison T1 makes against it is `range > ceiling` or
// `range <= ceiling.max(1)`, which the sentinel answers as `u64::MAX` does
// for every ceiling short of 2^63 cm. No Approach or Engage think reaches it
// in practice: the lost edge drops a target whose row is gone and the
// hostile edges pick only rows the view holds.
//
// # The fire half (17b cut M2.6; D-UM-16, D-UM-17, D-UM-18)
//
// D-UM-48: on a hull with no weapons station Standard Doctrine's fire half
// is absent — every fire-half function returns before it reads a weapon.
// On an armed hull the SLOT owns the shot: `engage_orders`, reached with a
// row and past the power, `hold_fire` and dry gates, records the ceiling
// that admitted the row (`p.arm_ceiling`), and the emission arms the first
// `slots()` weapons on it, in component order and whatever the heat
// (`arming_for`: one `arm(weapon, "best", ..)` per weapon whose view row
// does not already hold the terms wanted, the slot NUMBER never compared —
// it is the arena's — and one `disarm(())` only where some row is armed and
// the think wants none). A `fire(weapon, slot)` is cited ONLY beside a
// fresh lock, which no slot has yet been evaluated against: the first armed
// weapon ready at the landing (`lands_ms()` against the VIEW's clock,
// `view.t_ms`, as T1 reads it), loaded, in reach and not held back by heat,
// numbered as the view's row shows it (`armed.slot`), else `index + 1` for a
// weapon this very think arms. The emission is the arming first (after
// `startup()` in the power arms), then the helm half, then `lock`, then
// `fire` — the landing applies them in that order.
//
// # No float, no clock, no draw
//
// Integer arithmetic only (Rust's division); the clocks are the context
// functions (`t_ms()`, `decide_ms()`, ...), never a constant; nothing draws.
//
// # Saturation (where T1 saturates, this clamps)
//
// A Rhai INT overflow is a runtime error, so a fault and a strike where T1's
// `saturating_*` answers. Every site T1 saturates on is clamped here so that
// no INT the view or the memory can hold overflows:
//
//   range_cm      `min(range_m, 92233720368547758) * 100` (T1's
//                 `range_m.saturating_mul(100)`; the getter already reads a
//                 range past i64 as i64::MAX, and the clamp keeps the product
//                 inside an INT, past every ceiling as T1's is);
//   the dwell     `min(t, 9223372036854775807 - dwell) + dwell` (T1's
//                 `t_ms.saturating_add(..)`, `dwell` the context's own small
//                 `stand_ticks() * decide_ms()`);
//   the clocks    T1's `t_ms.saturating_sub(since) <op> card_ms` compares a
//                 clamped difference with a card's u32 milliseconds, which is
//                 the exact difference's compare; written here as
//                 `since <op'> t_ms() - card_ms`, the arithmetic on the arena
//                 clock (never below zero) and the card alone, so a memory
//                 clock of any INT never overflows (the search, the hunt and
//                 the rally);
//   the bearings  `bearing_deg * 1000` on a view's whole degrees (0 to 359;
//                 an INT product of a u32 and 1,000 cannot overflow);
//   the landing   T1's `lands_ms.saturating_sub(view.t_ms).max(0)`, written
//                 as `0` when the view's clock is at or past the landing and
//                 the plain difference otherwise — the view's clock is the
//                 arena clock (never below zero), so it cannot overflow;
//   the reserve   T1's `reserve_pct.saturating_mul(rounds_max) / 100` held to
//                 a u32: the product of two u32s can pass an INT, so a product
//                 past `i64::MAX` (whose hundredth is past every u32) reads
//                 4294967295 before it is formed, and the hundredth is held
//                 at 4294967295 otherwise.

// ---------------------------------------------------------------------------
// The entry
// ---------------------------------------------------------------------------

// **One think** (T1's `decide_on_card`): walk the memory (`this`) one step
// over `view` on `card` and push the intents, in emission order.
fn think(view, card) {
    // ---- the prologue: a fresh sortie writes all fifteen keys, so the
    //      first think's bytes are T1's (an empty memory reads as the
    //      default `StockMemory`) ----
    if !("state" in this) {
        this.fresh();
    }

    // ---- what the motor and the rounds said since the last think ----
    //
    // The reflex learns its `no line` edge from the impact and its arrival
    // and its `unreachable` edge from the drive it runs itself; a mind reads
    // all three off its notice buffer (D-UM-8).
    let words = this.read_notices(view);
    // What the motor holds now, as far as the mind knows: the goal it was
    // handed, and nothing once it said no route reached it.
    let held = if words[1] { () } else { this.goal };
    let p = #{
        here: view.cell, reached: words[0], held: held, goal: held,
        stop: false, heading: (), lock: (), stood: (), arm_ceiling: (), fire: (),
        view_order: view.order, order: (), objective: (),
        health: 1000, dry: false, ranges: (),
        leash_on: false, charge: (), off_leash: false, leash_cm: 0, station_cm: 0,
        leashed: false, station_on: ()
    };

    // ---- the `Disabled` edge, before anything else ----
    if view.wreck || (view.immobile && view.weapons.len() == 0) {
        this.fresh();
        this.state = 5;
        return;
    }

    // ---- the `unreachable` edge's decision half, a think late
    //      (departure (i)) ----
    // A matching NoRoute already proves this.goal is the motor's cell.
    if words[1] {
        p.goal = this.unreachable(view, card, p);
    }

    // ---- the standing order (16b cut N.2; 17b cut M1b.5) ----
    p = this.standing_order(view, card, p);

    // ---- power ----
    switch view.power {
        "off" => {
            // 16b cut D.3 (`B-34`): a cold tank starts up, wherever it is
            // going; a `Withdraw` stays one.
            if this.state != 4 {
                this.state = 0;
            }
            this.goal = p.held;
            startup();
            // 17b cut M1b.2: a cold hull's slots are disarmed, after the
            // `Startup` — 17b cut M1b.3: where its view shows one armed.
            arming_for(view, card, ());
            return;
        }
        "starting" => {
            if this.state != 4 {
                this.state = 0;
            }
            this.goal = p.held;
            arming_for(view, card, ());
            return;
        }
        _ => {
            if this.state == 0 || this.state == 5 {
                this.state = 1;
                p.goal = ();
            }
        }
    }

    // ---- the `lost` edge, the order's steering, the `Withdraw` edges ----
    p = this.edges(view, card, p);

    // ---- the hostile edges, and the leash's reading (16b cut Z.2) ----
    p = this.hostile_edges(view, card, p);

    // ---- the `no line` edge (W's `blocked`/`cover` miss) ----
    if this.no_line {
        this.no_line = false;
        p.goal = ();
        if this.state == 3 {
            this.state = 2;
        }
    }

    // ---- the state's own goal and orders ----
    //
    // 17b §5's feas F6: the order cap off the OBEYED order (`p.order`, after
    // the early ends and `ignore_until_ms`), as T1 passes `obeyed(view, mem)`
    // to `view_ranges` — never `view.order`.
    p.ranges = ranges(view, card.band_permille, p.order != ());

    // ---- the leash (16b cut Z.2) ----
    p = this.leash_arms(view, card, p);
    if !p.leashed {
        p.station_on = if p.leash_on { p.charge } else { () };
        switch this.state {
            1 => {
                p = this.patrol_arm(view, card, p);
            }
            2 => {
                p = this.approach_arm(view, p);
            }
            3 => {
                p = this.engage_arm(view, p);
            }
            4 => {
                p = this.withdraw_arm(view, card, p);
            }
            // 0 startup, 5 disabled: nothing.
            _ => {}
        }
    }

    // ---- what this think saw, and how long it has seen nothing (16b cut
    //      E.4) ----
    this.stamps(view, p);

    // ---- the emission ----
    this.emit(view, card, p);
}

// ---------------------------------------------------------------------------
// The memory (D-UM-30's shape)
// ---------------------------------------------------------------------------

// **All fifteen keys at their defaults** (T1's `StockMemory::default()`,
// written as `Memory::from` writes it): the prologue's fresh sortie and the
// `Disabled` edge's reset.
fn fresh() {
    this.state = 0;
    this.target = ();
    this.goal = ();
    this.waypoint = ();
    this.from_waypoint = ();
    this.last_plot = ();
    this.unheld_since_ms = ();
    this.quiet_since_ms = ();
    this.stand_until_ms = ();
    this.hold_retry_ms = ();
    this.rallied = false;
    this.hunt = ();
    this.no_line = false;
    this.ignore_until_ms = ();
    this.ordered = false;
}

// `[reached_goal, no_route]` reads the persistent requested-goal result.
// A blocked/cover miss teaches no_line only while still at its launch cell.
fn read_notices(view) {
    let reached = ();
    let no_route = false;
    let motor = view.motor;
    if motor != () {
        switch motor.outcome {
            "arrived" => { reached = motor.goal; }
            "no_route" => { no_route = motor.goal == this.goal; }
        }
    }
    for notice in view.notices {
        switch notice.kind {
            "miss" => {
                let reason = notice.reason;
                let origin = notice.launch_cell;
                if origin != () && origin == view.cell && (reason == blocked() || reason == cover()) {
                    this.no_line = true;
                }
            }
        }
    }
    [reached, no_route]
}

// ---------------------------------------------------------------------------
// The kernels' companions (T1's small helpers)
// ---------------------------------------------------------------------------

// **The view's standing order, unless this hull has ended it** (T1's
// `obeyed`, 17b cut M1b.5): the order whose `expires_ms` is not
// `ignore_until_ms`.
fn obeyed(order, ignore) {
    if order == () || order.expires_ms == ignore {
        ()
    } else {
        order
    }
}

// **The contact an order points the hull at** (T1's `hint`): an `Engage`'s
// re-keyed hint. A `Support`'s charge designation is not on the view (the
// divergence `support`), so a supporter prefers nothing.
fn hint(order) {
    if order != () && order.objective == "engage" {
        order.target
    } else {
        ()
    }
}

// **The utility score's three weights off the card**, as
// `StockCard::as_reflex` derives them from `prefer` (17b cut M1b.5):
// `balanced` and `best_odds` the card's own carried triple, `nearest`
// 200/20/30, `exposed` 100/120/30, `hurt` 100/40/120.
fn weights(card) {
    switch card.prefer {
        "nearest" => [200, 20, 30],
        "exposed" => [100, 120, 30],
        "hurt" => [100, 40, 120],
        _ => [card.weight_range, card.weight_facet, card.weight_health],
    }
}

// **A row's held range, centimetres** (T1's `row_range_cm`: the view's whole
// metres back to the arena's unit), and RANGE_NONE (the head) for no row.
fn range_cm(row) {
    if row == () {
        9223372036854775807
    } else {
        min(row.range_m, 92233720368547758) * 100
    }
}

// Arrival names the requested cell in the motor's persistent completion.
fn arrived(p, cell) {
    p.reached == cell
}

// ---------------------------------------------------------------------------
// The edges
// ---------------------------------------------------------------------------

// **The `unreachable` edge's decision half** (T1's departure (i)): the
// motor dropped the goal with `drive_no_route`, and this is the reflex's
// edge read off it, in the reflex's order — the order's own ends, the
// search's and the hunt's give-ups, and the fallback waypoint as the next
// goal (answered).
fn unreachable(view, card, p) {
    let failed = this.goal;
    // 17b cut M1b.5: the view's order unless this hull ended it.
    let live = obeyed(p.view_order, this.ignore_until_ms);
    // 16b cut E.2 (`B-41`): the order's own `Advance` cell.
    if live != () && live.objective == "advance" && live.to == failed {
        this.ignore_until_ms = live.expires_ms;
    }
    // 16b cut Z.2's review: a supporter's station, in `Patrol`.
    if live != () && live.objective == "support" && this.state == 1 {
        this.ignore_until_ms = live.expires_ms;
    }
    // 16b cut E.4: the search's plot and the hunt's cell.
    let plot = this.last_plot;
    if plot != () && [plot[0], plot[1]] == failed {
        this.last_plot = ();
    }
    if this.hunt == failed {
        this.hunt = ();
        this.unheld_since_ms = t_ms();
    }
    let objective = view.objective;
    if objective != () && objective.cell == failed {
        let retry = max(card.hunt_after_ms, repath_ms());
        this.hold_retry_ms = min(t_ms(), 9223372036854775807 - retry) + retry;
    }
    if p.here == () {
        return ();
    }
    let walk = fall_back_to_waypoint(view, failed, this.waypoint, this.from_waypoint);
    this.waypoint = walk.waypoint;
    this.from_waypoint = walk.from_waypoint;
    walk.goal
}

// **The standing order** (16b cut N.2): the expiry is the ARENA's; a mind
// cannot end an order itself, so the three early ends write the ended
// order's `expires_ms` to `ignore_until_ms` and it stops obeying THAT order
// (the divergence `order_end`); the key is spent when the view carries no
// order or an order with another deadline. Then the health, the dryness,
// the expiry's restoration and the obeyed order with its objective.
fn standing_order(view, card, p) {
    let order = p.view_order;
    let deadline = if order == () { () } else { order.expires_ms };
    if deadline != this.ignore_until_ms {
        this.ignore_until_ms = ();
    }
    p.health = health_permille(view);
    p.dry = is_dry(view);
    // **The expiry's restoration** (`ReflexUnit::drop_to_doctrine`, read off
    // the view): a `Withdraw` no live order holds, that an order put the hull
    // in (`this.ordered`, 17b cut M4.3) and neither of its own edges would
    // have taken goes back to `Patrol`; a hurt one keeps going home, and so
    // does a released seat's seeded Withdraw (it rallies on Q.7's clock).
    if order == () && this.state == 4 && this.ordered && !p.dry && p.health >= card.withdraw_permille {
        this.state = 1;
        this.target = ();
        this.ordered = false;
        p.goal = ();
    }
    p.order = obeyed(order, this.ignore_until_ms);
    let ends = false;
    if p.order != () {
        switch p.order.objective {
            // 16b cut AA.1 (`B-76`): an `Advance` a hull that cannot close
            // can never carry out; an Advance into the Hold square also
            // ends once the hull reaches it. End remains mind-side ignore.
            "advance" => {
                ends = view.immobile || hold_advance_arrived(view, p.order.to);
            }
            // 16b cut Z.2: a `Support` whose charge this hull's own row
            // reads dead.
            "support" => {
                let of = p.order.of;
                if of != () {
                    let row = contact(view, of);
                    ends = row != () && row.reads_dead == true;
                }
            }
            _ => {}
        }
    }
    if ends {
        this.ignore_until_ms = p.order.expires_ms;
        p.order = ();
    }
    p.objective = if p.order == () { () } else { p.order.objective };
    p
}

// **The `lost` edge, the order's steering and the `Withdraw` edges.**
fn edges(view, card, p) {
    // ---- the `lost` edge (16b cut E.2, `B-45`): a target the view no
    //      longer holds, or one its row reads dead ----
    if this.target != () {
        let row = contact(view, this.target);
        if row == () || row.reads_dead == true {
            this.target = ();
            if this.state == 2 || this.state == 3 {
                this.state = 1;
                p.goal = ();
            }
        }
    }
    // ---- what the order does to the machine (16b cut N.2, Z.2) ----
    switch p.objective {
        "withdraw" => {
            if this.state != 4 {
                this.state = 4;
                this.target = ();
                p.goal = ();
                this.last_plot = ();
                // 17b cut M4.3: an order's Withdraw, restored at its end.
                this.ordered = true;
            }
        }
        "hold" | "advance" | "screen" => {
            if this.state == 2 || this.state == 3 {
                this.state = 1;
                p.goal = ();
            }
            this.target = ();
        }
        // `engage`, `support` and no order: nothing.
        _ => {}
    }
    // ---- the `Withdraw` edges (16b cut Q.7) ----
    if this.state != 4 && (p.dry || (!this.rallied && p.health < card.withdraw_permille)) {
        this.state = 4;
        this.target = ();
        p.goal = ();
        this.last_plot = ();
        // 17b cut M4.3: its own edge's Withdraw, never restored early.
        this.ordered = false;
    }
    p
}

// **The hostile edges, and the leash's reading** (16b cut Z.2): a
// supporter reads its own row of its charge (`charge` `()` is a supporter
// that does not hold it); the best hostile by the utility score, with the
// order's hint, wherever pursuit is allowed.
fn hostile_edges(view, card, p) {
    let state = this.state;
    if p.objective == "support" && (state == 1 || state == 2 || state == 3) {
        p.leash_on = true;
        let of = p.order.of;
        if of != () {
            p.charge = contact(view, of);
        }
    }
    // The leash's two radii, read only under a leash (17b cut M2.6: nothing
    // but the leash reads them, and a think pays for what it computes).
    if p.leash_on {
        let cell = cell_cm();
        p.leash_cm = leash_cells() * cell;
        p.station_cm = station_cells() * cell;
        p.off_leash = p.charge == () || range_cm(p.charge) > p.leash_cm;
    }
    let objective = p.objective;
    let may_pursue = !p.off_leash && objective != "hold" && objective != "advance" && objective != "screen";
    if !may_pursue || !(state == 1 || state == 2 || state == 3) {
        return p;
    }
    let w = weights(card);
    let found = best_hostile(view, w[0], w[1], w[2], p.health, hint(p.order), (), (), ());
    if found != () {
        if this.target != found {
            this.target = found;
            p.goal = ();
        }
        if this.state == 1 {
            this.state = 2;
        }
    } else {
        this.target = ();
        if this.state == 2 || this.state == 3 {
            this.state = 1;
            p.goal = ();
        }
    }
    p
}

// ---------------------------------------------------------------------------
// The leash, the stands and the lock
// ---------------------------------------------------------------------------

// **The leash's two arms** (16b cut Z.2): (a) blind to the charge, or past
// the leash and unable to close — hold where it stands and fight from
// there; (b) past the leash — back toward the station off its own plot of
// the charge, firing nothing until it is back inside; (c) inside the leash
// — the state's own arm (`p.leashed` false).
fn leash_arms(view, card, p) {
    if !p.leash_on {
        return p;
    }
    if p.charge == () || (p.off_leash && view.immobile) {
        p.leashed = true;
        if this.state == 2 || this.state == 3 {
            this.state = 1;
        }
        this.target = ();
        p.goal = p.here;
        p.stop = true;
        this.last_plot = ();
        this.hunt = ();
        this.unheld_since_ms = t_ms();
        let row = stand(view, card, p, ());
        if row != () {
            p = face(view, row, p.ranges.release_cm, p);
        }
    } else if p.off_leash {
        p.leashed = true;
        this.state = 1;
        this.target = ();
        this.last_plot = ();
        this.hunt = ();
        this.unheld_since_ms = t_ms();
        let station = stand_off_cell(view, p.charge.id, p.station_cm);
        p.goal = if station == () { p.here } else { station };
    }
    p
}

// **A stand on `row`**: the cell it stood on, the facing onto the row's
// whole-degree bearing, and `Engage`'s orders at `ceiling` (the ceiling
// that admitted the row: `release_cm`, or a withdraw stand's `stand_cm` en
// route).
fn face(view, row, ceiling, p) {
    p.stood = row.cell;
    p.heading = row.bearing_deg * 1000;
    engage_orders(view, row, ceiling, p)
}

// **`Engage`'s two discrete actions** (T1's `engage_orders`): the lock on a
// fresh row (the lock is sensors), then — inside `release_cm`, with power
// and not under `hold_fire` — the slots and the citation. D-UM-48: on a hull
// with no weapon rows the fire half is absent — not idled — and the guard
// says so where the reader looks.
//
// 17b cut M1b.2 (D-UM-16, D-UM-17, D-UM-18): past the power, `hold_fire` and
// dry gates the think records `ceiling` — the ceiling that admitted `row` —
// as the slots' arm ceiling (`p.arm_ceiling`); the emission arms on it
// (`arming_for`) and from then on the SLOT owns the shot. A citation is
// planned ONLY beside a fresh lock (one no slot has been evaluated against):
// the first weapon of the first `slots()` that is ready AT THE LANDING (its
// countdown against the milliseconds from the view's clock to `lands_ms()`:
// T1's 16b cut M.2 rule), loaded, reaches the row's held range and is not
// held back by heat (`would_overheat`), cited by the number its view row
// shows (`armed.slot`, the arena's stable number: 17b cut M1b.3), else
// `index + 1` for a weapon this very think arms.
fn engage_orders(view, row, ceiling, p) {
    let weapons = view.weapons;
    if weapons.len() == 0 {
        return p;
    }
    let id = row.id;
    let fresh = view.lock != id;
    if fresh {
        p.lock = id;
    }
    // The power, `hold_fire` and dry gates (`p.dry` is this view's
    // `is_dry`, read once by `standing_order`).
    if view.power != "online" || (p.order != () && p.order.hold_fire) || p.dry {
        return p;
    }
    // Reached with a row, the gates passed: the slots arm.
    p.arm_ceiling = ceiling;
    // The row's held range (`range_cm` of a row the view holds).
    let range = min(row.range_m, 92233720368547758) * 100;
    if range > p.ranges.release_cm || !fresh {
        // Out of the release band, or the lock is held: the armed slot owns
        // the shot.
        return p;
    }
    // The milliseconds from the view's clock to the landing (the head's
    // saturation): a weapon whose countdown ends inside them is ready when
    // the shot is resolved.
    let lands = lands_ms();
    let to_landing = if view.t_ms >= lands { 0 } else { lands - view.t_ms };
    let n = slots();
    for (weapon, i) in weapons {
        if i >= n {
            break;
        }
        if weapon.ready_in_ms <= to_landing && weapon.rounds > 0 {
            let reach = weapon.bands_cm;
            if reach.len() > 0 && reach[-1] >= range && !would_overheat(view, weapon.id) {
                let armed = weapon.armed;
                p.fire = [weapon.id, if armed == () { i + 1 } else { armed.slot }];
                return p;
            }
        }
    }
    p
}

// **A stand** (T1's `stand`): the best hostile inside `release_cm` — and,
// for a screen, inside `sector[1]` of `sector[0]` off the row's whole-degree
// bearing — with a line; `()` with nothing to fire.
fn stand(view, card, p, sector) {
    // `p.dry`: this view's `is_dry`, read once by `standing_order`.
    if p.dry {
        return ();
    }
    let ceiling = max(p.ranges.release_cm, 1);
    let w = weights(card);
    let id = if sector == () {
        best_hostile(view, w[0], w[1], w[2], p.health, hint(p.order), ceiling, (), ())
    } else {
        best_hostile(view, w[0], w[1], w[2], p.health, hint(p.order), ceiling, sector[0], sector[1])
    };
    if id == () {
        return ();
    }
    let row = contact(view, id);
    if row == () || !line_to(view, id) {
        return ();
    }
    row
}

// **Whether a withdrawing hull is cornered, and by which row** (T1's
// `withdraw_stand`): never with nothing to fire or under `hold_fire`; the
// best hostile, inside `release_cm` once home and `stand_cm` (and the card's
// `stand_min_permille` of health) en route, with a line.
fn withdraw_stand(view, card, p, home) {
    if p.dry || (p.order != () && p.order.hold_fire) {
        return ();
    }
    let w = weights(card);
    let id = best_hostile(view, w[0], w[1], w[2], p.health, (), (), (), ());
    if id == () {
        return ();
    }
    let row = contact(view, id);
    if row == () {
        return ();
    }
    let ceiling = if home { p.ranges.release_cm } else { p.ranges.stand_cm };
    if range_cm(row) > max(ceiling, 1) {
        return ();
    }
    if !home && p.health < card.stand_min_permille {
        return ();
    }
    if !line_to(view, id) {
        return ();
    }
    row
}

// ---------------------------------------------------------------------------
// The states' own arms
// ---------------------------------------------------------------------------

// **`Patrol`** (16b cuts E.4, N.2, Z.2): the movement orders, `Hold`, the
// screen, the supporter's station; else the search, then the hunt, then the
// waypoint walk.
fn patrol_arm(view, card, p) {
    let objective = p.objective;
    if objective == "advance" || objective == "hold" || objective == "screen" || p.station_on != () {
        this.last_plot = ();
        this.hunt = ();
        this.unheld_since_ms = t_ms();
    }
    switch objective {
        "advance" => {
            p.goal = p.order.to;
        }
        "hold" => {
            p.goal = p.here;
            p.stop = true;
        }
        // The screen: parked, faced onto the order's bearing, and the stand
        // over the arc — off the view's whole degrees (the divergence
        // `deg_round`).
        "screen" => {
            let facing = p.order.facing_mdeg;
            p.goal = p.here;
            p.stop = true;
            p.heading = facing % 360000;
            let row = stand(view, card, p, [facing, screen_half_arc_mdeg()]);
            if row != () {
                p = face(view, row, p.ranges.release_cm, p);
            }
        }
        _ => {
            if objective == "support" && p.station_on != () {
                // A supporter idle inside its leash keeps station.
                p = keep_station(view, p);
            } else {
                // The search, then the hunt, then the waypoint walk (16b
                // cuts E.4, O.1; no Hold spot reaches a production think).
                p = this.look(view, card, p);
            }
        }
    }
    p
}

// **A supporter keeping station** (16b cut Z.2): parked inside
// `station_cells()`, else the stand-off cell, and once there the plot.
fn keep_station(view, p) {
    let plot = p.station_on;
    if range_cm(plot) <= p.station_cm {
        p.goal = p.here;
        p.stop = true;
        return p;
    }
    let station = stand_off_cell(view, plot.id, p.station_cm);
    let next = station;
    if station != () && arrived(p, station) {
        let cell = plot.cell;
        if cell != () {
            next = cell;
        }
    }
    p.goal = if next == () { p.here } else { next };
    p
}

// Patrol searches, then pursues the Hold objective or hunts, then walks.
fn look(view, card, p) {
    let cell = this.search_goal(card, p);
    if cell == () {
        if view.objective != () {
            let held = this.hold_goal(view, p);
            if held != () { return held; }
        } else {
            cell = this.hunt_goal(view, card, p);
        }
    }
    if cell != () {
        p.goal = cell;
        return p;
    }
    p.goal = this.walk(view, p);
    p
}

// Hold geometry is public; capture ownership and clocks never reach the mind.
fn hold_goal(view, p) {
    if this.hold_retry_ms != () && t_ms() < this.hold_retry_ms {
        return ();
    }
    this.hold_retry_ms = ();
    if hold_arrived(view) {
        p.goal = p.here;
        p.stop = true;
    } else {
        p.goal = view.objective.cell;
    }
    this.hunt = ();
    this.unheld_since_ms = t_ms();
    p
}

// **`Patrol`'s first goal: where it last saw something** (T1's
// `search_goal`): the last plot while it is no older than the card's
// `search_ms` (inclusively) and not arrived at — answered, or `()`.
fn search_goal(card, p) {
    let plot = this.last_plot;
    if plot == () {
        return ();
    }
    let cell = [plot[0], plot[1]];
    // T1's `t_ms.saturating_sub(at) > search_ms` (the head's saturation).
    if card.search_ms == 0 || plot[2] < t_ms() - card.search_ms {
        this.last_plot = ();
        return ();
    }
    if arrived(p, cell) {
        this.last_plot = ();
        return ();
    }
    cell
}

// **`Patrol`'s second goal: the hunt** (T1's `hunt_goal`): the far zone
// once the card's `hunt_after_ms` has passed with nothing hostile held — the
// cell `far_entry_cell` answers, the motor fanning the landing — answered,
// or `()`.
fn hunt_goal(view, card, p) {
    if holds_a_hostile(view) {
        this.unheld_since_ms = ();
        this.hunt = ();
        return ();
    }
    let hunt = this.hunt;
    if hunt != () {
        if arrived(p, hunt) {
            // It went and looked: the walk resumes from the nearest node.
            this.hunt = ();
            this.unheld_since_ms = t_ms();
            this.waypoint = ();
            this.from_waypoint = ();
            return ();
        }
        return hunt;
    }
    if this.unheld_since_ms == () {
        this.unheld_since_ms = t_ms();
    }
    // T1's `t_ms.saturating_sub(since) < hunt_after_ms` (the head's
    // saturation).
    if card.hunt_after_ms == 0 || this.unheld_since_ms > t_ms() - card.hunt_after_ms {
        return ();
    }
    let cell = if p.here == () { () } else { far_entry_cell(view) };
    if cell == () {
        this.unheld_since_ms = t_ms();
        return ();
    }
    this.hunt = cell;
    cell
}

// **The waypoint walk** (T1's `walk`): `patrol_step` keyed on the view's own
// unit, with the motor's arrival word for a fanned node; the next goal
// answered.
fn walk(view, p) {
    let step = patrol_step(view, p.reached != () && p.reached == p.goal, this.waypoint, this.from_waypoint, p.goal);
    this.waypoint = step.waypoint;
    this.from_waypoint = step.from_waypoint;
    step.goal
}

// **`Approach`**: every range, goal and heading off the hull's own row of
// its target — the plot, at the view's resolution. Inside the public Hold
// square it holds its ground and uses the release band.
fn approach_arm(view, p) {
    let target = this.target;
    if target == () {
        return p;
    }
    let row = contact(view, target);
    let range = range_cm(row);
    let band = p.ranges;
    let inside = hold_contains(view, p.here);
    let close_edge = if view.immobile || inside { band.release_cm } else { band.engage_cm };
    if band.limit_cm != 0 && range <= max(close_edge, 1) {
        let line = if row == () { true } else { line_to(view, target) };
        if line {
            this.state = 3;
            p.goal = ();
            if row != () {
                p = engage_orders(view, row, band.release_cm, p);
            }
        } else if inside {
            p.goal = p.here;
        } else if row != () && row.cell != () {
            p.goal = row.cell;
        } else {
            p.goal = p.here;
        }
    } else if inside {
        p.goal = p.here;
    } else {
        let want = stand_off_want(band.engage_cm, max(cell_cm(), 1), range);
        let cell = if row == () { () } else { stand_off_cell(view, target, want) };
        p.goal = if cell == () { p.here } else { cell };
    }
    p
}

// **`Engage`**: past the release edge, back to `Approach` and the stand-off
// cell; inside it, holding the band — stop, turn onto the plot, lock.
fn engage_arm(view, p) {
    let target = this.target;
    if target == () {
        return p;
    }
    let row = contact(view, target);
    let range = range_cm(row);
    let band = p.ranges;
    if band.limit_cm == 0 || range > band.release_cm {
        this.state = 2;
        let want = stand_off_want(band.engage_cm, max(cell_cm(), 1), range);
        let cell = if row == () { () } else { stand_off_cell(view, target, want) };
        p.goal = if cell == () { p.here } else { cell };
    } else {
        p.goal = p.here;
        p.stop = true;
        p.heading = if row == () { () } else { row.bearing_deg * 1000 };
        if row != () {
            p = engage_orders(view, row, band.release_cm, p);
        }
    }
    p
}

// **`Withdraw`** (16b cut Q.7): home, the stand, the dwell, the rally.
fn withdraw_arm(view, card, p) {
    if p.goal == () {
        let entry = if p.here == () { () } else { entry_cell(view) };
        p.goal = if entry == () { p.here } else { entry };
    }
    let home = view.immobile || p.goal == () || (p.here != () && arrived(p, p.goal));
    let row = withdraw_stand(view, card, p, home);
    if row != () {
        // Cornered: the dwell is `stand_ticks()` thinks of the doctrine's
        // cadence. 17b cut M1b.2: the ceiling `withdraw_stand` admitted the
        // row by — `release_cm` home, `stand_cm` en route.
        let ceiling = if home { p.ranges.release_cm } else { p.ranges.stand_cm };
        p = face(view, row, ceiling, p);
        // T1's `t_ms.saturating_add(..)` (the head's saturation).
        let dwell = stand_ticks() * decide_ms();
        this.stand_until_ms = min(t_ms(), 9223372036854775807 - dwell) + dwell;
        this.quiet_since_ms = ();
        p.stop = true;
        return p;
    }
    if this.stand_until_ms != () && t_ms() < this.stand_until_ms {
        this.quiet_since_ms = ();
        p.stop = true;
        return p;
    }
    this.stand_until_ms = ();
    if holds_a_hostile(view) || !home || p.dry || p.objective == "withdraw" {
        this.quiet_since_ms = ();
        return p;
    }
    if this.quiet_since_ms == () {
        this.quiet_since_ms = t_ms();
    }
    // The card's `rally_ms` 0 never rallies (`StockCard::as_reflex` writes it
    // as `u32::MAX`, which no match reaches). T1's
    // `t_ms.saturating_sub(since) >= rally_ms` (the head's saturation).
    if card.rally_ms != 0 && this.quiet_since_ms <= t_ms() - card.rally_ms {
        this.state = 1;
        this.rallied = true;
        p.goal = ();
        this.quiet_since_ms = ();
    }
    p
}

// ---------------------------------------------------------------------------
// The plot stamps and the emission
// ---------------------------------------------------------------------------

// **What this think saw, and how long it has seen nothing** (16b cut E.4).
fn stamps(view, p) {
    let t = t_ms();
    if p.stood != () {
        this.last_plot = [p.stood[0], p.stood[1], t];
    } else if (this.state == 2 || this.state == 3) && this.target != () {
        let row = contact(view, this.target);
        if row != () && row.cell != () {
            let cell = row.cell;
            this.last_plot = [cell[0], cell[1], t];
        }
    }
    if this.state != 1 {
        this.hunt = ();
        this.unheld_since_ms = t;
    }
}

// **The emission, in T1's order**: the slots, then the helm half — `hold()`
// and the facing `helm(mdeg)` for a stand, `goto([x, y])` only when the goal
// CHANGED, `hold()` when a held goal is dropped — then the lock, then the
// citation. A hull whose motion is disabled is handed no goal and no
// heading.
fn emit(view, card, p) {
    // 17b cut M1b.2: the slots first, after `Startup` and before `Goal`.
    arming_for(view, card, p.arm_ceiling);
    let held = p.held;
    if view.immobile {
        held = ();
    } else if p.stop {
        hold();
        held = ();
        if p.heading != () {
            helm(p.heading);
        }
    } else if p.goal != () {
        if held != p.goal {
            goto(p.goal);
        }
        held = p.goal;
    } else if held != () {
        hold();
        held = ();
    }
    this.goal = held;
    if p.lock != () {
        lock(p.lock);
    }
    // The citation `engage_orders` planned beside a fresh lock, resolved
    // right after the lock lands.
    if p.fire != () {
        fire(p.fire[0], p.fire[1]);
    }
}

// **T1's slots for this think** (`arming_for`; 17b cuts M1b.2, M1b.3;
// D-UM-16, D-UM-18, D-UM-43): what the stock mind arms or disarms, emitted
// after `startup()` and before the helm half — answers nothing.
//
//   * `ceiling` an INT — `engage_orders` was reached with a row and its
//     gates passed: one `arm(weapon, "best", card.min_odds_permille,
//     aspect_mask(card.aspect), keeps, 0, min(ceiling, reach) / 100)` per
//     weapon of the first `slots()`, in component order, WHATEVER THE HEAT
//     (the resolver holds a hot slot's shot silently and the slot stays
//     armed: a heat skip here would put the numbering out of step with the
//     arena's), `keeps` the card's `reserve_pct` of the weapon's
//     `rounds_max` and `reach` the last of its `bands_cm` (0 for none);
//   * `ceiling` `()` — one `disarm(())` when some row is armed, else
//     nothing;
//   * a hull with no weapon rows — nothing (D-UM-48).
//
// Emitted ON CHANGE (D-UM-4's parity rule: a mind reads its own armed slots
// off its view exactly as a pilot reads them off `WEAPONSTATUS`): an `arm`
// only for a row whose `armed` is `()` or differs in any term but the slot
// NUMBER, which is the arena's and is not compared (T1's
// `FrameArming::of(armed.slot, &wanted) == armed`).
fn arming_for(view, card, ceiling) {
    let weapons = view.weapons;
    if weapons.len() == 0 {
        return;
    }
    if ceiling == () {
        // 17b cut M1b.3: nothing to clear is nothing said.
        for weapon in weapons {
            if weapon.armed != () {
                disarm(());
                return;
            }
        }
        return;
    }
    // The card's terms and the slot count, read once (each read is
    // operations a think pays: 17b §1 (iv)).
    let floor = card.min_odds_permille;
    let mask = aspect_mask(card.aspect);
    let reserve = card.reserve_pct;
    let n = slots();
    for (weapon, i) in weapons {
        if i >= n {
            break;
        }
        let reach = weapon.bands_cm;
        reach = if reach.len() == 0 { 0 } else { reach[-1] };
        // T1's saturating reserve, held to a u32 (the head's saturation);
        // no reserve (Line's) keeps nothing, whatever the magazine.
        let keeps = 0;
        if reserve != 0 {
            let rounds_max = weapon.rounds_max;
            if rounds_max != 0 {
                keeps = if reserve > 9223372036854775807 / rounds_max { 4294967295 } else { min(reserve * rounds_max / 100, 4294967295) };
            }
        }
        let under_m = min(ceiling, reach) / 100;
        let armed = weapon.armed;
        if armed == () || armed.target != "best" || armed.min_permille != floor || armed.aspects != mask || armed.keeps != keeps || armed.min_range_m != 0 || armed.max_range_m != under_m {
            arm(weapon.id, "best", floor, mask, keeps, 0, under_m);
        }
    }
}

// **The card's aspect word as a slot's facet mask** (T1's
// `AspectWord::mask`): `any` every facet (15), `flank` the two sides (6),
// `rear` the rear (8).
fn aspect_mask(word) {
    switch word {
        "flank" => 6,
        "rear" => 8,
        _ => 15,
    }
}
