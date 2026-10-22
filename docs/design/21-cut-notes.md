# 21 — cut notes

One `## G.n` heading per cut of [21-graphics-engine.md](21-graphics-engine.md), in the order they land: what landed, what it deferred and why, the lead's rulings, how it was verified, and the amendments doc 21 owes. G.0 is the document itself.

## G.0

The document (Draft 0.1, 2026-10-21), the README row and this file. Nothing of the engine moved: the spike's code (`crates/mush-pixel`, the TUI's `^G` view) stands as the proof the document describes, with its measurements in `crates/mush-pixel/NOTES.md`.

Draft 0.2 the same evening: the owner accepted every Q-21 default (now D-21-16..22) and answered Q-21-8 with w11term, their own Windows-native terminal on `wezterm-term` (D-21-23).

Draft 0.3 (2026-10-22): two cuts added from the owner's w11term work — G.10 bells and sounds (names over BEL / OSC 440 / OSC 99, D-21-24) and G.11 assets over the player's own connection (a read-only sftp subsystem on the gateway with a hashed manifest, D-21-25).

Draft 0.4 (2026-10-22): after reviewing w11term 331a550 against its game-client protocol, G.10 and G.11 restated in w11term's wire forms (four-field OSC 440 without loop; SFTP v3, `read_len` >= 16384, the manifest's undocumented rules; the TUI key parser must swallow DCS before the probe sends XTVERSION).
