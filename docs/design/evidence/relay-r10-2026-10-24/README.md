# R10 dev1 acceptance evidence

Source tree: `1b5a6dd1906e2bbb9b3c1f90ad90e00ddfa81c70`. Parent: `a517e74f02ad965cd602305cb00022ceee902a86`.
Production story source: `21a6f9b154c59a88c5c5aa9c4c8ec80d21dc2c47`; `test-fixture-delta.txt` records the test-only changes before the final build.

| Test group | Passed |
|---|---:|
| story-tests | 80 |
| content-baseline-tests | 492 |
| content-custom-tests | 90 |
| content-tests | 176 |
| metric-tests | 2 |
| owner-tests | 34 |
| org-tests | 24 |
| content-frame-tests | 10 |
| fixture-smokes | 7 |
| core-tests | 11 |
| contracts | 23 |
| ci-observer | 93 |
| process-tests | 4 |

Total: 1046 Linux tests; zero failures/skips in the accepted runs.

`metadata.json` summarizes source/binary identity, live counts, bounded-load observations and restoration. `live-split.json`, `live-allinone.json` and `ssh.json` retain actual command/response evidence. `fresh.json` and `process.json` record isolated fresh/restart and process-fault gates. Failure logs are retained separately and are not counted as passing runs.

The live harness uses normal game commands and read-only Redis inspection. No fabricated receipts or clock anchors. The original dev1 world was restored; dev2 was unchanged. No credentials, configuration or database snapshots are included. Normal CI is triggered by the subsequent main push and remains owner-monitored.

`build-runner.sh` records the final Linux test/build commands; `live-runner.sh` and `resume-live-runner.sh` record the scoped dev1 deployment and restore commands. The latter skips the already completed fresh-store and baseline-snapshot gates. Local driving/mining (44 tests) and plumbing (30 tests) results are retained in `local-recovery-tests.log` and `local-plumbing-tests.log`.

The restore reinstates the original lane v12/relay v1 world before startup. The subsequent R10 boot adds six new Atlas geography packs; it leaves the lane/relay upgrade explicit and does not activate Fleet or Terms.

Exported text has ANSI escapes and trailing whitespace removed; command output and assertions are otherwise retained.
