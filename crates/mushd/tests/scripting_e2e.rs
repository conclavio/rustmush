//! The M5 "scripted room trigger" demo as one end-to-end journey over real
//! SSH (in-memory store, ungated): the full server wiring — world task,
//! session layer, accounts, SSH listener — **with the standard M5 modules**
//! (`mush-script` among them) exactly as `main.rs` boots them
//! (`Engine::with_modules` → `boot_modules`), then a builder authoring Rhai
//! scripts and a mortal player triggering them, all asserted on the rendered
//! ANSI the clients actually read.
//!
//! Proves, over the wire:
//!
//! - a `script:command:*` `$`-command (`greet`) authored with `@set`
//!   pemits the triggering caller a personalised greeting;
//! - a `script:enter` room trigger announces arrivals when a player walks
//!   an exit into the room;
//! - a runaway script (`while true {}`) is aborted by the sandbox budget,
//!   and after three strikes the object is auto-`HALTED` — the owner sees
//!   every strike notice and the halt, and the object then goes inert.

mod support;

use support::{
    boot_module_server, login_create, parse_id_after, play_client, scratch_dir, strip_ansi,
};

const GM_PASSWORD: &str = "gm-secret-1";
const BANNER: &str = "Welcome to the scripting e2e MUSH!";
#[tokio::test(flavor = "multi_thread")]
async fn scripting_room_trigger_demo() {
    let dir = scratch_dir("scripting-e2e");
    let addr = boot_module_server(&dir, BANNER, GM_PASSWORD).await;

    // --- God (the builder) + a mortal player. ---------------------------
    let (_god_handle, mut god) = play_client(addr).await;
    god.expect(BANNER).await;
    god.send_line(&format!("connect boss {GM_PASSWORD}")).await;
    god.expect("Connected as God").await;
    god.expect("Limbo(#0)").await;

    let (_alice_handle, mut alice) = login_create(addr, "Alice", "secret123").await;
    god.expect("Alice has connected.").await;

    // ============ 1. a $-command greets its caller ============
    // Author the script with @set (the AttrSet signal recompiles it), then
    // drop the object into Limbo so a co-located player can trigger it.
    god.send_line("@create greeter").await;
    god.expect("Created: greeter(#").await;
    god.send_line(
        r#"@set greeter/script:command:greet=mush::pemit(mush::caller(), "Welcome, " + mush::name(mush::caller()) + "!")"#,
    )
    .await;
    god.expect("script:command:greet - Set.").await;
    god.send_line("drop greeter").await;
    god.expect("You drop greeter.").await;

    // Alice (in Limbo, where the greeter now sits) triggers it and is
    // greeted by name — the script ran as its owner and pemitted the caller.
    let mark = alice.mark();
    alice.send_line("greet").await;
    alice.expect_after(mark, "Welcome, Alice!").await;

    // A $-command is a fallback, never a shadow: the built-in `say` still
    // wins even with a greeter present.
    let mark = alice.mark();
    alice.send_line("\"still me").await;
    alice.expect_after(mark, "Alice says, \"still me\"").await;

    // ============ 2. a script:enter announces arrivals ============
    god.send_line("@dig Gallery=gallery,out").await;
    god.expect("Gallery created with room number #").await;
    let gallery = parse_id_after(&strip_ansi(&god.seen), "Gallery created with room number #");
    god.send_line(&format!(
        "@set #{gallery}/script:enter=mush::emit(mush::me(), mush::name(mover) + \" strolls in.\")"
    ))
    .await;
    god.expect("script:enter - Set.").await;

    // Alice walks the exit into the Gallery; the room's enter trigger fires
    // and everyone present (Alice included) reads the announcement.
    let mark = alice.mark();
    alice.send_line("gallery").await;
    alice.expect_after(mark, "Gallery(#").await;
    alice.expect_after(mark, "Alice strolls in.").await;

    // ============ 3. a runaway script is halted after 3 strikes =======
    // The owner (God, also a wizard) sees every strike notice.
    god.send_line("@create bomb").await;
    god.expect("Created: bomb(#").await;
    god.send_line("@set bomb/script:command:spin=while true {}")
        .await;
    god.expect("script:command:spin - Set.").await;

    // Strikes 1 and 2: aborted, the owner is told it failed.
    for _ in 0..2 {
        let mark = god.mark();
        god.send_line("spin").await;
        god.expect_after(mark, "failed").await;
    }
    // Strike 3 trips the auto-disable; the owner sees the HALTED notice.
    let mark = god.mark();
    god.send_line("spin").await;
    god.expect_after(mark, "has been HALTED after 3 script budget overruns")
        .await;

    // The object is now inert: its $-command no longer resolves, so the
    // line falls through to the classic `Huh?`.
    let mark = god.mark();
    god.send_line("spin").await;
    god.expect_after(mark, "Huh?").await;
}
