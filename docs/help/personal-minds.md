# Personal mind licences

Use `+actions` to read the actual available offers, then
`+act <action> with <JSON object>` to select one. Choose the product and step
explicitly; the server resolves the signed offer and rechecks current authority.

| Step | Original v3 | Derived v1 |
| --- | --- | --- |
| Publish terms | `original-list-v3` | `derived-list-v1` |
| Request quote | `original-quote-v3` | `derived-quote-v1` |
| Fund quote | `original-fund-v3` | `derived-fund-v1` |
| Complete purchase | `original-grant-v3` | `derived-grant-v1` |
| Refund purchase | `original-refund-v3` | `derived-refund-v1` |

Use the displayed parameter schema and exact identifiers returned by the service.
Funding holds money in escrow; a grant requires acknowledged completion.

Personal renewals use `mind-renewal-draft`, `mind-renewal-fund`,
`mind-renewal-grant`, `mind-renewal-refund`, and `mind-renewal-close`.
Draft takes a `licence` or a `previous_operation` for a permitted retry.
The other steps take the acknowledged renewal `operation`; Close also takes
`reason` (`declined` or `expired`). A renewal offer requires explicit acceptance.
House renewals have separate `house-renewal-*` actions.

Keep the same request identity while a result is unknown. An absent response
is not a failed action or permission to retry with a new identity. Old quotes,
changed terms or a different character's offer can be refused. The former
ambiguous `market-mind-entitlement` and `market-mind-renewal` names are no longer
selectable actions. Products appear only when their actual services are enabled.
