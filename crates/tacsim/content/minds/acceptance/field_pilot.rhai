// Field Pilot: an independent public-picture port of mush-bot's shipped pilot.
// No stock planning helper, server pose, hidden target heading or shared bot code.
// Class II: 500 ms, 3000 charged operations, four native gun slots.
// Memory is flat (55 keys); all persisted arrays contain at most 16 INTs.
// terrain_at returns [public_kind, tracked_passable], never occupancy/slope.
const MIND = #{name:"Field Pilot", api:6, class:"discretion", role:"combat", core:"II"};

fn turn(h) { (h % 360000 + 360000) % 360000 }
fn point(v, h, metres) {
    let d = metres * 100;
    [clamp((v.x_cm + sin_m(h) * d / 1048576) / cell_cm(), 0, 65535),
     clamp((v.y_cm + cos_m(h) * d / 1048576) / cell_cm(), 0, 65535)]
}
fn init(v) {
    this.born = t_ms(); this.target = (); this.last = (); this.before = ();
    this.lock_at = -1500; this.hit_at = -10000; this.hit_heading = (); this.hit_heading_at = -10000;
    this.armor = [0,0,0,0]; this.damage_seen = false; this.dead_track = 0;
    this.turn_until = 0; this.turn_heading = 0; this.hurt = false;
    this.mode = 0; this.until_ms = 0; this.follow = false; this.hits = 0;
    this.fallen_back = false; this.hold_motion = 0;
    this.broken = false; this.stopped_at = (); this.last_range = 0;
    this.range_at = 0; this.goal = (); this.goal_at = 0; this.sweep = 0;
    this.open = v.power != "online"; this.open_goal = (); this.open_at = 0;
    this.open_legs = 0; this.open_phase = 0; this.seen_hostile = false;
    this.unknown_until = 0; this.unknown_next = 0; this.unknown_count = 0;
    this.unknown_heading = 0; this.held_since = (); this.escape_step = 0;
    this.escape_until = 0; this.escape_heading = 0; this.escape_speed = 0;
    this.marks_x = []; this.marks_y = []; this.mark_index = 0;
    this.reapproaches = 0; this.reapproach_next = 0; this.ambush_used = false;
    this.command_heading = (); this.command_speed = (); this.quiet = t_ms();
    this.sense = 1; this.flip_at = t_ms();
    let b=spot_cell("B");
    this.search_axis=if b == () { v.heading_mdeg } else { bearing_to(v,b) };
    this.search_from=();
}
// Only own damage and public notices. The facet comparison substitutes for the
// bot's rendered own-hit facet; it never reads a hidden attacker's position.
fn observe(v) {
    let armor = [0,0,0,0]; this.hurt = false; this.dead_track = 0;
    for s in v.sections {
        if s.id == "hull" {
            if s.internals != () && s.internals_max > 0 && s.internals * 2 <= s.internals_max { this.hurt = true; }
            for f in s.facets {
                let i = switch f.id { "front" => 0, "right" => 1, "rear" => 2, "left" => 3, _ => -1 };
                if i >= 0 {
                    armor[i] = f.armor;
                    if f.armor_max > 0 && f.armor * 4 <= f.armor_max { this.hurt = true; }
                    if i > 0 && this.damage_seen && f.armor < this.armor[i] { this.hit_heading = turn(v.heading_mdeg + i * 90000); this.hit_heading_at=t_ms(); }
                }
            }
        }
        if s.condition == "lost" {
            if s.id == "left_track" { this.dead_track = -1; }
            if s.id == "right_track" { this.dead_track = 1; }
        }
    }
    this.armor = armor; this.damage_seen = true;
    // Damage starts one retreat per sortie, even before a target is held.
    if this.hurt && !this.fallen_back {
        this.fallen_back = true; this.mode = 1; this.until_ms = t_ms()+10000;
    }
    for n in v.notices {
        if n.kind == "struck" { this.hit_at = t_ms(); this.open = false; }
        if n.contact == this.target {
            if n.kind == "hit" { this.hits += 1; }
            if n.kind == "target_section_lost" { this.broken = true; }
            if n.kind == "kill" { this.target = (); this.last = (); this.follow = false; }
        }
    }
}
fn choose(v) {
    let best = (); let old = (); let unknown = ();
    for c in v.contacts {
        if !c.identified {
            if c.age_ms <= 3000 && c.range_m > 100 && (unknown == () || c.range_m < unknown.range_m) { unknown = c; }
        } else if !c.friendly && c.reads_dead != true && c.age_ms <= 3000 {
            if c.id == this.target { old = c; }
            if best == () || c.range_m < best.range_m { best = c; }
        }
    }
    if old != () && (best == () || best.range_m + 100 > old.range_m) { best = old; }
    if best == () && unknown != () && t_ms() >= this.unknown_next && this.unknown_count < 4 {
        this.unknown_until = t_ms() + 6000; this.unknown_next = t_ms() + 12000;
        this.unknown_count += 1; this.unknown_heading = unknown.bearing_deg * 1000;
    }
    if best != () {
        if best.id != this.target {
            this.hits = 0; this.broken = false; this.before = (); this.last = ();
            this.follow = false; this.ambush_used = false; this.sweep = 0;
        }
        this.target = best.id; this.seen_hostile = true; this.quiet = t_ms();
        let at = best.cell; let plotted = t_ms() - best.age_ms;
        if at != () && (this.last == () || plotted > this.last[2]) {
            this.before = this.last; this.last = [at[0],at[1],plotted];
        }
    }
    best
}
// A velocity estimate is made only from two published cells. Reject impossible
// jumps (>20 m/s); sparse/no track remains unavailable, not stationary truth.
fn course() {
    if this.before == () || this.last == () { return (); }
    let dt = this.last[2] - this.before[2];
    if dt < 1000 || dt > 20000 { return (); }
    let dx = (this.last[0] - this.before[0]) * cell_cm();
    let dy = (this.last[1] - this.before[1]) * cell_cm();
    let d = isqrt(dx*dx + dy*dy);
    if d == 0 || d * 1000 > dt * 2000 { return (); }
    [dx * 1000 / dt, dy * 1000 / dt]
}
fn guns(v, target, authored) {
    if authored { return; }
    let enabled = target != () && v.power == "online";
    if v.order != () && v.order.hold_fire { enabled = false; }
    let used = 0;
    for w in v.weapons {
        if used >= slots() { break; }
        if !enabled || w.rounds == 0 {
            if w.armed != () { disarm(w.id); }
        } else {
            let bands = w.bands_cm;
            let ceiling = if bands.len() == 0 { 0 } else { bands[bands.len()-1] / 100 };
            if w.armed == () || w.armed.target != "lock" || w.armed.min_permille != v.odds_floor_permille || w.armed.max_range_m != ceiling {
                arm(w.id,"lock",v.odds_floor_permille,15,0,0,ceiling);
            }
            used += 1;
        }
    }
}
// Hysteresis follows the requested movement, not acceleration or braking.
fn band_speed(distance, band, forward, reverse) {
    if (this.hold_motion == 1 && distance <= band+50) || (this.hold_motion == -1 && distance >= band-50) { this.hold_motion=0; }
    if this.hold_motion == 0 {
        if distance > band+75 { this.hold_motion=1; }
        else if distance < band-75 { this.hold_motion=-1; }
    }
    if this.hold_motion == 1 { forward }
    else if this.hold_motion == -1 { -reverse }
    else { 0 }
}
fn arrived(v, goal) { v.motor != () && v.motor.goal == goal && v.motor.outcome == "arrived" }
fn drive(v, goal) {
    if goal == () { return; }
    if this.goal != goal || (t_ms() - this.goal_at >= 4000 && (v.motor == () || v.motor.outcome == "no_route")) {
        goto(goal); this.goal = goal; this.goal_at = t_ms();
        this.command_heading = (); this.command_speed = ();
    }
}
fn motion(v, heading, velocity) {
    if v.immobile { return; }
    if this.goal != () { goto_off(); this.goal = (); }
    let h = turn(heading);
    if this.dead_track != 0 {
        let delta = turn(h - v.heading_mdeg + 180000) - 180000;
        if delta * this.dead_track > 0 {
            if abs(delta) <= 45000 { h = v.heading_mdeg; }
            else if t_ms() >= this.turn_until { this.turn_heading = h; this.turn_until = t_ms() + 12000; }
        }
        if t_ms() < this.turn_until && arc(v.heading_mdeg,this.turn_heading) > 10000 { h = this.turn_heading; }
    }
    if this.command_heading != h { helm(h); this.command_heading = h; }
    if this.command_speed != velocity { speed(velocity); this.command_speed = velocity; }
}
// Mark public impassable ground only. Occupied/unknown holds are short detours,
// never a permanent observation about another hull.
fn escape(v, forward, reverse) {
    if !v.held { this.held_since = (); return false; }
    if this.held_since == () {
        this.held_since = t_ms(); this.escape_step = 0;
        let mark = point(v,v.heading_mdeg,20);
        let tile = terrain_at(mark);
        if tile != () && !tile[1] {
            if this.marks_x.len() < 16 { this.marks_x.push(mark[0]); this.marks_y.push(mark[1]); }
            else { this.marks_x[this.mark_index] = mark[0]; this.marks_y[this.mark_index] = mark[1]; this.mark_index = (this.mark_index + 1) % 16; }
        }
        this.escape_until = 0;
    }
    if t_ms() >= this.escape_until {
        this.escape_heading = turn(v.heading_mdeg + 45000 + min(this.escape_step,3)*45000);
        this.escape_speed = if this.escape_step >= 3 { -reverse } else { forward };
        this.escape_step = (this.escape_step+1)%4; this.escape_until = t_ms()+3000;
    }
    this.motion(v,this.escape_heading,this.escape_speed); true
}
fn detour(v, heading) {
    let h = heading;
    for n in 0..this.marks_x.len() {
        let at = [this.marks_x[n],this.marks_y[n]];
        let d = range_to(v,at);
        if d >= 5000 && d <= 12000 && arc(h,bearing_to(v,at)) < 15000 { h = turn(h+45000); }
    }
    h
}
fn hiding(tile) { tile != () && tile[1] && tile[0] in ["grass","ground","rubble","woods_light"] }
// Nine charged probes at most: retreat corridor at 100/200/290 m, then
// lateral cover at 40/80 m. Every probe is within the local public-map window.
fn opening(v, target, enabled) {
    if !this.open || !enabled || v.mode != "skirmish" { return false; }
    let far = far_entry_cell(v);
    if far == () { this.open = false; return false; }
    let axis = bearing_to(v,far);
    if target != () || this.hit_at >= this.born || t_ms()-this.born >= 360000 {
        this.open = false;
        if target != () && target.range_m < 600 { this.mode = 4; this.until_ms = t_ms()+60000; this.range_at = t_ms(); this.last_range = target.range_m; }
        return false;
    }
    if this.open_goal != () {
        if arrived(v,this.open_goal) || t_ms()-this.open_at > 60000 { this.open_goal = (); this.open_phase += 1; }
        else { this.drive(v,this.open_goal); return true; }
    }
    if this.open_phase == 0 && this.open_legs < 3 {
        let room = 290;
        for d in [100,200,290] {
            let tile = terrain_at(point(v,turn(axis+180000),d));
            if tile == () || !tile[1] { room = min(room,d-100); }
        }
        if room < 250 {
            let goal = point(v,axis,290-room);
            let tile = terrain_at(goal);
            if tile != () && tile[1] { this.open_goal=goal; this.open_at=t_ms(); this.open_legs+=1; this.drive(v,goal); return true; }
        }
        this.open_phase = 1;
    }
    if this.open_phase == 1 {
        let here = terrain_at(v.cell);
        if !hiding(here) {
            for offset in [90000,-90000] {
                for d in [40,80] {
                    let at = point(v,turn(axis+offset),d);
                    if hiding(terrain_at(at)) { this.open_goal=at; this.open_at=t_ms(); this.open_phase=2; this.drive(v,at); return true; }
                }
            }
        }
        this.open_phase = 3;
    }
    this.motion(v,axis,0); true
}
fn lost(v, forward, reverse) {
    if this.last == () || t_ms()-this.last[2] > 90000 { return false; }
    let at = [this.last[0],this.last[1]];
    if this.follow {
        if this.mode != 2 && this.mode != 3 { this.mode=2; this.until_ms=t_ms()+65000; }
    }
    if this.mode == 2 {
        if range_to(v,at) >= 65000 || t_ms() >= this.until_ms { this.mode=3; this.until_ms=t_ms()+90000; }
        else { this.motion(v,bearing_to(v,at),-reverse); return true; }
    }
    if this.mode == 3 {
        if t_ms() < this.until_ms { this.motion(v,bearing_to(v,at),0); return true; }
        this.mode=0; this.follow=false;
    }
    if !this.ambush_used && !this.hurt && range_to(v,at) <= 70000 {
        this.ambush_used=true; this.mode=5; this.until_ms=t_ms()+20000;
    }
    if this.mode == 5 && t_ms() < this.until_ms { this.motion(v,bearing_to(v,at),0); return true; }
    let course=this.course(); let goal=at;
    if course != () {
        let age=min(t_ms()-this.last[2],3000);
        goal=[clamp(at[0]+course[0]*age/1000/cell_cm(),0,65535),clamp(at[1]+course[1]*age/1000/cell_cm(),0,65535)];
    }
    if range_to(v,goal) <= 10000 || arrived(v,goal) {
        let sweep_heading = (this.sweep%4)*90000;
        goal=point(v,sweep_heading,300);
        if this.goal == () || arrived(v,this.goal) { this.sweep+=1; }
    }
    this.drive(v,goal); true
}
// A front hit is not a turn. A held target already owns the bow unless a
// different earned hostile supports the struck side/rear arc. No shooter pose.
fn threat_heading(v, target, stance) {
    if stance == "orbit" || this.hit_heading == () || t_ms()-this.hit_heading_at >= 4000 { return (); }
    let other=();
    for c in v.contacts {
        if c.identified && !c.friendly && c.reads_dead != true && c.age_ms <= 6000 && (target == () || c.id != target.id) && arc(c.bearing_deg*1000,this.hit_heading) <= 45000 {
            if other == () || c.range_m < other.range_m { other=c; }
        }
    }
    if other == () && target != () { return (); }
    let h=if other == () { this.hit_heading } else { other.bearing_deg*1000 };
    if target != () && arc(h,target.bearing_deg*1000) <= 30000 { return (); }
    let delta=turn(h-v.heading_mdeg+180000)-180000;
    if this.dead_track != 0 && delta*this.dead_track > 0 && abs(delta) < 180000 { return (); }
    h
}
fn fight(v, target, card, forward, reverse, previous_hold) {
    let h=target.bearing_deg*1000; let band=card.get("band_m");
    let stance=card.get("stance"); let velocity=0;
    if this.fallen_back {
        band=max(band,min(450,band+150));
        if stance == "close" { stance="hold"; }
    }
    if this.mode == 1 {
        if t_ms() >= this.until_ms || target.range_m >= band { this.mode=0; }
        else { this.motion(v,h,-reverse); return; }
    }
    if stance == "withdraw" && this.hurt { this.motion(v,h,-reverse); return; }
    if card.get("reapproach") && this.hurt && this.reapproaches < 2 && t_ms() >= this.reapproach_next {
        this.mode=2; this.until_ms=t_ms()+60000; this.reapproaches+=1; this.reapproach_next=t_ms()+135000;
    }
    if this.mode == 2 {
        if target.range_m >= 650 || t_ms() >= this.until_ms { this.mode=3; this.until_ms=t_ms()+45000; }
        else { this.motion(v,h,-reverse); return; }
    }
    if this.mode == 3 && t_ms() < this.until_ms { this.motion(v,h,0); return; }
    let threat=this.threat_heading(v,target,stance);
    if threat != () { this.motion(v,threat,0); return; }
    if this.mode == 4 {
        if target.range_m < this.last_range { this.range_at=t_ms(); this.last_range=target.range_m; }
        if target.range_m <= band || t_ms() >= this.until_ms || t_ms()-this.range_at >= 6000 || this.hit_at >= this.born { this.mode=0; }
        else { this.motion(v,h,0); return; }
    }
    if card.get("follow") && (this.broken || this.hits >= 8) && target.moving {
        if this.range_at == 0 { this.range_at=t_ms(); this.last_range=target.range_m; }
        if t_ms()-this.range_at >= 2000 && target.range_m-this.last_range >= 100 { this.follow=true; }
        if t_ms()-this.range_at >= 10000 { this.range_at=t_ms(); this.last_range=target.range_m; }
    }
    if this.follow {
        let far=far_entry_cell(v);
        if far != () && range_to(v,far) < 75000 { this.mode=2; this.until_ms=t_ms()+65000; this.motion(v,h,-reverse); return; }
        band=350;
        if !target.moving {
            if this.stopped_at == () { this.stopped_at=t_ms(); }
            if t_ms()-this.stopped_at >= 5000 { this.follow=false; }
        } else { this.stopped_at=(); }
    }
    if stance == "close" { band=200; }
    if stance == "orbit" {
        let sense=card.get("sense");
        if sense == "cw" { this.sense=-1; }
        else if sense == "ccw" { this.sense=1; }
        else {
            let course=this.course();
            if course != () && target.cell != () {
                let relative_x=v.x_cm-(target.cell[0]*cell_cm()+cell_cm()/2);
                let relative_y=v.y_cm-(target.cell[1]*cell_cm()+cell_cm()/2);
                // Sign of rear-course versus the pilot as the target sees it;
                // two public plotted cells, never a hidden target heading.
                this.sense=if -course[0]*relative_y+course[1]*relative_x > 0 { -1 } else { 1 };
                this.flip_at=t_ms();
            } else if t_ms()-this.flip_at >= 20000 { this.sense=-this.sense; this.flip_at=t_ms(); }
        }
        let sign=this.sense;
        if this.dead_track != 0 { sign=-this.dead_track; }
        h=turn(h+sign*(90000-clamp((target.range_m-band)*60000/150,-60000,60000)));
        velocity=forward;
    } else { this.hold_motion=previous_hold; velocity=this.band_speed(target.range_m,band,forward,reverse); }
    if velocity > 0 { h=this.detour(v,h); }
    this.motion(v,h,velocity);
}
// Public Crossroads (B), within 100 m, then 800 m along the original line
// through it before turning back. No named spot is a hidden-unit oracle.
fn search(v, forward) {
    let b=spot_cell("B");
    if b == () { let far=far_entry_cell(v); if far != () { this.drive(v,far); } return; }
    if this.search_from != () {
        let dx=v.x_cm-this.search_from[0]; let dy=v.y_cm-this.search_from[1];
        if dx*dx+dy*dy < 80000*80000 { this.motion(v,this.search_axis,forward); return; }
        this.search_from=();
    }
    if range_to(v,b) <= 10000 { this.search_from=[v.x_cm,v.y_cm]; this.motion(v,this.search_axis,forward); }
    else { this.motion(v,bearing_to(v,b),forward); }
}
fn think(v,card) {
    if !("born" in this) { this.init(v); }
    // Any branch which does not fly the hold leaves it stopped next think.
    let previous_hold=this.hold_motion; this.hold_motion=0;
    if v.wreck || v.cell == () { return; }
    if v.power != "online" { if v.power == "off" { startup(); } return; }
    let authored=stock::rules();
    this.observe(v); let target=this.choose(v);
    this.guns(v,target,authored);
    if target != () && v.lock != target.id && t_ms()-this.lock_at >= 1500 { lock(target.id); this.lock_at=t_ms(); }
    let forward=card.get("forward_cms"); let reverse=card.get("reverse_cms");
    if v.immobile { return; }
    if this.escape(v,forward,reverse) { return; }
    if this.opening(v,target,card.get("opening")) { return; }
    if target != () { this.fight(v,target,card,forward,reverse,previous_hold); return; }
    let threat=this.threat_heading(v,(),card.get("stance"));
    if threat != () { this.motion(v,threat,0); return; }
    if t_ms() < this.unknown_until { this.motion(v,this.unknown_heading,0); return; }
    if this.lost(v,forward,reverse) { return; }
    if v.mode == "hold" && v.objective != () {
        if hold_contains(v,v.cell) { this.motion(v,v.heading_mdeg,0); }
        else { this.drive(v,v.objective.cell); }
        return;
    }
    if t_ms()-this.quiet < 5000 { this.motion(v,v.heading_mdeg,0); return; }
    this.search(v,forward);
}
