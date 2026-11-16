# Selling and bidding on the exchange

Any player may sell goods they hold, or post a bid for goods they want, on the
market board of the venue they trade in. Nobody needs to be online for a deal:
a listing is your standing agreement to sell on its terms, and a bid is your
standing agreement to buy on its terms, so trades settle while you are away.

The board is local. A listing or a bid stands in one venue (`market=<venue>`,
or the venue of the room you stand in), and goods change hands where they are.

## Listing a lot

You sell one exact lot at a time, from a vault you hold. Split it to the size
you want to sell first:

`@inv/split <lot>=<n>` leaves a lot of n units beside the rest.

`+market/post <lot> price=<n> [to=<player>] [market=<venue>] [days=<n>]`

lists the lot at a price per unit, in minor units of the venue's currency (a
price of 700 is 7.00cr). From that moment the lot is held for the listing, so
you cannot also use it or list it twice. A listing stands for 14 days unless
you name `days=`, and up to 90.

A buyer quotes it with `+market/quote`, and takes it up with `+market/accept`
into a vault of their own. Their payment is held first, the lot itself moves to
them, and you are paid once the deal settles. You do not need to be there.

## Posting a bid

`+market/bid <ruleset>:<code> qty=<n> price=<n> into=<vault> [to=<player>] [market=<venue>] [days=<n>]`

posts a bid for a good (a port good reads `tacsim:<good>`, as `tacsim:metals`).
The whole price, quantity times price, is held from your account at once, so
every seller can see the bid is good. Sellers fill it with `+market/sell
<listing>=<qty>`; each sale draws its share of the hold and the goods land in
the vault you named. You hold one bid for a good at a time.

Bulk goods are poured into a ship's hold only from its port's own counters, so
a bid for bulk goods receives into a vault, not a hold.

## Private offers

Add `to=<player>` to a listing or a bid to make it a private offer. Only that
player can see it, quote it or take it up (or, for a bid, sell into it); to
everyone else it does not exist. A private offer is how two players trade
something between themselves at an agreed price.

## Taking it down

`+market/withdraw <listing>` takes one of your listings or bids down. A listed
lot is yours to use again; a bid gives back whatever it still holds. A bid also
gives its hold back on its own when it lapses.

`+market/mine` lists your listings and bids, and how many more you may post.

## Limits

You may hold at most 32 listings and bids at once, and the whole board holds
at most 1,024. When the board is full, try later.
