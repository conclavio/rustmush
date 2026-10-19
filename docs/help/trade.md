# Trading

The trade run in one paragraph: at Ceres Freeport you buy what Stickney wants
(metal concentrate and water ice), bunker reaction mass and provisions for the
passage, and fly to Stickney Yards; there you sell into Kasei's standing bids,
buy what Ceres wants (the Yard's crated fittings and alloy stock), bunker for
the return, and fly home to sell again. Each sale pays you the price less the
Co-op's share of the margin (`help coop`); everything else you spend is yours.

That is the shuttle, the programme's first run. Past it the lane runs to three
ports on the moons — Aitken Highport over Luna, Voltaire Anchorage on Deimos and
Ligeia Terminal over Titan (`help ports`) — and two longer runs, the ring and
the Titan expedition, below.

## Why you carry both

The cargo hold has two limits, mass and packed volume (`help ships`). A dense
good — concentrate, alloy, carbon billet — weighs the hold out long before it
fills it; a bulky good — ice, crated fittings, crated instrument boards — fills
it long before it weighs it out. A hold of
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
  is the word; `+chart <port>` shows how the passage runs over the coming year,
  and what each of the port's bids posts at today.

  When a port's season runs long enough — the passage half again its usual — the
  port's house posts emergency bids for provisions, volatiles and propellant (at
  Ligeia, provisions only) at half again their worth, a hundred units each, and
  withdraws them when the season turns. `+chart <port>` says whether they are up.

  On the materials exchange: buy the fittings and the alloy for Ceres the same
  way — bulky first if money is tight — then bunker for the return and fly home.
  At Ceres, the Co-op's chit and Freeport Works' alloy bid buy them.

## The moons

No lane runs from a moon to Ceres (`help ports`): a run comes home on the
shuttle's return, from Stickney with the Yard's fittings and Kasei's alloy. At
each port the question is the one you answered at Ceres — which two goods, and
the best load of them.

  Ceres to Stickney or Aitken     concentrate (dense) and water ice (bulky)
  Stickney to Ceres               alloy stock (dense) and crated fittings
                                  (bulky)
  Aitken to Voltaire or Ligeia    lunar alloy stock (dense) and instrument
                                  boards (bulky)
  Ligeia to Aitken or Voltaire    carbon billet (dense) and water ice (bulky)
  Voltaire to Stickney            nothing: the hop

On every listed leg but the hop the best full load mixes its two goods by the
rule above, and `+ship/plan` reads it; the mix moves with the prices, so no
table gives it.

**Instrument boards, crated** (instrument_boards) are the chartered Earth-basin
fabs' boards, forwarded through Luna in shock crates: bulky and dear. Crisium
Instruments sells them at Aitken; Syrtis Bonded Imports buys them at Voltaire
and the Selk Townships Board at Ligeia. Nothing at Ceres or Stickney buys them.

**Carbon billet, pressed** (carbon_feedstock) is Titan's organics pyrolysed and
pressed to graphite: dense. The Titan Concession Company sells it at Ligeia; the
Lunar Habitat Board buys it at Aitken, whose closed loops lose carbon every
year, and the Strategic Reserve buys it at Voltaire.

### The ring

  Ceres to Aitken       concentrate and ice, into Cabeus Works' and the Lunar
                        Habitat Board's bids
  Aitken to Voltaire    lunar alloy and instrument boards, into Syrtis's bids
  Voltaire to Stickney  the hop: three to four and a half IC hours and a tonne
                        of reaction mass
  Stickney to Ceres     the shuttle's return

At Ceres the fork is Stickney or Aitken. Aitken is nearer in most seasons and
its bids sit a shade under Kasei's: the leg to Luna earns a little less a tonne
and often more an IC day. The season between Earth and Ceres decides, and
`+chart <port>` for each shows it.

The hop carries nothing by design. Voltaire sells only reaction mass and
provisions, and Stickney's boards never bid for either: with the Roads a few
hours away, Stickney's emergency Wanted for provisions and reaction mass
cannot post. So nothing you buy at Voltaire is worth carrying to Stickney:
sell the whole Luna load at Voltaire and come down empty to load the return.
No lane is posted the other way (`help ports`). Keeping Luna's alloy for
Freeport Works at Ceres only takes the room Kasei's alloy would fill at the
same margin. Reaction mass at Voltaire is priced as at Kasei.

A full load at Aitken costs about 60000.00cr, far more than the Co-op's stake:
the ring is a run for a captain who has made money on the shuttle. Over a Saturn
year it nets about 1.2 to 1.3 times the shuttle per IC day — a median, not every
season. Its wanted lines are finite like every board's, and Ceres's ice and
concentrate now serve Aitken as well as Stickney, so in a busy season they sell
out sooner.

### Titan

  Aitken to Ligeia      lunar alloy and instrument boards, into the Selk
                        Townships Board's bids at the frontier's price
  Ligeia to Aitken      carbon billet and ice, into the Habitat Board's bids
  Ligeia to Voltaire    carbon billet into the Reserve's bid, and the ice on
                        down the hop to Kasei

**Fly it at `throttle=450`, one soul, on every listed Titan leg.** At full
thrust a Titan leg burns more reaction mass than a Dray's tank holds. At that
throttle it fits the tank laden in every season of a Saturn year, and a leg
runs about 52 to 65 IC days — the sky decides, so plan it. One soul is the
rule: two need twice the crew-days, and in a slow season more than the locker
holds, so take a second only when `+ship/plan` reads the stores inside the
locker. Pass the throttle to `+ship/plan`, `+ship/undock` and the sailing
plan's departure (`help orders`); when the throttle you ask for will not fit
even on full tanks, `+ship/plan` names the highest that does, or says none
does (`help ships`).

Ligeia sells the cheapest reaction mass in the system and the dearest
provisions, both at the Company's price. The Townships buy little: about a
dozen full loads of alloy and boards between all comers, and then their lines
read not buying. To Ligeia the alloy earns more per credit than the boards, so
when money is short buy it first.

Over a Saturn year Titan nets about 1.5 times the shuttle per IC day, as a
median, and not in every season: at the campaign's opening in 2351 the shuttle
is in a good season and earns more per IC day than Titan does. It needs the
same full load at Aitken as the ring, and a round trip ties the hull up for
twice a leg.

At Ligeia the fork is Aitken, to run again, or Voltaire, to go home by the hop
and the shuttle's return.

**Titan's ice is worth more at Kasei than at the Reserve.** The Reserve's bid
for ice stands under Kasei's in every season, on purpose: the Reserve buys what
the Yard does not want. Bound home by Voltaire, sell the billet to the Reserve,
carry the ice down the hop, and sell it into Kasei's bid at the Receiving Bay
before you load the return.

**The Co-op's share on Titan's ice.** The share is taken on the margin over the
Co-op's reference price (`help coop`), and the reference for water ice is
Ceres's ask for it as it stands, wherever you bought yours. The Company asks far
less for its ice than Ceres does, so the share falls only on what you get over
Ceres's price, and the difference between the Company's price and Ceres's is
yours. The reference for instrument boards is Crisium's ask at Aitken, for
carbon billet the Company's at Ligeia, and for alloy Kasei's, which Cabeus
matches.

On any run, `+credit` shows your money at every step, and `+coop/statement` your sales, the
Co-op's share of them, and your cash against the buyout.

See also: `help orders`, `help ships`, `help ports`, `help coop`.
