//! The replay engine and its parts: `Bucket` and `Gate` (Task 2), `Engine` (Tasks 3 and 4).

use kaanal_core::replay::{Admit, Bucket, Gate};
use kaanal_core::topology::{Admission, RateLimit};

const MS: u64 = 1_000_000;

fn bucket(interval_ms: u64, burst: u32) -> Bucket {
    Bucket::new(&RateLimit {
        interval_ns: interval_ms * MS,
        burst,
    })
}

fn gate(slots: u32, queue: Option<u32>) -> Gate {
    Gate::new(&Admission {
        slots,
        queue,
        overflow_status: 503,
    })
}

#[test]
fn bucket_with_burst_one_allows_one_then_asks_for_a_full_interval() {
    let mut b = bucket(10, 1);
    assert_eq!(b.take(0), Ok(()));
    assert_eq!(b.take(0), Err(10 * MS));
    assert_eq!(b.take(4 * MS), Err(6 * MS));
    assert_eq!(b.take(10 * MS), Ok(()));
}

#[test]
fn bucket_starts_full_and_refills_one_token_per_interval() {
    let mut b = bucket(10, 3);
    for _ in 0..3 {
        assert_eq!(b.take(0), Ok(()));
    }
    assert_eq!(b.take(0), Err(10 * MS), "the fourth at once is refused");
    assert_eq!(b.take(10 * MS), Ok(()), "one token came back");
    assert_eq!(b.take(10 * MS), Err(10 * MS));
}

#[test]
fn bucket_never_banks_more_than_its_burst() {
    let mut b = bucket(10, 2);
    assert_eq!(b.take(0), Ok(()));
    let hour = 3_600_000 * MS;
    assert_eq!(b.take(hour), Ok(()));
    assert_eq!(b.take(hour), Ok(()));
    assert_eq!(
        b.take(hour),
        Err(10 * MS),
        "an idle hour still buys only the burst"
    );
}

#[test]
fn refusing_does_not_spend_a_token() {
    let mut b = bucket(10, 1);
    assert_eq!(b.take(0), Ok(()));
    for _ in 0..5 {
        assert!(b.take(MS).is_err());
    }
    assert_eq!(
        b.take(10 * MS),
        Ok(()),
        "refusals did not push the next token away"
    );
}

#[test]
fn extreme_buckets_do_not_overflow() {
    let mut huge = Bucket::new(&RateLimit {
        interval_ns: u64::MAX,
        burst: u32::MAX,
    });
    assert_eq!(huge.take(u64::MAX), Ok(()));
    assert_eq!(
        huge.take(u64::MAX),
        Ok(()),
        "a 4-billion burst is still open"
    );
    let mut tiny = Bucket::new(&RateLimit {
        interval_ns: 1,
        burst: 1,
    });
    for t in 0..1000 {
        assert_eq!(tiny.take(t), Ok(()), "1 token per ns at 1 GHz");
    }
    assert_eq!(tiny.take(999), Err(1));
}

#[test]
fn gate_runs_up_to_its_slots_then_queues_then_overflows() {
    let mut g = gate(1, Some(1));
    assert_eq!(g.arrive((0, 0)), Admit::Run);
    assert_eq!(g.arrive((1, 0)), Admit::Queued);
    assert_eq!(g.arrive((2, 0)), Admit::Overflow);
    assert_eq!((g.busy(), g.queued(), g.slots()), (1, 1, 1));
    assert_eq!(g.overflow_status(), 503);
}

#[test]
fn a_queue_of_zero_overflows_as_soon_as_every_slot_is_busy() {
    let mut g = gate(2, Some(0));
    assert_eq!(g.arrive((0, 0)), Admit::Run);
    assert_eq!(g.arrive((1, 0)), Admit::Run);
    assert_eq!(g.arrive((2, 0)), Admit::Overflow);
    assert_eq!(g.queued(), 0);
}

#[test]
fn release_hands_the_slot_to_the_oldest_waiter_without_freeing_it() {
    let mut g = gate(1, None);
    assert_eq!(g.arrive((0, 0)), Admit::Run);
    assert_eq!(g.arrive((1, 0)), Admit::Queued);
    assert_eq!(g.arrive((2, 0)), Admit::Queued);
    assert_eq!(g.release(), Some((1, 0)), "FIFO");
    assert_eq!(g.busy(), 1, "the slot never looked free");
    assert_eq!(
        g.arrive((3, 0)),
        Admit::Queued,
        "a later arrival cannot barge"
    );
    assert_eq!(g.release(), Some((2, 0)));
    assert_eq!(g.release(), Some((3, 0)));
    assert_eq!(g.release(), None);
    assert_eq!(g.busy(), 0);
}

#[test]
fn an_unbounded_queue_never_overflows() {
    let mut g = gate(1, None);
    assert_eq!(g.arrive((0, 0)), Admit::Run);
    for i in 1..10_000 {
        assert_eq!(g.arrive((i, 0)), Admit::Queued);
    }
    assert_eq!(g.queued(), 9_999);
}

#[test]
fn cancel_removes_one_waiter_and_keeps_the_order_of_the_rest() {
    let mut g = gate(1, None);
    g.arrive((0, 0));
    for i in 1..=3 {
        g.arrive((i, 0));
    }
    g.cancel((2, 0));
    assert_eq!(g.release(), Some((1, 0)));
    assert_eq!(g.release(), Some((3, 0)));
    g.cancel((9, 9)); // not queued: no effect
}
