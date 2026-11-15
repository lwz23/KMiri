//! Diagnostic copy of the correct LIFO control with a 4-byte payload.
//!
//! This probe is not a Step B1 verdict. It isolates the allocator observation
//! from `step-b1/REPORT-stop1.md`: a `Box<u32-sized payload>` uses an 8-byte
//! slab slot, and `SlabSlotList::<8>::push` writes an 8-byte freelist pointer.
//! The experiment is run once with Stacked Borrows and once with Tree Borrows.
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
    val: u32,
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
    Task::yield_now();

    ostd::miri_println!(
        "rcu-alloc-4byte-probe: reclaimed={} yields={} val={}",
        reclaimed,
        yields,
        val
    );
}
