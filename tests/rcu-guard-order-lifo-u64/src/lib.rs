//! Correct LIFO control for the issue #3896/#1120 sequence.
//!
//! The RCU reference and both IRQ guards are released before `update` and any
//! task switch. Therefore the grace period can finish without invalidating a
//! live read reference, and the old value must remain observable as 0xAA.
//! The payload is exactly 8 bytes to avoid the unrelated 4-byte allocator
//! freelist diagnosis recorded in `step-b1/REPORT-stop1.md`; this keeps the
//! deallocation write inside the Box's own layout.
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
    let val = read.deref_target().val;
    assert_eq!(val, 0xAA);

    // Correct LIFO order: invalidate the read before enabling task switches.
    drop(read);
    drop(inner);
    drop(outer);

    rcu.update(Box::new(Payload { val: 0xBB }));

    let mut yields = 0;
    while !RECLAIMED.load(Ordering::Acquire) && yields < 100_000 {
        Task::yield_now();
        yields += 1;
    }
    let reclaimed = RECLAIMED.load(Ordering::Acquire);
    assert!(reclaimed);

    DONE.store(true, Ordering::Release);
    // Let the peer observe DONE and exit before the kernel finishes.
    Task::yield_now();

    ostd::miri_println!(
        "rcu-guard-order-lifo: reclaimed={} yields={} val={}",
        reclaimed,
        yields,
        val
    );
}
