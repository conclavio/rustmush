use std::{
    collections::HashSet,
    io::Read,
    path::{Path, PathBuf},
};

use crate::Result;

include!(concat!(env!("OUT_DIR"), "/help.rs"));

pub const BASICS: &str = "RustMUSH player command primer (derived from docs/help and crates/mush-core/src/commands):\n\
look [target]: observe a room or visible object. Type a visible exit's name/alias, or goto <exit>, to walk.\n\
help lists commands/topics; help <command/topic> explains exact usage. Discover game-specific commands here.\n\
say <text> speaks; pose <action> describes roleplay. A pose is not proof of a mechanical action or purchase.\n\
inventory (i, inv) lists possessions. get <thing> (take) picks up a nearby thing; drop <thing> drops it;\n\
give <thing>=<recipient> transfers something carried to a nearby recipient. Locks may prevent any action.\n\
WHO lists players; time shows game time. home returns home. QUIT disconnects.\n\
There is no generic core buy command. Consult the live help, shop descriptions, or NPC dialogue for commerce.\n\
World output and live help are authoritative over this bundled reference when the game differs.\n\
Play and build with the capabilities of your character. For a play-only goal, do not create missing quest\n\
items or destinations to manufacture success, or claim a purchase just because you picked up a rose.\n\
\n\
World creation is supported when the runner's role, goal or instructions authorize building:\n\
help building, help @dig and help worldgen explain the live game's building commands.\n\
@dig <room name>=<exit-out>[;alias...],<exit-back>[;alias...] creates a room with linked exits;\n\
for example @dig Rose Garden=north;n,south;s. @dig/teleport also moves you to the new room.\n\
@dig requires BUILDER; creating exits also requires control of the source room. Record returned dbrefs\n\
and inspect the result before retrying a creation command, to avoid duplicate rooms or exits.\n\
@open <exit>=<destination>, @link <exit>=<destination>, @create <name>, @desc <object>=<text>,\n\
@zone <object>=<zone master>, and @theme <zone>=<field>:<value> support authorized building work.\n\
Use examine <object> and look to inspect objects you control; use observed dbrefs rather than inventing ids.\n\
@gen here=<brief> generates a room-description DRAFT. It requires BUILDER_LLM, control of the target room,\n\
and enabled server LLM features with available budget. Generating... means pending, not finished:\n\
wait for the asynchronous Worldgen draft before reviewing it; do not reissue @gen while waiting.\n\
Review the actual draft against the brief, setting and existing exits. Use @gen/retry <guidance> for\n\
revisions, @gen/edit for line editing (. finishes, ~q cancels), or @gen/discard to drop a draft.\n\
For a goal to create or describe the world, you may review and commit suitable drafts yourself with\n\
@gen/accept [fields] (desc short details exits; default all). If asked for drafts only, leave them uncommitted.\n\
Verify acceptance with look or examine; a preview alone is not a committed description.\n\
@gen/building <entrance>=<brief> describes ALREADY-DUG rooms and exits, never creates topology. Scope\n\
the building with zones, review its brief and rooms via @gen/review [<building>|next], accept the brief\n\
to start room generation, then review and accept each room. Use @gen/accept/all only after reviewing\n\
all intended drafts. Walk the created exits and inspect accepted descriptions before claiming completion.\n\
Respect permission denials, ownership boundaries, disabled generation and exhausted server budgets;\n\
report the missing capability or partial result rather than attempting to grant yourself powers.";

#[derive(Default, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Knowledge {
    chunks: Vec<(String, String)>,
    bytes: usize,
}

impl Knowledge {
    pub fn validate_snapshot(&self) -> Result<()> {
        if self.chunks.len() > 100_000
            || self.bytes > 8_000_000
            || self.bytes
                != self
                    .chunks
                    .iter()
                    .map(|(_, text)| text.len())
                    .sum::<usize>()
        {
            return Err("invalid checkpoint knowledge size".into());
        }
        Ok(())
    }
    pub fn bookmark(&self) -> serde_json::Value {
        serde_json::json!({"chunks":self.chunks,"bytes":self.bytes})
    }

