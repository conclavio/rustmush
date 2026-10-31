# 29b. The sshdoorway reviews, applied to the instance's public port

**2026-10-30.** This is §9 of [doc 29a](29a-sshdoorway-implementation-plan.md), "Applied
to the instance's public port", kept as its own file because 29a lives on PR #193's branch
(`claude/sshdoorway-impl`) and this work lands on `main` first. When PR #193 merges, this
file's body is appended to 29a as §9 and the file is removed.

## 9. Applied to the instance's public port

The two adversarial reviews of the doorway (29a §7, §8) found a class of problems that the
instance's own public SSH port (`mush-gateway`, crate `mush-net-ssh`, russh 0.63.1) still
had, because it predates them. What was wrong, in severity order:

1. **Nothing bounded a connection before it opened a channel.** The only clock was
   `PREAUTH_TIMEOUT` (120 s from channel open). A client stalled in the key exchange, or one
   that completed it and never authenticated, or one that ignored the server's
   `SSH_MSG_DISCONNECT` (russh half-closes and then reads until the client's EOF, and
   `inactivity_timeout` is `None` by design) held its task and descriptor for ever. Over-cap
   connections were not even counted, so their number was unbounded.
2. **The accept loop died on its first error.** russh's `run_on_socket` returns `Err` on
   the first `accept` failure (`EMFILE`); the gateway logged "SSH accept loop failed" and the
   port was gone until a restart while the web side kept serving.
3. **The guard had only a per-address concurrency cap and the password tempban**: no
   new-connections-per-minute rate, no global cap, no IPv6 /48 aggregate, and every
   connection it did refuse had already cost a key exchange.
4. **Nothing in `IpGuardConfig` was configurable**: the gateway constructed `IpGuard::new()`.

What was built, in `mush-net-ssh`, `mush-config` and `mush-gateway` only, each item with its
test:

