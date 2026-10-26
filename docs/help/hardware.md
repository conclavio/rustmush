# Governed cores and fitting

A configured hardware works sells certified cores and fits them to the governed
line-tank family. Ask for the site's supplier and fitting shop. These offers appear
only at an explicitly enabled works with real stock, accounts and permissions.
Use `+actions` to see the offers available to your character.

| Core | Finished price | Supplier die | Integration | Integration time | Fitting time |
| --- | ---: | ---: | ---: | ---: | ---: |
| Class I | 3,000 cr | 2,700 cr | 300 cr | 24 IC hours | 1 IC hour |
| Class II | 8,000 cr | 7,200 cr | 800 cr | 48 IC hours | 2 IC hours |
| Class III | 25,000 cr | 22,500 cr | 2,500 cr | 96 IC hours | 4 IC hours |

Fitting costs another 100 cr for every class. Class III also requires the buyer's
actual receiving entitlement; having the money or being staff does not supply it.
The named importer may issue at most 64 Class I, 16 Class II and 4 Class III dies
per IC civil month, shared by its two configured stock agents.

## Buying a core

The supplier selects a real certified die and names its buyer. A supplier offer
uses this form; replace the example buyer and lot with the actual choices:

```text
+act offer-core-work with {"requester":42,"class":"i","supplier_lot":"lot#0000000000000001"}
```

The named buyer selects that offer and their own account:

```text
+act accept-core-work with {"offer":"00000000000000000000000000000001","payer":"12"}
```

Use the actual offer identifier and account from the current choices. Acceptance
holds the price in your account. Work advances on the world's durable hourly
schedule. Reserve the output, then complete the order when its work is ready:

```text
+act reserve-core-output with {"order":"wo#0000000000000001"}
+act complete-core-work with {"order":"wo#0000000000000001"}
+act deliver-core-output with {"order":"wo#0000000000000001"}
```

The order identifier is the one returned for your accepted work. Completion
captures the hold and pays the published split. Collection moves the completed,
personally titled core into your custody. Before completion, the offered
cancel-core-work action releases the price hold and the reserved input. The work
reservation lasts seven IC days; missed deadlines require cleanup, not backdated
production. Restarting the server does not erase accepted work or earn extra hours.

A supplier can instead offer integration of a buyer-supplied certified die by
omitting supplier_lot. The buyer then supplies input_lot at acceptance. That route
charges only the integration price and still requires the actual Origin history.
An ordinary inventory item with the same grade is not a certified die.

## Fitting

Bring the personally owned, delivered core and the supported hull to the actual
fitting shop. The fitter names the hull, incoming core, buyer and original
manufacturing acceptance:

```text
+act offer-core-refit with {"requester":42,"hull":43,"incoming":44,"manufacture":"00000000000000000000000000000002","class":"i"}
+act accept-core-refit with {"offer":"00000000000000000000000000000003","payer":"12"}
```

Acceptance holds the fitting fee and reserves the real hull and incoming core.
Once the scheduled work is ready, the buyer uses install-core-component with the
actual order selector. The installation, title updates and fee payment commit
together. Existing non-core hull state, including damage and heat, is retained.
The offered cancel-core-refit action releases unfinished work and its fee hold.

Wait for acknowledged results before treating work, payment or delivery as done.
If a result is uncertain, check the same action's status and current offers before
starting another purchase. No response is not proof that an action failed.
