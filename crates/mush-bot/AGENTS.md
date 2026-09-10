# External bot architecture

The external bot and in-game NPC agents are intentionally independent approaches.
Their overlap is useful for comparing behavior and testing different designs. Do
not consolidate them into a shared agent implementation.

- Keep the bot's model client, provider types, prompts, decision loop, memory,
  knowledge retrieval, scheduling, run limits and scripts owned by this tool.
  Do not implement them using `mush-npc`, `mush-llm`, `mush-worldgen`, or any other
  game/server crate. This applies to normal and build dependencies, including
  indirect dependencies introduced through a shared helper crate.
- Interact with the game exclusively through the player-facing SSH terminal.
  Player and builder permissions come from the logged-in character. `@gen` may
  use the server's LLM service internally, just as it does for a human builder;
  that service must never run the external bot's own decisions.
- Bundled player help, exported setting documents and general-purpose libraries
  (SSH, HTTP, serialization, async runtime) are appropriate shared inputs. Using
  the same underlying model provider is fine; sharing the agent/service
  implementation is not.
- Character sheets and executable command scripts use JSON only. Keep Markdown
  for documentation and reference knowledge; do not maintain parallel executable
  Markdown formats or duplicate source records.
- Game crates are allowed only in development dependencies for server-under-test
  fixtures. The end-to-end tests launch the actual bot executable and drive the
  public SSH interface. `mush-llm` mocks belong to the fixture's server-side
  generation, while the bot uses a separate HTTP model stub and its own client.
- Compare the two approaches through common world fixtures, player-visible
  outcomes and evaluation criteria. Do not give the bot hidden engine state,
  NPC memory, internal action APIs or shared planning code to make them agree.

The shared Cargo workspace is an organizational convenience. The bot remains a
standalone executable that can run against a separately deployed game server.
When changing dependencies, verify the runtime/build graph with:

```text
cargo tree -p mush-bot --edges normal,build --target all
```

`mush-bot` must be the only RustMUSH package in that graph. See
[the bot guide](../../docs/bot.md) for user-facing behavior and the JSON script format.
