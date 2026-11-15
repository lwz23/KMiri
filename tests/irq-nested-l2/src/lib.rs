//! Verifies that a timer IRQ arriving during an L1 bottom half can enter L2.
//!
//! The L1 handler must drop its disabled-IRQ guard so that the CPU accepts a
//! nested timer IRQ. With the Step 5b-1 timer gate, timer events were not
//! generated while any IRQ was active, so this test would wait forever.
#![no_std]
#![deny(unsafe_code)]
#![feature(format_args_nl)]

extern crate alloc;

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

static TICKS: AtomicUsize = AtomicUsize::new(0);
static L1_RUNS: AtomicUsize = AtomicUsize::new(0);
static L2_RUNS: AtomicUsize = AtomicUsize::new(0);
static L1_OPENED: AtomicBool = AtomicBool::new(false);

fn bottom_half_l1(
    guard: ostd::irq::DisabledLocalIrqGuard,
    _irq_num: u8,
) -> ostd::irq::DisabledLocalIrqGuard {
    L1_RUNS.fetch_add(1, Ordering::SeqCst);
    if !L1_OPENED.swap(true, Ordering::SeqCst) {
        drop(guard);
        while L2_RUNS.load(Ordering::SeqCst) == 0 {
            core::hint::spin_loop();
        }
        ostd::irq::disable_local()
    } else {
        guard
    }
}

fn bottom_half_l2(_irq_num: u8) {
    assert_eq!(
        ostd::irq::InterruptLevel::current(),
        ostd::irq::InterruptLevel::L2
    );
    L2_RUNS.fetch_add(1, Ordering::SeqCst);
}

#[ostd::ktest::miri_main]
fn kernel_main() {
    ostd::irq::register_bottom_half_handler_l1(bottom_half_l1);
    ostd::irq::register_bottom_half_handler_l2(bottom_half_l2);
    ostd::timer::register_callback_on_cpu(|| {
        TICKS.fetch_add(1, Ordering::SeqCst);
    });

    while L2_RUNS.load(Ordering::SeqCst) < 1 || TICKS.load(Ordering::SeqCst) < 3 {
        core::hint::spin_loop();
    }

    let ticks = TICKS.load(Ordering::SeqCst);
    let l1_runs = L1_RUNS.load(Ordering::SeqCst);
    let l2_runs = L2_RUNS.load(Ordering::SeqCst);
    ostd::miri_println!(
        "irq-nested-l2: ticks={} l1_runs={} l2_runs={}",
        ticks,
        l1_runs,
        l2_runs
    );
    assert!(l2_runs >= 1);
}
