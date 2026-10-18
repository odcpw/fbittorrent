use super::*;
use crate::cx::cap::CapSetRuntimeMask;
use crate::runtime::RuntimeBuilder;
use std::time::Duration;

#[test]
fn reusable_owner_key_distinguishes_real_runtimes_and_handle_domains() {
    let first = RuntimeBuilder::new().worker_threads(1).blocking_threads(1, 2).enable_time().build().unwrap();
    let second = RuntimeBuilder::new().worker_threads(1).blocking_threads(1, 2).enable_time().build().unwrap();
    let first_cx = first.request_cx_with_budget(Budget::INFINITE);
    let second_cx = second.request_cx_with_budget(Budget::INFINITE);
    // Region IDs are runtime-local arenas: equal IDs cannot identify an owner.
    assert_eq!(first_cx.region_id(), second_cx.region_id());
    let key = first_cx.reusable_region_owner_key().unwrap();
    assert_eq!(key, first_cx.clone().reusable_region_owner_key().unwrap());
    assert_ne!(key, second_cx.reusable_region_owner_key().unwrap());
    let changed = first_cx.clone().with_blocking_pool_handle(first_cx.blocking_pool_handle());
    assert_ne!(key, changed.reusable_region_owner_key().unwrap(), "Arc::make_mut changes the authority domain even for the same handle value");
    let child = first.block_on(first_cx.open_child_region(crate::cx::ChildRegionSpec::inherit())).unwrap();
    assert_ne!(key, child.cx().reusable_region_owner_key().unwrap());
    let closed = first.block_on(crate::time::timeout(first_cx.now(), Duration::from_secs(2), child.close()));
    assert!(matches!(closed, Ok(Ok(()))));
    assert!(second.shutdown_timeout(Duration::from_secs(2)));
    assert!(first.shutdown_timeout(Duration::from_secs(2)));
}

#[test]
fn reusable_owner_key_excludes_finite_and_restricted_contexts() {
    let runtime = RuntimeBuilder::new().worker_threads(1).blocking_threads(1, 2).build().unwrap();
    let cx = runtime.request_cx_with_budget(Budget::INFINITE);
    assert!(cx.reusable_region_owner_key().is_some());
    assert!(Cx::for_testing().reusable_region_owner_key().is_none(), "a fabricated context has no actual gateway");
    assert!(cx.clone().with_blocking_pool_handle(None).reusable_region_owner_key().is_none());
    let finite = runtime.request_cx_with_budget(Budget { poll_quota: 100, ..Budget::INFINITE });
    assert!(finite.reusable_region_owner_key().is_none());
    let quota = runtime.request_cx_with_budget(Budget::INFINITE);
    quota.apply_child_capability_budget(CapabilityBudget::UNSPECIFIED.with_io_bytes(1024), crate::types::CapabilityBudgetRequirements::NONE).unwrap();
    assert!(quota.reusable_region_owner_key().is_none());
    for mask in [
        cap::CapSet::<false, true, true, true, true>::MASK,
        cap::CapSet::<true, true, true, false, true>::MASK,
    ] {
        let mut restricted = cx.clone();
        restricted.runtime_mask = mask;
        assert!(restricted.reusable_region_owner_key().is_none());
    }
    let mut regenerated = cx.reusable_region_owner_key().unwrap();
    regenerated.region = RegionId::from_arena(crate::util::ArenaIndex::new(0, 42));
    assert_ne!(regenerated, cx.reusable_region_owner_key().unwrap(), "full generation-bearing RegionId participates in equality");
    assert!(runtime.shutdown_timeout(Duration::from_secs(2)));
    assert!(cx.reusable_region_owner_key().is_none(), "retired gateway cannot create a new reusable owner");
}
