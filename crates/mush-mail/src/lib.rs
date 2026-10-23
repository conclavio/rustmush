//! # mush-mail — the `@mail` module
//!
//! Classic MUSH mail on the merged module API (design doc 04 §8, treaty
//! ruling 4): flat messages with a `reply_to` chain (threads are a
//! client-side derivation), named folders, quotas, compose via the shared
//! input-context stack, and soft delete.
//!
//! - **Data**: each message is one [`MailMsg`] stored once; per-recipient
//!   state is a [`MailboxEntry`] (`folder`, `status`, `read_at`). Folders
//!   are `inbox` / `sent` / `archive` plus up to 16 user-created ones and
//!   a hidden `trash` (soft delete). Live state is one [`MailState`] slot
//!   on the `World`; every mutation writes through as [`ModuleOp`]s in the
//!   same batch as the command's object writes, and boot rebuilds the slot
//!   from the persisted documents in
//!   [`GameModule::on_world_loaded`].
//! - **Commands**: `@mail` (list), `@mail <n>` (read), and the
//!   `/send /quick /reply /replyall /fwd /file /folder /newfolder
//!   /rmfolder /delete /purge /unread /urgent` switches — see the in-game
//!   help. Compose retention, message size, and recipient fan-out all have
//!   hard bounds; persisted messages are validated against the same bounds.
//! - **Notifications**: a live [`OutputEvent::MailNotice`] per delivery
//!   and a connect-time summary line via the `Connected` hook.
//! - **Timezone policy**: timestamps are Unix UTC seconds; the terminal
//!   renders UTC ([`date`]), the web client localizes later.
//!
//! [`ModuleOp`]: mush_core::ModuleOp
//! [`OutputEvent::MailNotice`]: mush_proto::OutputEvent::MailNotice
//! [`GameModule::on_world_loaded`]: mush_core::GameModule::on_world_loaded

// WS01-S12: the panic-shaped restriction lints this crate measures zero of,
// held at zero. `crates/mush-contract-tests/tests/restriction_lints.rs` carries
// the measurement this set was cut from — including the counts of the lints
// NOT named here — and fails if this attribute loses one of them.
#![deny(
    clippy::unwrap_used,
    clippy::panic,
    clippy::unreachable,
    clippy::panic_in_result_fn
)]

use std::sync::Arc;

use mush_core::module::{EventHook, HookPoint, WorldEvent, WorldEventKind};
use mush_core::{
    ArgShape, CommandSpec, Ctx, GameModule, ModError, ModuleBootData, ModuleCtx, ObjId, Registrar,
};

mod cmds;
pub mod data;
pub mod date;
pub mod query;
pub mod transit;

pub use data::{
    ARCHIVE, DEFAULT_QUOTA, INBOX, MAX_FOLDER_NAME, MAX_MAIL_BODY_BYTES, MAX_MAIL_BODY_LINES,
    MAX_MAIL_RECIPIENTS, MAX_MAIL_SUBJECT_BYTES, MAX_USER_FOLDERS, MailMsg, MailState, MailStatus,
    Mailbox, MailboxEntry, NS, SENT, TRASH,
};
pub use date::{format_date, format_datetime};
pub use query::ListQuery;

/// The `@mail` game module. Hand to
/// [`Engine::with_modules`](mush_core::engine::Engine::with_modules).
#[derive(Debug, Default)]
pub struct MailModule;

impl MailModule {
    #[must_use]
    pub fn with_transport(self, transport: Arc<dyn transit::Transport>) -> TransportMailModule {
        TransportMailModule { transport }
    }
}
#[derive(Debug)]
pub struct TransportMailModule {
    transport: Arc<dyn transit::Transport>,
}
impl GameModule for TransportMailModule {
    fn name(&self) -> &'static str {
        NS
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        MailModule.register(reg)?;
        reg.state(transit::Services(Arc::clone(&self.transport)));
        Ok(())
    }
    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        MailModule.on_world_loaded(ctx, data)
    }
}

