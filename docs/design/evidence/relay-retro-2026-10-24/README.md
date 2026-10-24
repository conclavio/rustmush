# R1–R10 retrospective evidence

Review of `57c5747dc02370e6854188e009dda61b9108df94` on 2026-10-24.

`review-probes.rs` is an additive test fragment. To reproduce, copy that commit's
`crates/mush-server/tests/relay_registry.rs` to a temporary sibling target named
`relay_retro_review.rs`, append the fragment, and run the command in `metadata.json`.
The original Rig and production implementation are unchanged. The first probe uses
an explicitly constructed persisted independent-pair fixture, following the existing
R4 fixture; it does not claim a new manufacturing or live private-link acceptance run.
The other probes drive normal commands through the existing production test Rig.
All three tests assert the observed behavior, so passing means the review finding
was reproduced. They are not regression tests asserting corrected behavior.

The six source hashes were checked against the VPS build cache and match the local
R10 tree. The test target used in-memory worlds; the live dev1 world was not changed.
The temporary remote target was removed after the run. No deployed binary was changed.

`r10-ci-failure.txt` is the relevant excerpt from the completed `test (tacsim-lib)`
job. Other jobs were still running when that log was collected. CI results do not
establish whether a failure is limited to a fixture; the boot dependency needs review.

The rejected recipient-transfer command and zero forwarding count are in the
already committed R9b transcript, at the references in `metadata.json`.
