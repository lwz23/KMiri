//! Reproduce Asterinas issue #3896/#1120 from a safe OSTD client.
//!
//! A safe client may create nested `DisabledLocalIrqGuard`s and pass either one
//! to `Rcu::read_with`. Dropping the outer guard first re-enables IRQs while
//! the inner guard still exists. `Rcu` trusts that surviving guard as proof of
//! atomic mode, so a subsequent `Task::yield_now` can switch tasks and finish
//! the RCU grace period. The old payload is then reclaimed while `read` lives.
//!
//! KMiri does not yet model object lifetime in OSDK's slab allocator slots, so
//! a plain `u64` load can observe the freelist pointer without an error (see
//! `step-b1/REPORT-stop2.md`). `Tag` is an 8-byte enum whose only valid values
//! are `0xAA` and `0xBB`; the typed copy below detects the consequence of the
//! stale read when the freed slot contains an invalid tag. This reports the
//! UAF's consequence, not the UAF itself; after heap lifetime modeling lands,
//! the payload should return to a plain integer so Miri reports use-after-free
//! directly. The 8-byte size also avoids the unrelated 4-byte allocator
//! freelist diagnosis recorded in `step-b1/REPORT-stop1.md`.
//!
//! The cfg(miri) IRQ observation helper is not part of the OSTD contract and is
//! deliberately unused: the trigger behavior must not depend on it.
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
    drop(outer);

    rcu.update(Box::new(Payload { tag: Tag::B }));

    let mut yields = 0;
    while !RECLAIMED.load(Ordering::Acquire) && yields < 100_000 {
        Task::yield_now();
        yields += 1;
    }
    let reclaimed = RECLAIMED.load(Ordering::Acquire);

    // Issue #3896: the old payload has been dropped, but `read` is still live.
    // This typed copy must reject the allocator-written freelist tag.
    let tag: Tag = read.deref_target().tag;

    drop(inner);
    DONE.store(true, Ordering::Release);
    ostd::miri_println!(
        "rcu-guard-order-3896: reclaimed={} yields={} val={}",
        reclaimed,
        yields,
        tag as u64
    );
}