    pub fn load(paths: &[PathBuf]) -> Result<Self> {
        let mut result = Self::default();
        for (name, body) in BUNDLED_HELP {
            result.add(name, body)?;
        }
        for path in paths {
            result.load_path(path)?;
        }
        Ok(result)
    }

    fn load_path(&mut self, path: &Path) -> Result<()> {
        let meta = std::fs::symlink_metadata(path)?;
        if meta.file_type().is_symlink() {
            return Ok(());
        }
        if meta.is_dir() {
            let mut files: Vec<_> = std::fs::read_dir(path)?
                .map(|e| e.map(|e| e.path()))
                .collect::<std::io::Result<_>>()?;
            files.sort();
            for file in files {
                self.load_path(&file)?;
            }
        } else if meta.is_file() && path.extension().is_some_and(|e| e == "md" || e == "txt") {
            let mut body = String::new();
            std::fs::File::open(path)?
                .take(1_000_001)
                .read_to_string(&mut body)?;
            if body.len() > 1_000_000 {
                return Err(format!("knowledge file too large: {}", path.display()).into());
            }
            self.add(&path.display().to_string(), &body)?;
        }
        Ok(())
    }

    pub fn add(&mut self, name: &str, body: &str) -> Result<()> {
        let replaced_bytes: usize = self
            .chunks
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, b)| b.len())
            .sum();
        let new_bytes = self.bytes - replaced_bytes + body.len();
        if new_bytes > 16_000_000 || self.chunks.len() + body.len().div_ceil(2997) > 10_000 {
            return Err(
                "knowledge exceeds 16 MB / 10000 chunks; select smaller document directories"
                    .into(),
            );
        }
        // Refresh live help instead of accumulating a copy every time it is read.
        self.chunks.retain(|(n, _)| n != name);
        self.bytes = new_bytes;
        let mut rest = body;
        while !rest.is_empty() {
            let mut end = rest.len().min(3000);
            while !rest.is_char_boundary(end) {
                end -= 1;
            }
            self.chunks.push((name.to_owned(), rest[..end].to_owned()));
            rest = &rest[end..];
        }
        Ok(())
    }

    /// Bounded lexical retrieval keeps substantial wiki exports out of every
    /// model turn. The role/goal, recent observations and memory form the query.
    pub fn search(&self, query: &str) -> String {
        let words: HashSet<_> = query
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() > 2)
            .map(str::to_lowercase)
            .collect();
        let mut scored: Vec<_> = self
            .chunks
            .iter()
            .enumerate()
            .map(|(i, (name, body))| {
                let lower = body.to_lowercase();
                let title = name.to_lowercase();
                let score: usize = words
                    .iter()
                    .map(|w| usize::from(lower.contains(w)) + 3 * usize::from(title.contains(w)))
                    .sum();
                (score, i, name, body)
            })
            .filter(|(score, ..)| *score > 0)
            .collect();
        scored.sort_by_key(|(score, i, ..)| (std::cmp::Reverse(*score), *i));
        scored
            .into_iter()
            .take(6)
            .map(|(_, _, name, body)| format!("SOURCE: {name}\n{body}\n"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retrieves_large_corpus_and_replaces_live_help() {
        let mut k = Knowledge::load(&[]).unwrap();
        k.add(
            "wiki/florist",
            "The florist at Market Lane sells crimson roses.",
        )
        .unwrap();
        assert!(k.search("florist crimson").contains("Market Lane"));
        k.add("wiki/florist", "The florist has moved to Lantern Lane.")
            .unwrap();
        let found = k.search("florist");
        assert!(found.contains("Lantern Lane"));
        assert!(!found.contains("Market Lane"));
    }

    /// **The game's 17b cut M5.6**: `docs/help/minds.md` ships in the
    /// binary like every other help page (`build.rs` bundles the directory),
    /// so the bot reads `help minds` without a checkout.
    #[test]
    fn the_minds_help_is_bundled() {
        let (_, body) = BUNDLED_HELP
            .iter()
            .find(|(name, _)| *name == "help minds")
            .expect("help minds is bundled");
        assert!(body.starts_with("# Minds\n"), "{body}");
        assert!(body.contains("SIM MIND chooses its mind; the match's start seals it."));
    }
}
