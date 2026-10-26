//! An additional real marked Registry run, using the acknowledged owner adapters.
use super::super::super::{
    ack, admit, artifacts, input, prepare_admit, prepare_transition, run, typed,
};
use super::*;
pub(super) async fn certify(
    f: &PurchaseFixture,
    saved: &Durable,
) -> TestResult<(Durable, RatingRef)> {
    let mut world = PlayerWorld::boot_derived_updates(f, saved)?;
    let tasks = world.tasks.clone();
    let result: TestResult<(Durable, RatingRef)> = async {
        let (binding, mut ready) = admit(&mut world, 81_100_000)?;
        let subject: RatingJob = typed(&world, "rating_jobs", binding.job)?;
        // Keep this one full genuine certificate eligible for the dependent
        // filing child. Lapse settlement uses a separate actual paid quantum.
        assert!(world.engine.world.now() < super::parent_end(&world)?);
        let executor = Executor::with_wake(&BulkConfig::default(), None)?;
        let deadline = Instant::now() + Duration::from_secs(7_200);
        let mut number = 81_100_001_u128;
        let mut measured = 0_u64;
        let mut quanta = 0_u64;
        loop {
            if Instant::now() >= deadline {
                return Err("actual marked Registry liveness guard".into());
            }
            let ticket = artifacts(&mut world)?
                .registry
                .grant(binding, ready.next_grant_ms())?;
            let prepared =
                prepare_transition(&mut world, binding, ticket, number, Transition::Grant)?;
            assert!(run(&world, binding)?.flight.is_none());
            ack(
                &mut world,
                prepared,
                ticket,
                binding.job,
                number,
                Transition::Grant,
            )?;
            let lease = world
                .engine
                .world
                .module_state_mut::<BulkStore>()
                .ok_or("quota")?
                .take_registry_lease(token(number))
                .ok_or("actual paid lease")?;
            number += 1;
            executor
                .submit(lease, ready)
                .map_err(|(error, _, _)| error)?;
            let mut done = loop {
                if let Some(done) = executor.poll() {
                    break done;
                }
                if Instant::now() >= deadline {
                    return Err("actual marked worker liveness guard".into());
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            };
            measured = measured
                .checked_add(done.receipt.actual_unit_ms)
                .ok_or("measured charge overflow")?;
            let next = done
                .ready
                .take()
                .ok_or("Machine retained through actual ACK")?;
            let settlement = done
                .registry_settlement
                .take()
                .ok_or("actual measured settlement")??;
            let ticket = artifacts(&mut world)?
                .registry
                .insert_settlement(settlement)
                .map_err(|(error, _)| error)?;
            let before = run(&world, binding)?;
            let prepared =
                prepare_transition(&mut world, binding, ticket, number, Transition::Settle)?;
            assert_eq!(run(&world, binding)?, before);
            ack(
                &mut world,
                prepared,
                ticket,
                binding.job,
                number,
                Transition::Settle,
            )?;
            number += 1;
            quanta += 1;
            let after = run(&world, binding)?;
            assert_eq!(after.charged_unit_ms, measured);
            if after.status == Status::Complete {
                drop(next);
                break;
            }
            if after.status.terminal() {
                return Err(format!(
                    "actual marked Registry ended {:?} after {quanta} quanta",
                    after.status
                )
                .into());
            }
            ready = next;
        }
        let receipt: RatingReceipt = typed(&world, "rating_receipts", binding.job)?;
        assert_eq!(receipt.subject(), subject.subject());
        assert_eq!(receipt.reference()?.job(), binding.job);
        assert_eq!(
            receipt.subject().revision().revision(),
            source(&world)?.reference()
        );
        assert!(
            source(&world)?.require_original_access().is_err(),
            "rating never erases lineage"
        );
        // Retain the exact eligible snapshot without rewriting its clock,
        // receipt or immutable parent window. This is the later child's input.
        assert!(world.engine.world.now() < super::parent_end(&world)?);
        let certified = capture(&world, &disk(&world));
        let reference = receipt.reference()?;
        // A separate later world state proves expiry is not boot corruption.
        world.engine.world.set_now(super::parent_end(&world)?);
        let terminal = capture(&world, &disk(&world));
        let cold = PlayerWorld::boot_derived_updates(f, &terminal)?;
        assert_eq!(run(&cold, binding)?.status, Status::Complete);
        assert_eq!(
            typed::<RatingReceipt>(&cold, "rating_receipts", binding.job)?,
            receipt
        );
        cold.tasks.shutdown(Duration::from_secs(5)).await;
        // A fresh new job remains refused. This is not a cached rating ACL.
        let input = input(&mut world)?;
        let before = disk(&world);
        assert!(prepare_admit(&mut world, &input, number).is_err());
        assert_eq!(disk(&world), before);
        eprintln!(
            "M7_DERIVED_UPDATE_CERTIFICATE job={} quanta={quanta} unit_ms={measured}",
            binding.job
        );
        Ok((certified, reference))
    }
    .await;
    tasks.shutdown(Duration::from_secs(5)).await;
    result
}
