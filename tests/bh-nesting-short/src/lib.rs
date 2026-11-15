// SPDX-License-Identifier: MPL-2.0

//! Checks that bottom-half nesting stays bounded when IRQs arrive exactly as a
//! bottom half re-enables them.
//!
//! Each L1 bottom half drops its guard and spins only ten iterations before
//! disabling IRQs again. Periodic timer ticks rarely land in such a short
//! window, but critical-point injection raises a device IRQ right at the
//! `IF: false -> true` transition. OSTD handles the nested IRQ at L2, which
//! keeps IRQs disabled, so at most one L1 bottom half is active at a time. If
//! nested IRQs went to L1 again (Asterinas issue #1648), nesting would grow
//! until KMiri reports "IRQ nesting limit reached".
//!
//! Expected output: `bh-nesting-short: ... max_active=1 ...`. Needs timer
//! ticks and critical-point injection, see `kmiri.toml`.

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
