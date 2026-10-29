# Personal mind licences

## Filing a mind

- `@mind/rate <name>@<rev>` asks the Registry to rate a saved revision. Rating
  takes a while; the Registry tells you when it is finished.
- `@mind/file <name>@<rev>[=<charter name>]` files a rated revision. The first time,
  it reserves the charter; type it again once that is confirmed. Filing costs the
  filing fee and gives you the charter and a free owner's licence. The charter name
  defaults to the mind's name.
- `@mind/update <filed> <name>@<rev>` files a newer rated revision on the line of
  a filed mind you own.
- `@mind/withdraw <filed>` stops new sales of a filed revision. Licences already sold
  keep working.
- `@mind/attest <filed>` renews your owner's licence once its term is due. Add
  `org=<org>` to renew a House's licence for an organisation you act for.
- `@mind/fork <licence>=<name>` makes a working copy of a mind you hold an Open licence
  for. `@mind/localcopy` still copies your own drafts.

## Listing and buying

- `@mind/publish <filed> form=sealed|open|open-resale [royalty=<per mille>] [transfer=no]`
  lists a filed mind you own at the counter where you stand. Sealed buyers never see
  the source; open buyers may fork it; open-resale buyers may also list their forks.
  `royalty` is what forks of it pay you; `transfer=no` makes the licences it sells
  non-transferable. The market sets the price, seats and term.
- `+minds` lists the minds for sale at this counter; `+minds <code>` shows one listing's
  full terms, and `+minds/next <code>` the listings after it.
- `+minds/buy <code>` asks for a quote. `+minds/buy/pay <code>` pays it into escrow, and
  `+minds/buy/complete <code>` takes the licence. If the purchase cannot complete,
  `+minds/buy/refund <code>` returns the money.
- `+minds/holdings` lists your filed minds and licences, their terms and Grace, and any
  purchase waiting to complete.

Acting for an organisation: a character granted `roll.holder.org` by an organisation
(`@org/grant <char>=<org> roll.holder.org`) may add `org=<org>` to `+minds/buy` and its
`/pay`, `/complete` and `/refund` steps, to `+minds/renew` and its steps, and to
`+minds/holdings`. The organisation then holds the licence, its treasury pays, and
`+minds/holdings org=<org>` lists what it holds. A House's curator granted on the House's
scope (`scope=<scope>`) may add `org=<org>` to `@mind/attest` to renew the House's filed
mind. Without the grant every such form answers that you do not act for that organisation.
A purchase between an organisation and a character of the same account as the seller is
refused like any sale between one account's characters.

Purchases, resales and transfers between two characters of one account are refused.
Money held for a purchase stays in escrow until it completes or is refunded; a request
whose result you have not seen is not a refusal, so check `+minds/holdings` before
trying again.

## Renewing and trading

- `+minds/renew <licence>` shows a paid licence's renewal, or asks for a renewal offer once
  it is due. Pay the offer with `+minds/renew/pay <licence>` or turn it down with
  `+minds/renew/decline <licence>`; a paid renewal completes on its own. House Pins renew
  the same way.
- `+minds/resell <licence>=<player> price=<credits>` offers a paid licence to one player.
  They see the offer with `+minds/resell/accept <licence>` and accept it by repeating the
  price: `+minds/resell/accept <licence> price=<credits>`. A licence sold from a
  `transfer=no` listing cannot be resold.
- `+minds/charter <filed>=<player> price=<credits>` offers a filed mind's charter, and with
  it the owner's licence and the proceeds of future sales, to one player. They accept with
  `+minds/charter/accept <filed> price=<credits>`; the transfer settles once no sortie under
  the charter is out.

Registry operators can recall a filed revision or revoke a licence term. A recalled
revision flies no new sortie, and a revoked term ends early; sorties already out finish.

The raw actions stay available: `+actions` lists what you can do, and
`+act <action> with <JSON object>` sends one.

On the named launch, `@mind/launch` prints the player market's current prices, terms and offer
windows; they are the deployment's and may change between worlds. Registry recall and revocation
(`roll-registry-recall`, `roll-registry-revocation`) are offered only to the Registry operators the
deployment names, once staff grant them those capabilities.
