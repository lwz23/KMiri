//! Short-window bottom-half nesting benchmark modeled after Asterinas #1648.
//!
//! Each level-1 bottom half re-enables local IRQs by dropping its guard and
//! spins for only ten `spin_loop()` iterations before disabling IRQs again.
//! Because the open-interrupt window is tiny, periodic timer ticks rarely land
//! inside it; critical-point injection on `IF: false -> true` directly probes
//! this short window.
//!
//! Original OSTD handles nested interrupts in level 2 (keeping IRQs disabled,
//! so `max_active == 1`). The #1648 mutant dispatches level 2 back to level 1,
//! causing unbounded nesting that KMiri catches at `max_nesting_depth`.
#![no_std]
#![deny(unsafe_code)]
#![feature(format_args_nl)]

extern crate alloc;

use core::sync::atomic::{AtomicUsize, Ordering};

static TICKS: AtomicUsize = AtomicUsize::new(0);
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static MAX_ACTIVE: AtomicUsize = AtomicUsize::new(0);
static L1_RUNS: AtomicUsize = AtomicUsize::new(0);
static L2_RUNS: AtomicUsize = AtomicUsize::new(0);
static HITS: AtomicUsize = AtomicUsize::new(0);

fn bottom_half_l1(
    guard: ostd::irq::DisabledLocalIrqGuard,
    _irq_num: u8,
) -> ostd::irq::DisabledLocalIrqGuard {
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

    drop(guard);

    for _ in 0..10 {
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

    let mut line = ostd::irq::IrqLine::alloc_specific(0x40).unwrap();
    line.on_active(|_| {
        HITS.fetch_add(1, Ordering::SeqCst);
    });

    while L1_RUNS.load(Ordering::SeqCst) < 20 || TICKS.load(Ordering::SeqCst) < 3 {
        core::hint::spin_loop();
    }

    let l1_runs = L1_RUNS.load(Ordering::SeqCst);
    let l2_runs = L2_RUNS.load(Ordering::SeqCst);
    let max_active = MAX_ACTIVE.load(Ordering::SeqCst);
    let hits = HITS.load(Ordering::SeqCst);
    ostd::miri_println!(
        "bh-nesting-short: l1_runs={} l2_runs={} max_active={} hits={}",
        l1_runs,
        l2_runs,
        max_active,
        hits
    );
    assert_eq!(max_active, 1);
    drop(line);
}
