//! Test-only deliberate retention of a real observation across dispatches.
//! Production never stores View. No grant/selector/currentness is manufactured.
use super::*;
use mush_core::{ArgShape, CmdError, CommandSpec};
use std::sync::{Arc, Mutex};

pub(in crate::minds) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    let held: Arc<Mutex<Option<Box<View>>>> = Arc::new(Mutex::new(None));
    reg.command(
        CommandSpec {
            name: "@authored-probe".into(),
            shape: ArgShape::Raw,
            ..CommandSpec::default()
        },
        Arc::new(move |ctx: &mut Ctx<'_>| {
            let mut held = held.lock().map_err(|_| CmdError::msg("probe lock"))?;
            let result = (|| -> Result<(), RollError> {
                match ctx.args.raw().trim() {
                    "history-wrong-digest" | "history-foreign" => {
                        let mind = roll(ctx)?.mind(ctx.actor, "child")?;
                        let mut revision = saved_reference(ctx, mind, &MindRef::parse("child@1")?)?;
                        if ctx.args.raw().trim() == "history-wrong-digest" {
                            revision.digest = mush_core::DefinitionDigest::from_bytes([0xff; 32]);
                        } else {
                            revision.key.character = mush_core::ObjId(0);
                        }
                        current_saved(ctx, revision, Use::Source)?;
                    }
                    "capture-history" => {
                        *held = Some(View::source(ctx, &MindRef::parse("child@1")?, None)?)
                    }
                    "capture-source" => *held = Some(View::current_source(ctx, "child", None)?),
                    "capture-edit" => {
                        *held = Some(View::capture(
                            ctx,
                            &MindRef::parse("child")?,
                            None,
                            Use::Edit,
                        )?)
                    }
                    "check" => held.as_ref().ok_or(RollError::Missing)?.check(ctx)?,
                    "emit" => {
                        let view = held.as_ref().ok_or(RollError::Missing)?;
                        view.emit(
                            ctx,
                            super::super::source_lines(&view.copy().source, 1, 200)?,
                        )?;
                    }
                    "publish" => {
                        let view = held.take().ok_or(RollError::Missing)?;
                        let base = view.mind.base();
                        let draft = view.copy.clone();
                        view.publish(ctx, base, draft)?;
                    }
                    _ => return Err(RollError::Missing),
                }
                Ok(())
            })();
            ctx.out.info(
                ctx.actor,
                if result.is_ok() {
                    "probe accepted"
                } else {
                    "probe refused"
                },
            );
            Ok(())
        }),
    )
}
