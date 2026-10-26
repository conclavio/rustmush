# Minds fact inventory

The optional actual `MindCommerce::with_minds_facts` composition registers one
Economy provider namespace, `minds`, and the exact Roll owner reader
`minds_contract_facts@2`. Default composition remains absent. All reference IDs
and revision numbers are canonical and bounded. Unknown, malformed, missing or
unavailable references uniformly have no usable fact.

| Kind | Reference id | Revision | Scope and truth |
| --- | --- | --- | --- |
| `filed` | `filed/` plus the 16-digit lower-case filed ID | 1 | Existing public source-free immutable filing root; true after commit, unchanged by withdrawal/recall. |
| `licence.lapsed` | `licence/` plus the 16-digit lower-case licence ID | Accepted term generation | Actual committed lapse occurrence; current personal holder/title authority and immutable contract consent. Later holder proof uses the actual resale chain; historical acceptance attribution stays unchanged. |
| `licence.transferred` | Actual resale acceptance commit, 32 lower-case hex characters | Resulting holder generation, 2..65537 | Exact native monetary history, typed resale/holder receipt and source-retention roots. Funded/refunded is not completion. Only that receipt's actual seller or buyer can authorize attachment for an immutable contract in which they are a party. Later unrelated ownership gives no access. |

Restricted licence facts require the real authenticated Economy draft/offer,
contract read or settlement dispatch. Economy retains exact attachment consent;
changed parties, copied/missing markers, unrelated staff and unscoped world-only
queries confer no authority. Actual contract participants can read a properly
authorized reference. This adds no source output, licence eligibility or
settlement callback. House-born paid licences have personal buyer/seller scope;
Org-front delegation remains unavailable until its real scoped path exists.

Event time is actual retained RealUTC: an immediate transfer's accepted UTC or a
deferred transfer's terminal posted UTC. Filing has no retained UTC and returns
0. Lapse uses its committed occurrence UTC. IC dates stay in the referenced
records. Event actor is audit attribution only. Existing open IC claims and
warrants remain independent of these facts and later transfers.

The M8 kinds `seized`, `unwarranted` and `claimed` remain unsupported.
