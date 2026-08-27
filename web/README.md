# rustmush web client

Svelte 5 + Vite + TypeScript SPA. Scaffolded properly at milestone M4 (see ../docs/PLAN.md);
until then this directory is a placeholder.

Planned shape:

- `WsClient` module: connect/auth/resume/backoff, envelope dispatch by event type.
- Rune stores per domain: scrollback (virtualized ring), channels (+unread), mail, who, sheet, map, session.
- DOM-rendered rich spans (semantic `tag` → CSS class, `link` → click-to-run) — no xterm.js.
- Module panels lazy-loaded by id (`wod/SheetPanel.svelte`, later `btech/HexMap.svelte` on canvas).
- Types generated from `mush-proto` via ts-rs — never hand-mirrored.
- Built to `web/dist`, embedded into `mush-net-web` via rust-embed; `vite dev` proxies `/api` + `/ws`.