| # | Item | Where | Test |
|---|---|---|---|
| 1 | **The kill switch.** `mush-sshdoorway`'s `kill.rs` copied as `mush_net_ssh::kill` (`KillSwitch`, `Killable`, plus `arm(delay)`); every accepted stream is wrapped in one | `crates/mush-net-ssh/src/kill.rs` | `a_killed_stream_fails_a_blocked_write`, `a_killed_stream_fails_a_blocked_read`, `an_armed_switch_kills_after_its_delay` |
| 2 | **The accept-time deadlines.** `SshConfig::auth_deadline` (default 30 s, `[ssh].auth_deadline_s`): authenticate within it of accept, open a channel within another, else a polite disconnect and, `DISCONNECT_GRACE` (5 s) later, the cut — unconditional, because russh takes no message while a key exchange is active, and a half-open client never sends EOF. `run_stream` itself runs under the deadline, so a client that never sends its version line is dropped too. In-game connections are untouched: no inactivity timeout, as before | `Ingress::handle`, `ConnHandler::auth_succeeded`, `channel_open_session` | `a_connection_that_never_authenticates_is_cut_at_the_deadline`, `a_client_silent_before_its_version_line_is_cut`, `a_client_stalled_in_the_key_exchange_is_released_by_the_cut` (the half-closed-after-disconnect release: the disconnect alone does not free it, the cut does), `an_authenticated_connection_that_opens_no_channel_is_cut`, `an_in_game_connection_outlives_every_deadline` |
| 3 | **Every server-initiated disconnect arms the cut**: the welcome screen's farewell (`close_polite`: QUIT, the in-band lockout), the 120 s pre-auth watchdog, and the pumps' close after the game ended the session (`Cut::after_close`: the client gets `auth_deadline` to answer the channel close with its own, then a disconnect, then the cut — §8 A-3) | `close_polite`, the pre-auth watchdog, `Cut` | covered by the existing QUIT and lockout flows plus the deadline tests above; the channel-close grace, as in the doorway, has no client-side driver (russh's client always answers a close) |
| 4 | **Over-cap connections are told and cut within 5 s** (§7 "Splice and DoS" 3): the auth banner says "Too many connections from your address", every auth method is refused, and one that never tries a method is disconnected at 3 s and cut at 5 s. At most `MAX_UNCOUNTED_PER_KEY` (2) such connections per network hold global capacity at once; further ones are dropped at accept (§8 F-1) | `OVER_CAP_DEADLINE`, `OVER_CAP_GRACE`, `OVER_CAP_NOTICE`; `IpGuard::admit_at` | `an_over_cap_connection_is_told_refused_and_cut_within_five_seconds`, `over_cap_banner_slots_are_bounded_per_network` |
| 5 | **An accept loop that survives errors.** `BoundSshServer::run` is its own loop over `TcpListener::accept` + `russh::server::run_stream` (as the doorway's `Doorway::serve`); an accept error is logged and retried after 100 ms (§8 B-2). russh's `Server` trait is no longer implemented: the only way in is through the gate | `serve_accepts`, the `Acceptor` seam | `the_accept_loop_survives_accept_errors` (a scripted acceptor: three `EMFILE`s, then real streams) |
| 6 | **The gates, in the guard**: `per_ip_new_per_minute` (20), `global_connections` (2048), `per_v6_48_connections` (64), answered at accept with no handshake; the rate table and the failure/connection map bounded at 65,536 networks, swept at most once a second, and **evicting rather than refusing** when still full (§8 F-5: one /48 is exactly 65,536 /64s); the global and /48 caps checked before a new network is inserted so a flood that would be dropped anyway cannot fill the table; refusal counters taken once a minute into one log line | `IpGuard::admit_at` / `release` / `take_counters`; `accept_refusal_summary_loop` | `the_rate_window_drops_the_twenty_first_and_resets`, `the_global_cap_counts_admitted_and_over_cap_alike_and_releases`, `a_v6_48_is_capped_across_its_64s_and_released`, `the_rate_table_is_bounded_and_evicts_rather_than_refuse`, `one_v6_48_cannot_fill_the_rate_table_against_everyone_else`, `global_and_prefix48_drops_do_not_pollute_the_rate_table`, `the_failure_map_is_bounded_and_evicts_idle_entries_only`, `a_dropped_connection_gets_no_handshake` |
| 7 | **The allow-list** `exempt_nets` (default `127.0.0.0/8`, `::1/128`; `IpNet`, the doorway's `Net` shape: CIDR or one address, IPv4-mapped addresses folded): exempt from the per-network caps (concurrency, rate, /48), **not** from the global cap, which bounds the process rather than the player — the doorway's semantics, so the two halves match — and never from the tempban or the sitebans. Exempt admissions are counted (`exempt`, in the per-minute line), not logged one by one. The owner's bot fleet from one home address is the case | `IpNet`, `IpGuard::is_exempt` | `ipnet_parses_cidr_and_bare_addresses_and_contains_mapped_v4`, `exempt_networks_skip_the_per_network_caps_but_never_the_global_cap_or_the_bans` |
| 8 | **The concurrency-only pair is untouched.** `conn_opened`/`conn_closed` (the web ingress, and PR #193's trusted ingress) still count against `max_conns` alone: a browser tab's reconnect storm must never meet the SSH rate | `IpGuard::conn_opened` | `the_concurrency_only_pair_is_untouched_by_the_rate_and_global_caps` |
| 9 | **Configuration.** `[ipguard]` (`IpGuardCfg`, `deny_unknown_fields`, every key defaulting to the guard's own) and `[ssh].auth_deadline_s`, validated at the `--check-config` preflight and at boot (a cap of zero, a prefix past its family or an unparsable exempt network refuses, §8 C-13); the gateway builds the guard from it. Declared in `deploy/instance/mushd.toml.tmpl` and `mushd.toml.example`; documented in `docs/ops.md` | `IpGuardCfg::guard_config`, `Config::ssh_auth_deadline`, `preflight::check` | `ipguard_section_defaults_to_the_guard_and_validates_its_gates`, the shipped-config census |

**The hooks seam.** `SshHooks` gained `admit_connection(addr) -> Admission` (`Admit`,
`OverCap`, `Drop`) and `release_connection(addr, admission)`, both defaulting onto the
existing `connection_opened`/`connection_closed` pair so every other embedder keeps exactly
what it had; `ConnHandler` remembers which pair admitted it (`Slot`) and releases through
the same one. `Ingress::handler_for(addr, counted)` keeps PR #193's signature for the
trusted ingress, which gates at its preamble for the address the doorway names.

**Not covered by a test**, as in §8: the channel-close grace of item 3 needs a client that
ignores a channel close, which russh's client does not offer; the `EMFILE` itself is
scripted rather than provoked.

**Left open.** `mushd` (the all-in-one, local only) still constructs `IpGuard::new()` and
takes the default deadline through `start_listener`; it reads the same `Config`, so wiring
it is one line each when wanted. A `/metrics` counter for the refusals (beside
`mush_connections_opened_total`) would complement the per-minute log line. §8 B-4 (a
per-account connection cap) and the §7 F5 note (tempban keyed per /64 at the instance —
already the case here through `v6_prefix`) are unchanged.
