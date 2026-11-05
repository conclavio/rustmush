# Your first run on dev1: mining and trading

A step-by-step guide for someone who has never played. You will connect, sign on with the Miners'
Co-op, take a ship down to an asteroid to mine ore, sell it, and then start a trade run between
ports. Type exactly what is shown; the lines you should see back follow each command.

| | |
|---|---|
| Connect and sign on | about 10 minutes |
| A mining run, start to sale | about 45 minutes |
| A trade voyage | about one real day (you can log off) |
| The game clock | 24× real time: 1 real hour is 1 game day |

In the command blocks, lines starting `>` are what you type; the others are what the game answers,
abbreviated with `…`. Ship and rig names will differ for you.

## Before you start

- **An SSH client.** Any terminal works (Windows Terminal, macOS Terminal, PuTTY). A window at
  least 120 columns wide makes the maps readable.
- **A fresh account of your own.** Each account can join the Co-op only once, ever.
- **Every action previews first.** Commands for rigs, licences and leases show what they would do
  and end with "… `go` starts it." Repeat the command with `go` on the end to do it.
- **Lost?** `help`, `help mining`, `help ports` and `help orders` explain everything here in more
  depth. `look` redraws where you are.

## Part 1: Connect and sign on

1. Connect to dev1 and create your account. Use letters, digits, `_` or `-` in the name.

   ```
   $ ssh dev1@conclav.io
   (welcome screen)
   > create YourName YourPassword
   ```

   If your SSH client warns that the host key has changed, run `ssh-keygen -R conclav.io` and try
   again. Next time you can go straight in with `ssh dev1+YourName@conclav.io`.

2. You start in Limbo. Step into the world:

   ```
   > c
   Port Watch Arrivals …   (Ceres Freeport)
   ```

3. Walk to the Co-op desk and read its terms. You have no money yet; the Co-op stakes you.

   ```
   > registry
   Registry & Co-op Desk …
   > +coop
   … stake 25,000cr … the Co-op's share is 25% of your margin … N Drays free
   ```

4. Join the Co-op and name your ship. This charters you a Dray, a small freighter.

   ```
   > +coop/enrol Salt Moth
   The Co-op charters you …, laid up as the …, now the Salt Moth, cradled at the …
   > +ship
   > +credit
   ```

   If it refuses: "not enrolling yet" means the world is still starting up (wait a minute and
   retry). "No Dray free to charter" means all eight ships are taken; tell the organiser.

## Part 2: A mining run

You will convert your ship to carry a mining rig, buy a prospecting licence, lease a rig, fly down
to an asteroid site, find a good pocket of ore, drill it, and bring the ore home to sell. Do these
steps in this order: the conversion needs an empty hold.

### Kit up at Ceres

1. Convert your Dray to a Dray-P, which has a bay for two rigs.

   ```
   > +coop/refit prospector
   … as a Dray-P: a two-rig vehicle bay aft of the hold, which now takes 86 t …
   ```

2. Buy a prospecting licence (2,500cr for 168 game days).

   ```
   > +licence/buy
   … +licence/buy go buys it.
   > +licence/buy go
   > +licence
   … in force until …
   ```

   If `+licence` still says "You hold no prospecting licence", wait five seconds and check again.

3. Lease a mining rig at the Exchange Hall (1,400cr plus a 5,000cr deposit you get back).

   ```
   > out
   > concourse
   > exchange
   > +tv/lease
   … +tv/lease go signs it.
   > +tv/lease go
   Occator Water & Propellant leases you OWP-TV-… into the …'s bay as rig 1 …
   > +tv
   Rig 1: …
   ```

4. Plan the trip to the Ahuna Flank mining site. The plan tells you how much reaction mass (fuel)
   and how many crew-days of provisions you need, out and home.

   ```
   > +chart
   > +ship/plan ahuna
   Plan for … burn … t … crew-days … (and the leg home)
   ```

5. Buy fuel and provisions at the chandlery: a little more than the plan says, for both legs. Each
   purchase is a quote then an accept.

   ```
   > chandlery
   > +market/list
   (numbered list of what is for sale)
   > +market/quote 3=12
   Quote #41 …
   > +market/accept 41
   Agreed. …
   ```

   The numbers are examples: use the list number for reaction mass and the tonnes your plan needs,
   then the quote number the game gives you. Repeat for provisions. "That quote no longer stands"
   means quote again.

### Fly down to the site

1. Walk to your ship and board it.

   ```
   > exchange
   > concourse
   > arrivals
   > lobby
   > gallery
   > +ship/board
   You cycle the lock into the …
   ```

2. Lift for Ahuna Flank. The hop takes about 4–5 real minutes.

   ```
   > +ship/lift ahuna
   Clearance filed for …
   … Salt Moth settles onto the pad at Ahuna Flank.
   ```

   While you fly, `+travel/status` shows progress, and `help mining` is a good read.

### Find ore

1. Look at the site map. `@` is you, `H` your hull.

   ```
   > +tv/map site
   ```

   "Is still being mapped" just after a restart: wait ten seconds and try again.

2. Send the rig down the ramp (about 12 seconds).

   ```
   > +tv/1 launch go
   Rig 1 is down the … on Ahuna Flank.
   ```

3. Scan the ground (about 25 seconds), then take a sounding (about 50 seconds). The sounding lists
   pockets of ore with their quality.

   ```
   > +tv/1 scan go
   Rig 1 scanned 9 fields from …
   > +tv/1 sound go
   Rig 1 sounded N pockets around …; every pocket not named reads barren:
     C117 fair metals
     …
   ```

4. Drive to the best pocket. An address is a block letter and a number, like `C117`. Pick one
   marked *good* or *fair* for metals.

   ```
   > +tv/1 drive C117 go
   Rig 1 arrives at C117.
   ```

