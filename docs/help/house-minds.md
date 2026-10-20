# House mind licences

Use `+actions` to read the actions currently available to your character.
Choose the listed one-seat or two-seat product explicitly. A product appears
only when the host has enabled its real services; a command name does not grant
access to an unconfigured product.

Each House purchase has a separate action:

| Step | One-seat action | Two-seat action |
| --- | --- | --- |
| Publish scoped house terms | `house-list-one-seat` | `house-list-two-seat` |
| Request the current quote | `house-quote-one-seat` | `house-quote-two-seat` |
| Fund the exact quote | `house-fund-one-seat` | `house-fund-two-seat` |
| Complete the funded purchase | `house-grant-one-seat` | `house-grant-two-seat` |
| Refund the funded purchase | `house-refund-one-seat` | `house-refund-two-seat` |

Send a displayed action with `+act <action> with <JSON object>`. Listing uses
the owner's `owner_licence`; quote uses the exact `listing` token; funding uses
the exact `quote` token; grant and refund use the original funded `operation`
token. Use the identifiers returned by the real service. Publishing requires
current authorization over the house and its scoped title.

Funding holds money in escrow. The licence is granted only when completion is
acknowledged. Keep the returned request identity while an operation is pending
or its result is unknown; an absent response is not a refund or a failed sale.
Refund does not require the house to regain current selling eligibility.

The server rechecks current authority when resolving and preparing the chosen
offer. Old quotes, a different character's offer or changed terms may be refused.
The old ambiguous `market-mind-house` name is no longer a selectable action.
