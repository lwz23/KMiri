// SPDX-License-Identifier: MPL-2.0

//! Three CPUs. Each AP runs two tasks pinned to it; they count under a lock
//! shared by all CPUs. Then an RCU grace period must complete, which needs
//! every CPU to switch tasks. KMiri must report nothing.
//!
//! The AP tasks never end, like the idle loops of a kernel: the program ends
//! when `kernel_main` returns, which powers the machine off.

#![no_std]
#![deny(unsafe_code)]
#![feature(format_args_nl)]

extern crate alloc;

mod pin_sched;

use alloc::boxed::Box;
use core::sync::atomic::{AtomicBool, Ordering::SeqCst};

use ostd::{
    cpu::num_cpus,
    sync::{Rcu, SpinLock},
    task::{Task, TaskOptions},
};
use pin_sched::Pinned;

const NUM_CPUS: usize = 3;

static RAN: [AtomicBool; NUM_CPUS] = [const { AtomicBool::new(false) }; NUM_CPUS];
static COUNT: SpinLock<usize> = SpinLock::new(0);
static RECLAIMED: AtomicBool = AtomicBool::new(false);
static STOP: AtomicBool = AtomicBool::new(false);
static DRIVER_DONE: AtomicBool = AtomicBool::new(false);

struct Payload(u64);

impl Drop for Payload {
    fn drop(&mut self) {
        if self.0 == 1 {
            RECLAIMED.store(true, SeqCst);
        }
    }
}

fn ap_entry() {
    let cpu = pin_sched::cpu();
    for _ in 0..2 {
        TaskOptions::new(move || {
            loop {
                assert_eq!(pin_sched::cpu(), cpu, "a pinned task left its CPU");
                RAN[cpu].store(true, SeqCst);
                *COUNT.lock() += 1;
                Task::yield_now();
            }
        })
        .data(Pinned(cpu))
        .spawn()
        .unwrap();
    }
}

fn wait_until(what: &str, cond: impl Fn() -> bool) {
    for _ in 0..10_000 {
        if cond() {
            return;
        }
        Task::yield_now();
    }
    panic!("timed out waiting until {what}");
}

#[ostd::ktest::miri_main]
fn kernel_main() {
    assert_eq!(num_cpus(), NUM_CPUS);
    pin_sched::install(ap_entry);

    TaskOptions::new(|| {
        // A second task on CPU 0, so that CPU 0 switches tasks too.
        let _peer = TaskOptions::new(|| {
            while !STOP.load(SeqCst) {
                Task::yield_now();
            }
        })
        .spawn()
        .unwrap();

        wait_until("every AP ran a task", || RAN[1..].iter().all(|r| r.load(SeqCst)));

        let rcu = Rcu::new(Box::new(Payload(1)));
        rcu.update(Box::new(Payload(2)));
        wait_until("an RCU grace period passed", || RECLAIMED.load(SeqCst));

        STOP.store(true, SeqCst);
        assert!(*COUNT.lock() >= NUM_CPUS - 1);
        ostd::miri_println!("smp-ap-tasks: ok, count = {}", *COUNT.lock());
        DRIVER_DONE.store(true, SeqCst);
    })
    .spawn()
    .unwrap();

    while !DRIVER_DONE.load(SeqCst) {
        Task::yield_now();
    }
}
