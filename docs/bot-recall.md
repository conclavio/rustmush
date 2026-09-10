# Local recall experiments

The bot can use different local policies to select records from its own evidence
journal. This changes which existing experiences enter a model request; it does
not generate a summary, change beliefs, make a provider call, or read another
actor's journal. The same setting applies to automatic context retrieval and
explicit `/recall` searches.

Use a JSON context profile with `--context`, or load one during a session:

```text
/context load bots/context/recency-recall.json
/recall How much did I pay for the rose?
/inspect preview How much did I pay for the rose?
```

The checked-in [recency profile](../bots/context/recency-recall.json) is an
experiment in favoring recent experience. A context profile can select a combined
policy instead:

```json
{
  "version": 1,
  "recall": {
    "version": 1,
    "policy": "relevance_recency",
    "max_records": 96,
    "recent_episodes": 4,
    "recency_window": 128,
    "recency_weight": 200
  }
}
```

Context profiles that omit `recall` retain the default lexical policy. A supplied
`recall` object fills omitted fields from defaults. Unknown fields and policy
names are rejected. The defaults are version 1, lexical policy, a limit of 512
records, four recent episodes, a recency window of 128 journal records, and a
recency weight of 200.

| Policy | Ranked candidates |
| --- | --- |
| `lexical` | Distinct query terms found in each serialized evidence event; more matches first, then newer journal ID. This retains the previous default selection order. |
| `recency` | Newest journal ID first. Query relevance does not influence ranking. |
| `relevance_recency` | Query-matching records ranked by `100 × matched_terms + recency_bonus`. The bonus decreases linearly from `recency_weight` to zero across `recency_window` journal records. |

Lexical matching is case-insensitive substring matching after splitting the query
into distinct alphanumeric terms and excluding short/common terms. It is not
semantic search. The recency window uses journal sequence distance, including
intervening journal metadata and working-note records, so repeating a search
against unchanged history gives the same result even when wall-clock time changes.
Recency parameters only affect the combined policy. Pure recency can return
unrelated experiences; the returned query-match count makes this visible.

Every policy first considers an explicit `#id` lookup, then up to 32 ranked
candidates, then `recent_episodes` recent episode records. An exact lookup only
retrieves evidence episodes, beliefs, or journeys. Journal headers, session
markers and working notes are outside this search. Working notes remain a
separate context field and are identified as model-authored notes.

Candidate expansion retains a belief's quoted source episodes, a journey's source
episodes, and an episode's associated command attempt and adjacent episode frames
from the same session. Expansion is one level deep. Results are presented in
ascending journal ID, retaining their original text, timestamps, session, source,
place, evidence links and belief confidence. A speaker label still does not prove
the character knows a name; a payment claim or sent command does not become a
confirmed transaction. Route lookup continues to use recorded directed journeys.

`max_records` counts every returned record, including expanded evidence. It can
be 0 through 1024; zero returns no records. `recent_episodes` can be 0 through 64.
The recency window is 1 through 10000 and weight is 0 through 1000. At the default
settings, the record cap is above the previous selector's maximum expansion.

The effective byte budget is the smaller of context `recall_bytes` and configured
memory `recall_bytes`. As before, it counts the sum of the serialized record
bytes. It excludes JSON array punctuation, query, belief status and selection
diagnostics. Whole records that exceed the remaining budget are skipped; their
text is never truncated or rewritten. The prompt composer applies its own
component and total budgets to the complete resulting recall view.

Each view contains `selection`, which explains the experiment:

- Effective settings, candidate and query-match counts, and the record byte budget.
- Up to 32 ranked candidate IDs with lexical hit counts and rank scores.
- Up to 32 selected record IDs with the reason and originating candidate ID.
- Counts excluded by the byte or record limit and counts of omitted diagnostic rows.
- Explicit lookup status and up to 32 rows identifying missing source episodes
  for returned beliefs or journeys.

Scores rank relevance to a query; they are not confidence in a claim. The
`evidence_gaps` rows make budget-limited evidence visible. A selected belief may
retain a quoted reference while its full source episode is outside the budget;
an exact follow-up lookup can retrieve that episode. Missing records or query
matches establish a recall gap, not that an event never happened.

Use harness matrix `contexts` to compare these profiles while holding other
settings fixed. Replay uses the records already captured in a model request;
changing a local recall profile cannot retroactively alter that capture. See the
[bot guide](bot.md) for context controls, captures and harness matrices.
