//! Invoked only after the parent's genuine Registry certificate and real filing.
//! Each branch executes actual v2 listing/quote/funding/Grant through the same
//! registered participants and cold boot audits. No synthetic grant or source
//! observation is used. This does not claim server writer/Unknown evidence.
use super::*;
use mush_core::DefinitionDigest;
use mush_roll::commerce::paid::PaidForm;

pub(super) fn exercise(base: &PurchaseFixture, filed: &Durable) -> TestResult {
    for (index, name, form) in [
        (0_u128, "sealed", PaidForm::Sealed {}),
        (1, "open", PaidForm::Open {}),
        (2, "open_resale", PaidForm::OpenResale {}),
    ] {
        let mut f = PurchaseFixture::boot_protocol(
            base.resources.clone(),
            base.accounts.clone(),
            filed,
            true,
        )?;
        let start = 2_000_000 + index * 100;
        let list = f.config.quotes.listing_action()?;
        let quote = f.config.quotes.quote_action()?;
        let [fund, grant, refund] = f.config.purchases.actions()?;
        assert_eq!(list.version(), 2);
        assert!(
            f.config
                .purchases
                .actions()?
                .iter()
                .all(|a| a.version() == 2)
        );
        let before = f.snapshot();
        for request in [
            json!({"owner_licence":"1"}),
            json!({"owner_licence":"1","form":"Open"}),
            json!({"owner_licence":"1","form":"open","holder":BUYER}),
        ] {
            assert!(f.prepare(&list, GOD, start, request).is_err());
            assert_eq!(
                f.disk, before.disk,
                "missing/foreign consent cannot publish"
            );
        }
        f.perform(&list, GOD, start, json!({"owner_licence":"1","form":name}))?;
        let listing = one(f.rows("market", "market_mind_original_listing"))?;
        assert_eq!(listing.pointer("/product/form"), Some(&json!(form)));
        assert_eq!(listing.get("format"), Some(&json!(2)));
        f.perform(&quote, BUYER, start + 1, json!({"listing":token(start)}))?;
        let actual_quote = one(f.rows("market", "market_mind_original_quote"))?;
        assert_eq!(actual_quote.pointer("/product/form"), Some(&json!(form)));
        let quoted = f.snapshot();
        eprintln!("M7_STORY phase=form_fund event=begin form={index}");
        let prepared = f.prepare(&fund, BUYER, start + 2, json!({"quote":token(start + 1)}))?;
        assert_eq!(f.disk, quoted.disk);
        assert!(f.rows("roll", "roll_mind_personal_paid_licence").is_empty());
        f.install(prepared)?;
        let operation = token(start + 2);
        let funded = f.snapshot();
        let allocation = one(f.rows("roll", "roll_mind_paid_purchase"))?;
        assert_eq!(allocation.get("format"), Some(&json!(2)));
        assert_eq!(allocation.pointer("/terms/form"), Some(&json!(form)));
        eprintln!("M7_STORY phase=form_fund event=complete form={index}");
        let mut refunded = f.reboot(&funded)?;
        refunded.perform(&refund, GOD, start + 4, json!({"operation":operation}))?;
        assert!(refunded.rows("roll", "roll_mind_paid_term").is_empty());
        refunded.reboot(&refunded.snapshot())?;
        f.engine.world.set_now(1017);
        eprintln!("M7_STORY phase=form_grant event=begin form={index}");
        let prepared = f.prepare(&grant, BUYER, start + 3, json!({"operation":operation}))?;
        assert!(
            f.rows("roll", "roll_mind_paid_term").is_empty(),
            "no pre-ACK grant"
        );
        f.install(prepared)?;
        let term: PaidTermRecord =
            serde_json::from_value(one(f.rows("roll", "roll_mind_paid_term"))?)?;
        assert_eq!(term.terms().form(), form);
        assert_eq!(term.window().issued_at().get(), 1017);
        let history = f.economy()?.purchase_receipt_audit().get(operation)?;
        assert_eq!(history.effect(), &fund);
        assert!(history.terminal().ok_or("actual terminal")?.granted());
        let granted = f.snapshot();
        let restarted = f.reboot(&granted)?;
        assert_eq!(
            restarted.rows("roll", "roll_mind_paid_term"),
            f.rows("roll", "roll_mind_paid_term")
        );
        assert_eq!(restarted.balances()?, f.balances()?);
        eprintln!("M7_STORY phase=form_grant event=complete form={index}");
        // The actual configured player module consumes the owner service. An
        // Open flag in a fixture is never itself a source authorization.
        eprintln!("M7_STORY phase=form_source event=begin form={index}");
        paid_checkout::exercise_source(&f, &granted, name != "sealed")?;
        eprintln!("M7_STORY phase=form_source event=complete form={index}");
        eprintln!("M7_STORY phase=form_forks event=begin form={index}");
        paid_checkout::exercise_forks(&f, &granted, name != "sealed")?;
        eprintln!("M7_STORY phase=form_forks event=complete form={index}");
        coherent_form_tampering(
            &f,
            &granted,
            if name == "open_resale" {
                "open"
            } else {
                "open_resale"
            },
        )?;
    }
    Ok(())
}

