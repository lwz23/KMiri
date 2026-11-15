//! Verifies that KMiri timer injection reaches Asterinas' native interrupt
//! path (`trap_handler` -> top_half -> bottom_half), not a shortcut.
//!
//! Discriminator: the bottom-half L1 handler can only run if the injection
//! goes through `call_irq_callback_functions`. The legacy shortcut
//! (`call_timer_callback_functions_from_miri`) never calls bottom_half,
//! so BOTTOM_RUNS stays 0 there and the final assert fails.
#![no_std]
#![deny(unsafe_code)]
#![feature(format_args_nl)]

extern crate alloc;

use core::sync::atomic::{AtomicUsize, Ordering};

static TICKS: AtomicUsize = AtomicUsize::new(0);
static BOTTOM_RUNS: AtomicUsize = AtomicUsize::new(0);

fn bottom_half_l1(
    guard: ostd::irq::DisabledLocalIrqGuard,
    _irq_num: u8,
) -> ostd::irq::DisabledLocalIrqGuard {
    BOTTOM_RUNS.fetch_add(1, Ordering::SeqCst);
    // v1 keeps IRQs disabled throughout (single-level interrupts),
    // so the guard is returned without being dropped.
    guard
}

#[ostd::ktest::miri_main]
fn kernel_main() {
    ostd::irq::register_bottom_half_handler_l1(bottom_half_l1);
    ostd::timer::register_callback_on_cpu(|| {
 TICKS.fetch_add(1, Ordering::SeqCst);
    });

    // Spin to generate basic blocks so the interpreter injects timer IRQs.
    while TICKS.load(Ordering::SeqCst) < 3 {
 core::hint::spin_loop();
    }

    let ticks = TICKS.load(Ordering::SeqCst);
    let bottoms = BOTTOM_RUNS.load(Ordering::SeqCst);
    ostd::miri_println!("irq-trap-path: ticks={} bottom_half_runs={}", ticks, bottoms);
    assert!(
        bottoms >= 1,
 "bottom half never ran: timer IRQs bypassed the native trap_handler path"
    );
    ostd::miri_println!("irq-trap-path: native trap path verified");
}
