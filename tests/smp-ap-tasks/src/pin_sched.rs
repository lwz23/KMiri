// SPDX-License-Identifier: MPL-2.0

//! A scheduler that keeps each task on one CPU: tasks spawned with
//! `Pinned(cpu)` stay on `cpu`, all others on CPU 0. OSTD's default scheduler
//! puts every task on CPU 0, so the APs would never run a task.

use alloc::{boxed::Box, collections::VecDeque, sync::Arc, vec::Vec};

use ostd::{
    cpu::{CpuId, PinCurrentCpu, num_cpus},
    sync::SpinLock,
    task::{
        Task, disable_preempt,
        scheduler::{EnqueueFlags, LocalRunQueue, Scheduler, UpdateFlags, info::CommonSchedInfo},
    },
};

/// Task data that pins a task to a CPU.
pub struct Pinned(pub usize);

/// The CPU the current task runs on.
pub fn cpu() -> usize {
    u32::from(disable_preempt().current_cpu()) as usize
}

/// Installs the scheduler and the AP entry. Call first in `kernel_main`.
pub fn install(ap_entry: fn()) {
    let rq = (0..num_cpus())
        .map(|_| SpinLock::new(RunQueue { current: None, queue: VecDeque::new() }))
        .collect();
    ostd::task::scheduler::inject_scheduler(Box::leak(Box::new(PinScheduler { rq })));
    ostd::boot::smp::register_ap_entry(ap_entry);
}

struct PinScheduler {
    rq: Vec<SpinLock<RunQueue>>,
}

struct RunQueue {
    current: Option<Arc<Task>>,
    queue: VecDeque<Arc<Task>>,
}

impl Scheduler for PinScheduler {
    fn enqueue(&self, runnable: Arc<Task>, _flags: EnqueueFlags) -> Option<CpuId> {
        let place = runnable.data().downcast_ref::<Pinned>().map_or(0, |p| p.0);
        let place = CpuId::try_from(place).unwrap();
        // As in OSTD's FIFO scheduler: a task that is still on a run queue stays there.
        let (still_in_rq, target) = match runnable.cpu().set_if_is_none(place) {
            Err(cpu) => (true, cpu),
            Ok(()) => (false, place),
        };
        let mut rq = self.rq[u32::from(target) as usize].disable_irq().lock();
        if still_in_rq && runnable.cpu().set_if_is_none(target).is_err() {
            return None;
        }
        rq.queue.push_back(runnable);
        None
    }

    fn local_rq_with(&self, f: &mut dyn FnMut(&dyn LocalRunQueue)) {
        let guard = disable_preempt();
        let rq: &RunQueue = &self.rq[u32::from(guard.current_cpu()) as usize].disable_irq().lock();
        f(rq);
    }

    fn mut_local_rq_with(&self, f: &mut dyn FnMut(&mut dyn LocalRunQueue)) {
        let guard = disable_preempt();
        let rq: &mut RunQueue =
            &mut self.rq[u32::from(guard.current_cpu()) as usize].disable_irq().lock();
        f(rq);
    }
}

impl LocalRunQueue for RunQueue {
    fn current(&self) -> Option<&Arc<Task>> {
        self.current.as_ref()
    }

    fn update_current(&mut self, flags: UpdateFlags) -> bool {
        match flags {
            UpdateFlags::Tick => false,
            UpdateFlags::Wait | UpdateFlags::Yield | UpdateFlags::Exit => !self.queue.is_empty(),
        }
    }

    fn try_pick_next(&mut self) -> Option<&Arc<Task>> {
        let next = self.queue.pop_front()?;
        if let Some(prev) = self.current.replace(next) {
            self.queue.push_back(prev);
        }
        self.current.as_ref()
    }

    fn dequeue_current(&mut self) -> Option<Arc<Task>> {
        self.current.take().inspect(|task| task.cpu().set_to_none())
    }
}