pub(super) fn historical_v1(
    base: &PurchaseFixture,
    funded: &Durable,
    granted: &Durable,
) -> TestResult {
    // Upgrade composition only: permanent historical bytes and E4 proof stay
    // unchanged, including the old Sealed form and exact old funding effect.
    let migrated = PurchaseFixture::boot_protocol(
        base.resources.clone(),
        base.accounts.clone(),
        granted,
        true,
    )?;
    for (owner, family) in [
        ("roll", "roll_mind_paid_purchase"),
        ("roll", "roll_mind_paid_term"),
        ("market", "market_mind_purchase"),
        ("market", "market_mind_purchase_receipt"),
    ] {
        assert_eq!(migrated.rows(owner, family), base.rows(owner, family));
    }
    paid_checkout::exercise_source(&migrated, &migrated.snapshot(), false)?;
    let mut pending = PurchaseFixture::boot_protocol(
        base.resources.clone(),
        base.accounts.clone(),
        funded,
        true,
    )?;
    let old = one(pending.rows("market", "market_mind_purchase"))?;
    let operation: CommitToken =
        serde_json::from_value(old.get("operation").ok_or("operation")?.clone())?;
    let [_, grant, refund] = pending.config.purchases.actions()?;
    assert!(
        pending
            .prepare(&grant, BUYER, 3_000_000, json!({"operation":operation}))
            .is_err(),
        "v2 cannot grant changed consent over v1 funding"
    );
    pending.perform(&refund, GOD, 3_000_001, json!({"operation":operation}))?;
    assert!(pending.rows("roll", "roll_mind_paid_term").is_empty());
    pending.reboot(&pending.snapshot())?;
    Ok(())
}

fn coherent_form_tampering(
    f: &PurchaseFixture,
    granted: &Durable,
    replacement: &str,
) -> TestResult {
    // Repair all typed pair/product/quote joins, leaving the actual acknowledged
    // E4 funding history untouched. Typed metadata cannot promote paid consent.
    let mut corrupt = granted.clone();
    let form = json!({"kind":replacement});
    let mut listing = one(f.rows("market", "market_mind_original_listing"))?;
    *listing.pointer_mut("/product/form").ok_or("listing form")? = form.clone();
    let listing_digest =
        DefinitionDigest::for_canonical_value("market.original_listing.v2", &listing)?;
    let purchase = one(f.rows("market", "market_mind_purchase"))?;
    let mut quote = purchase.get("quote").ok_or("quote")?.clone();
    *quote.pointer_mut("/product/form").ok_or("product form")? = form.clone();
    let product_digest = DefinitionDigest::for_canonical_value(
        "market.original_product.v2",
        quote.get("product").ok_or("product")?,
    )?;
    *quote.get_mut("product_digest").ok_or("product digest")? = json!(product_digest);
    *quote.get_mut("listing_digest").ok_or("listing digest")? = json!(listing_digest);
    let decoded =
        mush_market::mind_trade::records::MindQuote::decode(&serde_json::to_vec(&quote)?)?;
    let quote_digest = decoded.digest()?;
    change_one_family(
        &mut corrupt,
        "market",
        "market_mind_original_listing",
        |row| {
            *row = listing;
            Ok(())
        },
    )?;
    change_one_family(&mut corrupt, "market", "market_mind_purchase", |row| {
        *row.get_mut("quote").ok_or("quote")? = quote;
        *row.get_mut("quote_digest").ok_or("quote digest")? = json!(quote_digest);
        Ok(())
    })?;
    for (owner, family) in [
        ("roll", "roll_mind_paid_purchase"),
        ("market", "market_mind_purchase_receipt"),
    ] {
        change_one_family(&mut corrupt, owner, family, |row| {
            *row.get_mut("quote_digest").ok_or("quote digest")? = json!(quote_digest);
            *row.get_mut("product_digest").ok_or("product digest")? = json!(product_digest);
            if owner == "roll" {
                *row.pointer_mut("/terms/form").ok_or("paid form")? = form.clone();
            }
            Ok(())
        })?;
    }
    change_one_family(&mut corrupt, "roll", "roll_mind_paid_term", |row| {
        *row.pointer_mut("/terms/form").ok_or("term form")? = form;
        Ok(())
    })?;
    assert!(
        f.reboot(&corrupt).is_err(),
        "coherent typed form rewrite cannot replace actual E4 consent"
    );
    Ok(())
}
