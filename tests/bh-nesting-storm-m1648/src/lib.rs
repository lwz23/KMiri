//! Exercises the native IRQ path with a bottom-half nesting storm.
//!
//! This corresponds to the historical Asterinas issue #1648. In the original
//! code, a nested interrupt is dispatched to the level-2 bottom half, which
//! keeps IRQs disabled, so timer nesting is bounded. The evaluation mutant for
//! #1648 dispatches that level-2 case to the level-1 processor again, which
//! re-enables IRQs at every level and makes nesting grow without bound.
//!
//! This test verifies KMiri's interrupt-design diagnostic on real OSTD. It is
//! not a newly discovered OSTD bug, and its expected mutant failure is the IRQ
//! nesting-limit diagnostic rather than a language-level Miri UB report.
#![no_std]
#![deny(unsafe_code)]
#![feature(format_args_nl)]

extern crate alloc;

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

static TICKS: AtomicUsize = AtomicUsize::new(0);
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static MAX_ACTIVE: AtomicUsize = AtomicUsize::new(0);
static L1_RUNS: AtomicUsize = AtomicUsize::new(0);
static L2_RUNS: AtomicUsize = AtomicUsize::new(0);
static SPIN_CAP_HIT: AtomicBool = AtomicBool::new(false);

fn bottom_half_l1(
    guard: ostd::irq::DisabledLocalIrqGuard,
    _irq_num: u8,
) -> ostd::irq::DisabledLocalIrqGuard {
    let entry_ticks = TICKS.load(Ordering::SeqCst);
    let active = ACTIVE.fetch_add(1, Ordering::SeqCst) + 1;

    let mut observed_max = MAX_ACTIVE.load(Ordering::SeqCst);
    while active > observed_max {
        match MAX_ACTIVE.compare_exchange_weak(
            observed_max,
            active,
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            Ok(_) => break,
            Err(current) => observed_max = current,
        }
    }

    L1_RUNS.fetch_add(1, Ordering::SeqCst);

    // Dropping this guard re-enables IRQs while this level-1 bottom half is
    // still active. The original kernel handles a nested IRQ at level 2; the
    // #1648 mutant recursively enters this level-1 path.
    drop(guard);

    let mut spins = 0usize;
    while TICKS.load(Ordering::SeqCst) < entry_ticks.saturating_add(2) {
        if spins >= 1_000_000 {
            SPIN_CAP_HIT.store(true, Ordering::SeqCst);
            break;
        }
        spins += 1;
        core::hint::spin_loop();
    }

    ACTIVE.fetch_sub(1, Ordering::SeqCst);
    ostd::irq::disable_local()
}

fn bottom_half_l2(_irq_num: u8) {
    L2_RUNS.fetch_add(1, Ordering::SeqCst);
}

#[ostd::ktest::miri_main]
fn kernel_main() {
    ostd::irq::register_bottom_half_handler_l1(bottom_half_l1);
    ostd::irq::register_bottom_half_handler_l2(bottom_half_l2);
    ostd::timer::register_callback_on_cpu(|| {
        TICKS.fetch_add(1, Ordering::SeqCst);
    });

    while L1_RUNS.load(Ordering::SeqCst) < 3
        || L2_RUNS.load(Ordering::SeqCst) < 2
        || TICKS.load(Ordering::SeqCst) < 6
    {
        core::hint::spin_loop();
    }

    let ticks = TICKS.load(Ordering::SeqCst);
    let l1_runs = L1_RUNS.load(Ordering::SeqCst);
    let l2_runs = L2_RUNS.load(Ordering::SeqCst);
    let max_active = MAX_ACTIVE.load(Ordering::SeqCst);
    let spin_cap_hit = SPIN_CAP_HIT.load(Ordering::SeqCst);
    ostd::miri_println!(
        "bh-nesting-storm: ticks={} l1_runs={} l2_runs={} max_active={} spin_cap_hit={}",
        ticks,
        l1_runs,
        l2_runs,
        max_active,
        spin_cap_hit
    );
    assert_eq!(max_active, 1);
}
