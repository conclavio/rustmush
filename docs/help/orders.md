# The sailing plan

A captain cannot stand at the counter for every sale. The **sailing plan** is a
list of orders you file for your Dray; the port runs it under your signature
while you are off watch — selling, buying, bunkering and clearing the ship at
its hour — whether you are at the counter or asleep in a bunk. You file it
aboard the Dray, or in the port where she is docked.

## Reading it

`+ship/orders` shows the plan from anywhere: each order with its state, the next
one due (in UTC, with its IC date and the real time until it), any hold, and the
last lines of its log, each signed with your name.

## Filing and amending

`+ship/order` takes one of:

  sell <good> <qty|all> [min=<cr>] [short=partial|skip|hold]
  buy <good> <qty|fill> [max=<cr>] [short=partial|skip|hold]
  depart <port> [at=now|HH:MM|daily HH:MM|+Nh|+Nm] [until=+Nh|+Nm]
  stop
  del <n> | move <n>=<pos> | clear | pause | resume | skip <n>

A good is named by its id (metals, volatiles, propellant, provisions,
industrial_parts, structural_alloys) or by the start of its name. Each order
gets a number that never changes; the plan's revision goes up with every edit.

  min= / max=     a sale's floor and a buy's ceiling, in credits per unit
  fill            reaction mass or provisions only: bring the tank or the locker
                  up to what the next departure in the plan needs, for the load
                  your orders can make
  short=          what to do when an order can only be part met: partial (the
                  default) takes what it can and moves on, skip passes the order
                  over, hold stops the plan there
  at=             when a departure may leave, in UTC: now, the next HH:MM, every
                  day at daily HH:MM, or +Nh / +Nm from now — at most a week ahead
  until=          how long after its hour a departure may still leave, +Nh or +Nm,
                  at most 24 hours; a daily departure that misses it waits for the
                  next day
  stop            the plan stops there, as filed, until you resume it

A departure plans its passage at its hour, over the load the plan will make, and
burns nothing until it is cleared. It takes the fastest listed route, by way of a
place you name (`via=kirkwood`), at the throttle you set (`throttle=500` is half
thrust: longer and thriftier; 250 to 1000); `+ship/orders` shows both.

Put the fuel and the stores **before** the cargo buys of a leg: the port keeps
back enough of your money for the next departure's reaction mass and stores, and
clamps each cargo buy to what is left, to the stock, and to the room in the hold
by mass and by volume. The log names every bound that bit ("short of 55 t:
funds").

## How the port runs it

One order at a time, one step per tick. A sale or a buy is made at the counter
in your name and checked on the next ticks until the goods have moved; a step is
never made twice. A departure sails only with its pilot aboard: ashore, it waits
and burns nothing. Anyone else aboard when you filed the departure is listed on
it; a listed passenger who is ashore when she sails is left at the port, and you
are both told.

When an order cannot go on, the plan holds. Some holds wait and clear by
themselves when their cause does (a hull not yet docked, a charter not in force);
others stop the plan until you act — fix the cause, then `+ship/order resume`
(or `del` or `skip` the order). `pause` stops new steps until you resume.

## Off watch

What happens while you are away is kept for you. When you next connect you see
"While you were off watch:" and then each line once: the sales and their shares,
the buys and what bound them, the clearances, the boundary passed and the
docking.

## A worked day

Aboard at Ceres, loaded for Stickney, you might file:

  depart stickney at=now
  sell metals all min=250
  sell volatiles all min=150
  buy propellant fill max=60
  buy provisions fill
  buy industrial_parts 49 max=400
  buy structural_alloys 55 max=400
  depart ceres at=+10m

and log off. The ship clears on the next tick and flies; at Stickney the port
sells both goods into the standing bids, tops up the tank and the locker for the
return, buys the fittings and as much alloy as your money and the hold allow,
and clears her for Ceres. With `depart ceres at=daily 19:30` in place of the
last line she would wait for 19:30 UTC instead, and you could log in before
then to change the plan.

See also: `help ships`, `help trade`, `help ports`.
