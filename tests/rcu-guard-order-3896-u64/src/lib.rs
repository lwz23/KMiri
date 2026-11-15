//! Reproduce Asterinas issue #3896/#1120 from a safe OSTD client.
//!
//! A safe client may create nested `DisabledLocalIrqGuard`s and pass either one
//! to `Rcu::read_with`. Dropping the outer guard first re-enables IRQs while
//! the inner guard still exists. `Rcu` trusts that surviving guard as proof of
//! atomic mode, so a subsequent `Task::yield_now` can switch tasks and finish
//! the RCU grace period. Accessing the old read reference after its payload is
//! reclaimed must make Miri report use-after-free at this read.
//! The payload is exactly 8 bytes to avoid the unrelated 4-byte allocator
//! freelist diagnosis recorded in `step-b1/REPORT-stop1.md`; this keeps the
//! deallocation write inside the Box's own layout.
//! The cfg(miri) IRQ observation helper is not part of the OSTD contract and
//! is deliberately unused: the trigger behavior must not depend on it.
#![no_std]
#![deny(unsafe_code)]
#![feature(format_args_nl)]

extern crate alloc;

use alloc::boxed::Box;
use core::sync::atomic::{AtomicBool, Ordering};
use ostd::sync::Rcu;
use ostd::task::{Task, TaskOptions};

static RECLAIMED: AtomicBool = AtomicBool::new(false);
static DONE: AtomicBool = AtomicBool::new(false);

struct Payload {
    val: u64,
}

impl Drop for Payload {
    fn drop(&mut self) {
        RECLAIMED.store(true, Ordering::Release);
    }
}

fn spawn_peer() {
    TaskOptions::new(|| {
        let mut remaining = 1_000;
        while !DONE.load(Ordering::Acquire) && remaining > 0 {
            Task::yield_now();
            remaining -= 1;
        }
    })
    .spawn()
    .unwrap();
}

#[ostd::ktest::miri_main]
fn kernel_main() {
    spawn_peer();

    let rcu = Rcu::new(Box::new(Payload { val: 0xAA }));
    let outer = ostd::irq::disable_local();
    let inner = ostd::irq::disable_local();
    let read = rcu.read_with(&inner);
    drop(outer);

    rcu.update(Box::new(Payload { val: 0xBB }));

    let mut yields = 0;
    while !RECLAIMED.load(Ordering::Acquire) && yields < 100_000 {
        Task::yield_now();
        yields += 1;
    }
    let reclaimed = RECLAIMED.load(Ordering::Acquire);

    // Issue #3896: the old payload has been dropped, but `read` is still live.
    // Miri must report UB for this access.
    let val = read.deref_target().val;

    drop(inner);
    DONE.store(true, Ordering::Release);
    ostd::miri_println!(
        "rcu-guard-order-3896: reclaimed={} yields={} val={}",
        reclaimed,
        yields,
        val
    );
}
