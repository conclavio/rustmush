# Minds facts in contracts

Use an exact reference with the ordinary contract condition command:

```text
@contract/condition <contract>=fact minds filed filed/<16-digit-id> revision=1
@contract/condition <contract>=fact minds licence.lapsed licence/<16-digit-id> revision=<term-generation>
@contract/condition <contract>=fact minds licence.transferred <acceptance-commit> revision=<new-holder-generation>
```

A transferred reference uses the actual resale acceptance operation from the
acknowledged action result, including a funded acceptance waiting for settlement.
The new holder generation is the captured seller generation plus one. A pending
or refunded resale never satisfies it. A later transfer does not erase the event.

Only that transfer's actual seller or buyer can authorize its reference in a
contract where they are a party. The other explicitly authorized contract
participants can then observe the result. Owning the licence later does not give
access to somebody else's earlier transaction. Filed references expose only the
public filing; lapse references require current holder/title authority.

A fact condition does not transfer a licence, extend its term, release an old
claim or grant source access. Missing or unavailable evidence cannot settle a
contract. Use the normal contract offer/accept flow after choosing terms.
