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

A house's own licence to a mind it filed runs 365 game days, then 3 days of
Grace; its catalogue sells through both and stops when the Grace ends. A paid
House licence runs 90 game days and renews for another 90.

Your House Pins appear in `+minds/holdings`. Renew one with `+minds/renew <licence>` and
its `/pay` and `/decline` steps, as for a personal licence, and resell one with
`+minds/resell <licence>=<player> price=<credits>`. A buyer of a resold House Pin needs the
same standing as a buyer of a new one. Buying a House Pin still uses the actions above.

Where `+minds/renew` is not open, including any launch without the player market, a House
Pin renews through the actions `house-renewal-draft`, `house-renewal-fund`,
`house-renewal-grant`, `house-renewal-refund` and `house-renewal-close`, sent with `+act`.
Draft takes the `licence`, or a `previous_operation` for a permitted retry; the other steps
take the acknowledged renewal `operation`, and Close also takes `reason` (`declined` or
`expired`). Requesting an offer does not accept it.
