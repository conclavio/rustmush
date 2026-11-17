# TacSim 3D Client — Unity `6000.6.7f1` URP

Authoritative pure-online Unity 6 Universal Render Pipeline (URP) 3D client for `rustmush` / `tacsim`. Connects directly to `mushd` / `mush-gateway` over HTTP (`/api/auth/*`, `/api/content/*`) and WebSocket (`/ws?char=<key>&resume=<last_seq>` with `Sec-WebSocket-Protocol: rustmush.v1.<token>`).

## Capabilities Across Slices 1–3 & OOC Simulator Arena

- **Dynamic SHA-256 Content Pack Sync:** Fetches and caches authored JSON packs (`neighborhood.*`, `units/*`, `goods`, `arenas/*`, `atlas/*`) from `mushd` (`/api/content/manifest` or WS `tacsim.content` feed) under `Application.persistentDataPath/TacSimContentCache/<host>`, with `IPreprocessBuildWithReport` fallback packs for older servers.
- **Resilient WebSocket Transport:** Sends `ClientCommand::Caps`, tracks `seq` with exponential-backoff auto-reconnect (`?resume=<last_seq>`), handles `4401`/`4429`/`1012` close codes, validates `ViewEnvelope` cursors, renders `RichText` spans/colors/`CommandLink`s, and synchronizes live `Composing` typing bubbles.
- **Multi-Port 3D Stations & Ship Interiors:** Walkable 3-deck Ceres Freeport, Stickney Yards, Aitken Highport, and OOC Simulator Centre (`+ooc` -> `simulator`), plus 6/7-compartment `Dray`, `Dray-P Prospector`, and `Corvette` interiors with continuous doorway hysteresis and shortest-path BFS exit dispatch.
- **Diegetic Desks, Port Exchange & Sailing Plans:** Contextual `[E]` NPC conversations, Co-op chartering, Prospector refits, 1-click Commodity Exchange (`port.trade` feed + 4 stowage capacity gauges), and Corridor Sailing Plan builder (`+ship/plan`, `+ship/order`).
- **Orbital Flight, Zero-G Mining, 60 Hz 6-DOF Docking & Surface Rovers:** Bridge Holo-Tank (`space.holotank`), orbital flight/mining/fire-control (`space.cockpit`), client-side 60 Hz 6-DOF Bow Collar alignment with RLE verification tape (`space.docking`), and 3D planetary quarry rovers (`site.map`, `tv.status`).
- **Live 6v6 Red vs Blue OOC Simulator Arena:** Diegetic Simulator Selector Console GUI in `room.simulator_lobby` (`sim team`, `sim unit`, `sim ai`, `sim mode`, `sim start`) + pod boarding (`board`) into live `arena.cockpit` and `arena.map` (`zigzag-leb128-delta` elevation heightfields).

## Quick Start in Unity `6000.6.7f1`

1. Open the `unity/` folder in **Unity Hub** (`6000.6.7f1`).
2. *(Optional — Local Commercial 3D Packs)* Click **`TacSim -> 1. Import Vertical Slice Assets (Ida Faber + Station + Ships)`** from the top menu bar:
   - Imports curated `.unitypackage`s from `C:\Users\faeni\3d_assets` (override via `TACSIM_3D_ASSETS`), upgrades materials to `Universal Render Pipeline/Lit`, builds `TacSimHumanoid.controller`, and populates `Resources/TacSimAssetCatalog.asset`.
   - **Git Hygiene:** `.gitignore` excludes `unity/Assets/*` outside `unity/Assets/_TacSim/` as well as locally generated `Resources/` and `Scenes/` bindings so commercial `.fbx` / textures never enter git. Without commercial packs installed, all stations, ships, vehicles, and avatars automatically use built-in procedural URP meshes.
3. Click **`TacSim -> 2. Build Vertical Slice Scene`** to generate and open `Assets/_TacSim/Scenes/VerticalSlice1.unity`.
4. Press **Play**, enter your `mushd` server URL (e.g. `https://dev1.conclav.io` or `http://127.0.0.1:8080`), choose an avatar rig, and click **Login to mushd** or **Register Account**.

## Controls

| Context | Input | Action |
|---|---|---|
| **3rd-Person Station / Ship** | `WASD` + `Shift` | Camera-relative walk / sprint across doorways & stairs |
| **Camera Orbit** | `Right Mouse Drag` / `Scroll` | Orbit camera / adjust zoom |
| **Diegetic Interact** | `E` or `Left Click` | Talk to NPCs, use Co-op / Exchange / Simulator Console desks, cycle Airlock, board Sim Pod, or take Bridge Helm |
| **60 Hz 6-DOF Bow Docking** | `WASD`, `R/F`, `Arrows`, `Q/E` | Lateral RCS, Closure pulse, Pitch/Yaw/Roll trim (`X` Dampeners, `G` Guidance Assist, `Backspace` Abort) |
| **Surface Rover & Sim Arena** | `WASD`, `LMB`, `Space`, `RMB` | Sticky cruise throttle & hull steering, click-to-lock/drive, fire weapons, turret/cupola camera |
