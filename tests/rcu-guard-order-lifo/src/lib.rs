//! Correct LIFO control for the issue #3896/#1120 sequence.
//!
//! The RCU reference and both IRQ guards are released before `update` and any
//! task switch. Therefore the grace period can finish without invalidating a
//! live read reference, and the old value must remain observable as 0xAA.
//!
//! `Tag` is used for parity with the bug test. Its 8-byte size matches the
//! allocator slot and avoids the unrelated 4-byte freelist diagnosis in
//! `step-b1/REPORT-stop1.md`. The bug test explains in `REPORT-stop2.md` why an
//! invalid enum tag is currently used to detect the consequence of a UAF; this
//! control performs the same typed copy while the payload is still live.
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

#[repr(u64)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tag {
    A = 0xAA,
    B = 0xBB,
}

struct Payload {
    tag: Tag,
}

impl Drop for Payload {
    fn drop(&mut self) {
        RECLAIMED.store(true, Ordering::Release);
    }
}

fn spawn_peer() {
    // 1,000 yields keep another runnable task available while avoiding a long
    // post-test tail; the main test observes reclamation after its first yield.
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

    let rcu = Rcu::new(Box::new(Payload { tag: Tag::A }));
    let outer = ostd::irq::disable_local();
    let inner = ostd::irq::disable_local();
    let read = rcu.read_with(&inner);
    let tag = read.deref_target().tag;
    assert_eq!(tag, Tag::A);

    // Correct LIFO order: invalidate the read before enabling task switches.
    drop(read);
    drop(inner);
    drop(outer);

    rcu.update(Box::new(Payload { tag: Tag::B }));

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
        tag as u64
    );
}
