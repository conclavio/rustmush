# Trading

The trade run in one paragraph: at Ceres Freeport you buy what Stickney wants
(metal concentrate and water ice), bunker reaction mass and provisions for the
passage, and fly to Stickney Yards; there you sell into Kasei's standing bids,
buy what Ceres wants (the Yard's crated fittings and alloy stock), bunker for
the return, and fly home to sell again. Each sale pays you the price less the
Co-op's share of the margin (`help coop`); everything else you spend is yours.

## Why you carry both

The cargo hold has two limits, mass and packed volume (`help ships`). A dense
good — concentrate, alloy — weighs the hold out long before it fills it; a bulky
good — ice, crated fittings — fills it long before it weighs it out. A hold of
either alone wastes the other limit. On these routes the bulky good tends to pay
more margin per tonne and the dense good more per cubic metre, so neither alone
is best: the best full load mixes the two so that both limits are met at once.
How much of each depends on the prices of the day, so it moves when they do.

You do not have to work it out by hand: `+ship/plan` shows the room left in the
hold by mass and by volume, good by good. Buy, read the plan, and adjust: the
hold is at its best when the plan reads it full by mass and by volume together.

**When cash is short, buy the bulky good first**, up to the volume the tonnage
you can afford leaves room for, then the dense good with what is left. Money is
then a third limit, and per credit spent the bulky good usually earns more
margin; `+ship/plan`'s room by volume tells you where to stop.

## Fuel and stores

Every tonne aboard costs burn, so buy reaction mass and provisions **after** the
cargo, to the figures `+ship/plan` gives for the load you actually carry — and
in a sailing plan, file the fuel and stores **before** the cargo buys, so the
port keeps back the money the next departure needs (`help orders`).

## Prices

Prices are the boards' (`help ports`): they are posted by each house, they move
when the houses move them, and stock and intake are finite. Read them where you
stand with `+market/list`; `+market/depth <catalogue>:<code>` gives the public
picture of one good.

## A worked two legs

Every figure below is one you read off the game, not one to remember.

  At Ceres, at the Exchange: `+market/list`. Note the concentrate's and the ice's
  numbers and prices. `+ship/plan stickney` shows the room by mass and by volume.
  `+market/quote <number>=<tonnes>` and `+market/accept <quote>` for each good,
  until the plan reads the hold full by mass and by volume.

  At the chandlery: `+ship/plan stickney` again. Buy reaction mass to its burn
  and provisions to its crew-days; plan once more, since what you bunkered weighs
  too, and top up if the figures moved.

  Board in the berth gallery and `+ship/undock stickney`, or file the day as a
  sailing plan and log off.

  At Stickney, at Kasei's Receiving Bay: `+market/list` shows Kasei's bids. Sell
  each good with `+market/sell <number>=<tonnes>`. The sale line names the price,
  the gross, the Co-op's share and what you receive "when the goods are in"; the
  money lands once the tonnes have moved into Kasei's stores.

  A port's bids follow its season. When the sky has carried a port far from the
  ports that supply a good — the passage is long against the port's usual — its
  standing bid for that good is posted above its list price, up to half again;
  when the passage is short, the bid is the list price. The price is set when the
  bid is posted and holds until it lapses and is posted anew, so `+market/list`
  is the word; `+chart <port>` shows how the passage runs over the coming year.

  On the materials exchange: buy the fittings and the alloy for Ceres the same
  way — bulky first if money is tight — then bunker for the return and fly home.
  At Ceres, the Co-op's chit and Freeport Works' alloy bid buy them.

`+credit` shows your money at every step, and `+coop/statement` your sales, the
Co-op's share of them, and your cash against the buyout.

See also: `help orders`, `help ships`, `help ports`, `help coop`.