impl GameModule for MailModule {
    fn name(&self) -> &'static str {
        NS
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        reg.state(MailState::default());
        reg.tick(
            transit::TICK,
            mush_core::module::TickCadence::Coarse(std::time::Duration::from_secs(1)),
            Arc::new(transit::settle),
        )?;
        reg.hook(HookPoint::Connected, Arc::new(ConnectNotice));
        reg.subscribe(WorldEventKind::ObjDestroyed, Arc::new(on_obj_destroyed));
        // M4 web hydration: the "mail" panel feed plus the read-only
        // listing query. Reading stays a Line command — see `query.rs`.
        reg.panel_feed("mail")?;
        reg.query_handler("mail.list", Arc::new(query::ListQuery))?;
        reg.command(
            CommandSpec {
                name: "@mail".into(),
                switches: vec![
                    "send".into(),
                    "quick".into(),
                    "reply".into(),
                    "replyall".into(),
                    "fwd".into(),
                    "file".into(),
                    "folder".into(),
                    "newfolder".into(),
                    "rmfolder".into(),
                    "delete".into(),
                    "purge".into(),
                    "unread".into(),
                    "urgent".into(),
                ],
                shape: ArgShape::EqPair,
                help: "@mail [<n>]  --  List the current folder, or read message <n>. \
                       /send <players>=<subject> composes (\"--\" alone sends, \"~p\" \
                       previews, \"~q\" aborts); /quick <players>=<subject>/<body> is the \
                       one-shot form; add /urgent to flag either. /reply <n> and \
                       /replyall <n> answer a message; /fwd <n>=<players> passes it on. \
                       /folder [<name>] switches or lists folders; /newfolder and \
                       /rmfolder manage up to 16 of your own; /file <n>=<folder> refiles; \
                       /delete <n> soft-deletes to the trash; /purge empties the trash \
                       for good; /unread <n> flips a message back to unread. Recipients' \
                       @lock/mail and mailbox quotas are honored (trash doesn't count \
                       against quota). All mail is IC and queues for transit. Inbox \
                       delivery and returned confirmations take physical time."
                    .into(),
                ..CommandSpec::default()
            },
            Arc::new(cmds::mail_cmd),
        )?;
        Ok(())
    }

    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        let rebuilt = data::rebuild(&data)?;
        let st = ctx
            .state_mut::<MailState>()
            .ok_or_else(|| ModError::msg("mail: state slot missing"))?;
        *st = rebuilt;
        Ok(())
    }
}

/// `ObjDestroyed` observer: a destroyed character's mailbox (entries +
/// user folders) is removed and its persisted documents deleted; message
/// documents survive while any *other* recipient still references them.
/// Mail they SENT stays in the recipients' boxes (history is the
/// recipients', not the sender's).
fn on_obj_destroyed(ctx: &mut Ctx<'_>, ev: &WorldEvent) {
    let WorldEvent::ObjDestroyed { obj } = ev else {
        return;
    };
    let owner = *obj;
    let mut ops = Vec::new();
    {
        let Some(st) = ctx.state_mut::<MailState>() else {
            return;
        };
        let Some(b) = st.remove_box(owner) else {
            return;
        };
        let had_folders = !b.folders.is_empty();
        for &id in b.entries.keys() {
            ops.push(data::del_op(data::box_key(owner, id)));
            if !st.msg_referenced(id) && st.remove_msg(id).is_some() {
                ops.push(data::del_op(data::msg_key(id)));
            }
        }
        if had_folders {
            ops.push(data::del_op(data::folders_key(owner)));
        }
    }
    for op in ops {
        ctx.persist(op);
    }
}

/// The connect-time mailbox summary ("You have N unread messages
/// (M urgent).") — silent when there is nothing unread.
struct ConnectNotice;

impl EventHook for ConnectNotice {
    fn connected(&self, ctx: &mut Ctx<'_>, actor: ObjId) {
        let counts = ctx.state::<MailState>().map(|st| st.unread_counts(actor));
        let Some((unread, urgent)) = counts else {
            return;
        };
        if unread > 0 {
            ctx.out.info(
                actor,
                format!(
                    "MAIL: You have {unread} unread message{} ({urgent} urgent).",
                    if unread == 1 { "" } else { "s" }
                ),
            );
        }
    }
}
