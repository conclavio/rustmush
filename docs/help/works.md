# Works and yards: +works and +yard

A port's works makes fittings and parts to order, and its yard builds hulls.
You order at the counter, supply the bill of goods, and collect what is made.
Freeport Works' yard builds the light hulls (the Dray, the Dray-P and the four
rigs), and Stickney builds the heavy ones (`help ports`).

## At a works: +works

```text
+works
+works/quote <recipe> [qty]
+works/order <recipe> [qty][=<org>]
+works/deliver <order>=<good> <qty>
+works/buy <order>[=<good>]
+works/collect <order>
+works/cancel <order>
```

`+works` reads the works here: what it makes (each recipe's bill, its output and
its days), how many slots it has free, and your own orders and where they stand.

`+works/quote <recipe> [qty]` says what an order would take: the bill, the IC
days, the fee and the booking fee. Nothing is held. A quantity is one to a
hundred, and one if you name none.

`+works/order <recipe> [qty]` books it. The works opens an escrow for the order,
and holds the booking fee (one slot-day) on your account; a cancel forfeits it.
With `=<org>` you book it for an organization you act for: you pay, and what is
made is the organization's.

`+works/deliver <order>=<good> <qty>` moves goods on the bill from your hold,
docked at this port, into the order's escrow. A part goes in as its lots. Deliver
as many times as the bill takes: the work starts once the bill is covered and a
slot is free, in the order orders were booked.

`+works/buy <order>[=<good>]` buys the bill's next short line, or that good's, at
this port's asks, straight into the escrow from your own account; you need no hold
docked for it. A good the port stocks with no ask is listed to you first, and you
buy again to take it.

`+works/collect <order>` moves the finished output into your docked hold. Collect
within 28 IC days: after that each day of storage costs a slot-day, until your
account runs dry and the works takes the goods into its stores.

`+works/cancel <order>` is yours alone, and only before half the work is done.
The bill comes back, and `+works/collect <order>` moves it into your docked hold;
the booking fee is forfeit, and so is the fee for the work done so far.

## At a yard: +yard

```text
+yard
+yard/quote <recipe>
+yard/order <recipe>[=<org>]
+yard/deliver <order>=<good> <qty>
+yard/buy <order>[=<good>]
+yard/collect <order>
+yard/cancel <order>
+yard/fund <recipe>=<org>
```

`+yard` takes the same steps for a hull. A yard builds one hull an order, so it
takes no quantity. Its bill includes the hull's parts, one a slot, delivered as
lots; a yard part is listed to you first when you buy it, and you buy again to
take it. Asked for a hull it does not build, a yard names the port whose yard
does.

`+yard/collect <order>` is the hull's commission. She is handed over docked here
(a ship) or on the workshop floor (a rig), titled to you, or to the organization
you booked her for, with her magazines empty. A canceled order's bill comes back
into your docked hold instead.

`+yard/fund <recipe>=<org>` is taken only at Stickney's yard. An officer who may
spend the organization's treasury orders a hull for it: the organization's yard
agent is funded with the price, buys the bill here, and she is handed over docked
here, titled to the organization, in the IC days the yard names.

See also: `help ports`, `help ships`, `help exchange`, `help hardware`.
