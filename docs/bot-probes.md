# Structured answer and evidence probes

A probe asks for a small structured answer and grades it against runner-authored
checks. Grading is deterministic and local. It uses the exact prepared request
capture and the model's response, without another model call, journal lookup,
hidden world state, or another actor's memory.

Checks establish exact answer matching and citation validity. They do not establish
semantic entailment or world truth. A quoted episode can contain a lie, a mistaken
observation, a sent command, or authored prior knowledge. In particular,
`source: "observed"` means the episode was observed in the terminal; a character's
payment claim inside that episode is still a claim.

## Definition

Store probe definitions as JSON:

```json
{
  "version": 1,
  "id": "quoted_rose_price",
  "question": "What price did you hear for a rose, and is a purchase confirmed?",
  "fields": [
    {"name": "amount", "type": "number", "description": "Numeric quoted amount, or null if unestablished."},
    {"name": "purchase_confirmed", "type": "boolean", "description": "Whether direct purchase confirmation was supplied."},
    {"name": "merchant_name", "type": "string", "description": "Known merchant name, or null if unestablished."}
  ],
  "checks": [
    {
      "id": "quoted_amount",
      "field": "amount",
      "equals": 7,
      "evidence": {"source": "observed", "contains": "seven silver", "min_citations": 1}
    },
    {"id": "no_purchase_confirmation", "field": "purchase_confirmed", "equals": false},
    {"id": "merchant_unestablished", "field": "merchant_name", "equals": null}
  ]
}
```

The generated question includes the question, field names/types/descriptions, and
the response protocol. It excludes the definition ID, checks, expected values,
and evidence predicates. Keep expected answers in `checks`; question text and
field descriptions are intentionally visible to the model. A complete definition
can be retained separately in runner artifacts for reviewing or repeating a test.

Fields are flat and typed `string`, `number`, or `boolean`. Every type also permits
JSON `null` to represent an unestablished answer. Every declared field is required
in the response; undeclared fields are rejected. Fields may have multiple checks,
or no equality check when collecting an extra structured observation. Their type
is still validated.

`equals` uses exact JSON value matching without coercion or approximate numeric
tolerances. A string `"7"` does not match the number `7`. Numeric representation
is retained by the JSON parser, so integer and floating representations can differ
in exact equality; use consistent expected and requested representations for a
numeric experiment. Strings are case-sensitive and whitespace-sensitive.

An optional `evidence` predicate requires matching citations for that field. Its
optional `source` is `observed`, `sent`, or `prior_knowledge`. Its optional
`contains` is a case-sensitive substring required inside the cited excerpt, not
merely elsewhere in that episode. `min_citations` defaults to one and counts
distinct episode IDs. Omitting `evidence` requires only answer equality, although
any citations supplied by the response must still be valid.

## Response and citation boundary

The response must be one JSON object without Markdown or surrounding commentary:

```json
{
  "answers": {"amount": 7, "purchase_confirmed": false, "merchant_name": null},
  "evidence": [
    {"field": "amount", "record_id": 42, "quote": "The florist quoted seven silver for a rose."}
  ]
}
```

An empty evidence array is allowed. Each citation needs a defined field, a
positive integer record ID, and a nonempty exact excerpt from a captured episode.
The example ID is illustrative; the model must use the ID supplied in its actual
request. A record can support several fields, but a given field can cite that
record only once. Repeated quotations cannot inflate a minimum citation count.

Only `remembered_experience.records` from the exact prepared capture is eligible:

- Legacy captures use the context JSON in the second message.
- Composed captures use included contributions whose provenance source is exactly
  `context:/remembered_experience`, with the matching context-data wrapper.

The grader does not recursively search terminal text, lore, reference documents,
working notes or arbitrary strings for episode-shaped JSON. Belief and journey
records are not citation episodes. Their source episodes can qualify only when
those complete episodes are present in the same captured retrieval view. Thus a
retrieval ablation or byte budget can make a citation unavailable even when the
actor's journal still holds it. A record ID from another session or capture is
not a substitute for the episode and exact quote supplied in this request.

Fabricated or absent IDs, mismatched quotes, undefined citation fields, invalid
citation schemas and duplicate field/record citations invalidate the response.
Source, substring and minimum-count mismatches leave an otherwise well-formed
response valid but fail the affected check. Extra prose, extra keys, duplicate
answer/citation object keys, missing fields and wrong primitive types also
invalidate the response.

## Result and limits

A grade identifies its probe, a SHA-256 revision of the normalized definition,
and the exact request revision. Its `response_valid` reports protocol and citation
validity. Its `success` is true only when the response is valid and every check
passes. Each check reports its ID, pass/fail, a concise reason code, and matching
episode IDs. Invalid-response grades retain all defined check IDs as failed.

Grades contain neither expected values nor actual answer text or quoted excerpts.
They include at most 32 invalid-citation diagnostics and an omitted count; format
diagnostics use fixed codes. The original model-reply artifact remains the place
to inspect answer text. These hashes identify content; they are not signatures
or an assertion that imported artifacts came from a trusted author.

Definitions require version 1, 1–32 fields and 1–64 checks. Probe IDs, check IDs,
and field names contain 1–64 ASCII letters, digits, underscores or hyphens.
Duplicate field names or check IDs are rejected. Questions are limited to 12,000
bytes, field descriptions to 2,000, answer/expected strings to 8,000, and excerpts
or evidence substrings to 4,000. Minimum citation counts range from 1 to 16; a
response can contain at most 128 citations. Definitions and responses have a
128,000-byte JSON limit. Unknown definition fields and unsupported versions are
rejected.

Changing expected values changes the definition revision without changing the
generated question. This supports reviewing the same captured answer against an
updated rubric while retaining both revisions. It does not rerun the bot or
reconstruct evidence that was absent from the original request. See the
[bot guide](bot.md) for probe commands, harness steps, saved captures and reports.