### Mine and come home

1. Drill. The rig works in cycles of about 75 seconds (twice that in rock), 1–3 tonnes a cycle,
   until its 12-tonne hopper is full or 8 cycles pass.

   ```
   > +tv/1 mine metals go
   Rig 1 starts drilling C117: the first cycle …
   Rig 1, cycle 2 of 4: 2 t metal ore; hopper 4 of 12 t.
   ```

   A thin pocket? Sound again (`+tv/1 sound go`) and drive to the next one. Staking a claim
   (`+claim/stake`) is optional: open ground can be mined without one.

2. Bring the rig back aboard. It unloads its hopper into your hold.

   ```
   > +tv/1 recover go
   Rig 1 is back in the …'s bay; it unloads its hopper into the hold next.
   Rig 1 unloads 12 t of metal ore … into the …'s hold
   ```

3. Fly home to Ceres. The ship won't lift until every rig is aboard with an empty hopper.

   ```
   > +ship/plan ceres
   > +ship/lift ceres
   … docks at Ceres Freeport.
   > +ship/leave
   ```

4. Sell your ore at the Exchange.

   ```
   > lobby
   > arrivals
   > concourse
   > exchange
   > +market/list
   > +market/sell 2=all
   Sold 12 t of metal ore … you receive … when the goods are in.
   ```

   Metal ore fetches about 70cr a tonne: a full hopper grosses about 840cr, and you keep about
   735cr after the Co-op's share. "You have an order still being handled" means wait a few seconds
   and retry.

## Part 3: A trade run

Trading is buying goods where they are cheap and selling them where they are dear. Voyages are
long: the game clock runs at one game day per real hour, and a leg between ports takes many game
days. **Start the voyage, then log off**; the ship flies itself and you come back to sell.

> **Avoid Kirkwood.** Ceres's direct lane to Stickney passes Kirkwood Depot, where customs may stop
> your ship and demand manoeuvres a new player has not learned. Sail to Luna instead, as below.

| Good | Buy at Ceres | Sells at |
|---|---|---|
| Metal concentrate | 200cr/t | Luna (Cabeus Works); Stickney (Kasei, 280cr/t) |
| Water ice | 100cr/t | Luna (Lunar Habitat Board); Stickney (Kasei, 200cr/t) |
| Reaction mass (fuel) | 40cr/t | buy for your own burn |
| Provisions | 10cr | buy for your own crew-days |

1. In the Exchange, see what is for sale and check what your hold can carry on the trip to Luna
   (the port is called `luna`). A plain Dray carries 100 t; a Dray-P, 86 t.

   ```
   > +market/list
   > +ship/plan luna
   Plan for … room … burn … crew-days … passage … IC days ≈ … real
   ```

2. Buy cargo: concentrate and ice, as much as the hold and your money allow. Leave money for fuel.

   ```
   > +market/quote 1=40
   Quote #52 …
   > +market/accept 52
   Agreed. …
   ```

3. Buy fuel and provisions at the chandlery: re-run `+ship/plan luna` with the cargo aboard, then
   buy a margin over its burn and crew-days, as in Part 2.

4. Board and undock for Luna. If the game says goods "have not come aboard yet", wait 30 seconds
   and retry.

   ```
   > +ship/board
   > +ship/undock luna
   Clearance filed for luna: the plan burns N t of reaction mass and D crew-days;
   arrival in X IC days, about HH:MM UTC …
   ```

   Note the arrival time: that is real time. Log off with `QUIT`. When you reconnect, "While you
   were off watch:" tells you what happened.

5. When the ship has docked at Luna, go ashore and sell.

   ```
   … docks at Aitken …
   > +ship/leave
   > +market/list
   > +market/sell 4=all
   Sold … t of … at …/t: …; Co-op share …; you receive … when the goods are in.
   ```

6. Check your earnings. Then plan the next leg: home to Ceres, or onward to Deimos (`deimos`) and
   Stickney (`stickney`), where Kasei buys concentrate and sells alloy.

   ```
   > +credit
   > +coop/statement
   ```

## If something goes wrong

| You see | What to do |
|---|---|
| "The Co-op is not enrolling yet" | The world is still starting. Wait a minute and retry. |
| "No Dray free to charter" | All eight ships are out. Tell the organiser. |
| A site "is closed: its pad is not ready" | Normal for about 8 minutes after a restart. Wait and retry. |
| The refit is refused | Your hold isn't empty. Refit before buying anything. |
| The lift is refused | Not enough fuel for out and back, or a rig is still out or its hopper not empty. |
| "That quote no longer stands" | Prices moved. Quote again. |
| A rig is stuck and won't come back | Note the rig and site, and ask staff; only staff can recover it. |
| Customs hails you near Kirkwood | Note what it says and ask staff. Then use the Luna route. |

## Please tell us

- Where did you get stuck, and for how long? Quote the line the game showed.
- Which answers were confusing or didn't say what to do next?
- Was any wait too long or too short to be interesting?
- Did the money feel right: could you afford what you needed, and did the sale feel worth it?
- What did you want to do that the game didn't let you?

## For the organiser

- Run this against a freshly deployed dev1. Allow 10 minutes after boot before testers lift for a
  site.
- The Co-op has eight Drays. More than eight testers on one world will be turned away at enrolment.
- Give each tester their own new account: enrolment and the licence are one per account.
- The refining and works chain (Kasei, `+works`) is not in this script: it needs days of voyages and
  more money than a new player has.
- Kirkwood customs is not covered by player help yet, which is why the trade route goes by Luna.
- Market list numbers and quote numbers in the examples are illustrative; they vary by world.
