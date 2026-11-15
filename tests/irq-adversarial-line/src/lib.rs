//! Exercises a second, adversarial IRQ event source on a real OSTD IRQ line.
//!
//! Vector 0x40 is in priority class 4, above the timer's class-2 vector 32.
//! In phase one, the allocated line counts its top-half callbacks until at
//! least three adversarial IRQs have been delivered. In phase two, the line is
//! dropped: KMiri may still raise 0x40 as a spurious device IRQ, but OSTD must
//! safely dispatch and acknowledge it without a registered callback.
#![no_std]
#![deny(unsafe_code)]
#![feature(format_args_nl)]

extern crate alloc;

use core::sync::atomic::{AtomicUsize, Ordering};

static HITS: AtomicUsize = AtomicUsize::new(0);
static TICKS: AtomicUsize = AtomicUsize::new(0);

#[ostd::ktest::miri_main]
fn kernel_main() {
    ostd::timer::register_callback_on_cpu(|| {
        TICKS.fetch_add(1, Ordering::SeqCst);
    });

    let mut line = ostd::irq::IrqLine::alloc_specific(0x40).unwrap();
    line.on_active(|_| {
        HITS.fetch_add(1, Ordering::SeqCst);
    });

    while HITS.load(Ordering::SeqCst) < 3 {
        core::hint::spin_loop();
    }
    let hits_phase1 = HITS.load(Ordering::SeqCst);
    assert!(hits_phase1 >= 3);

    drop(line);
    let ticks_at_drop = TICKS.load(Ordering::SeqCst);
    assert!(ticks_at_drop < usize::MAX - 3);

    // At least three further timer ticks give the independent adversarial
    // source many opportunities to raise the now-unhandled 0x40 line.
    while TICKS.load(Ordering::SeqCst) < ticks_at_drop + 3 {
        core::hint::spin_loop();
    }
    let ticks = TICKS.load(Ordering::SeqCst);
    let hits_after_drop = HITS.load(Ordering::SeqCst) - hits_phase1;
    assert_eq!(hits_after_drop, 0);

    ostd::miri_println!(
        "irq-adversarial-line: hits_phase1={} hits_after_drop={} ticks={}",
        hits_phase1,
        hits_after_drop,
        ticks
    );
}
