//! The slot limit of one node: running slots, a FIFO wait queue with an optional cap.

use std::collections::VecDeque;

use crate::topology::Admission;

/// Who waits: (request table slot, token index), as the engine names them.
pub type Waiter = (usize, usize);

/// What [`Gate::arrive`] decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admit {
    /// A slot was free and is now held.
    Run,
    /// Every slot is busy; the waiter is in the queue.
    Queued,
    /// Every slot is busy and the queue is full.
    Overflow,
}

/// Slots, a FIFO queue and an overflow status.
#[derive(Debug, Clone)]
pub struct Gate {
    slots: u32,
    busy: u32,
    queue: VecDeque<Waiter>,
    cap: Option<u32>,
    overflow_status: u16,
}

impl Gate {
    /// A gate with every slot free.
    pub fn new(a: &Admission) -> Gate {
        Gate {
            slots: a.slots,
            busy: 0,
            queue: VecDeque::new(),
            cap: a.queue,
            overflow_status: a.overflow_status,
        }
    }

    /// Takes a free slot, else queues, else overflows. A queue cap of `None` never overflows.
    pub fn arrive(&mut self, w: Waiter) -> Admit {
        if self.busy < self.slots {
            self.busy += 1;
            Admit::Run
        } else if self.cap.is_none_or(|c| self.queue.len() < c as usize) {
            self.queue.push_back(w);
            Admit::Queued
        } else {
            Admit::Overflow
        }
    }

    /// A running hop ended. The oldest waiter, if any, gets the slot at once (the slot never
    /// looks free to a later arrival: no barging); otherwise the slot is freed.
    pub fn release(&mut self) -> Option<Waiter> {
        // the slot limit was lowered below the slots in use (a live change): this slot is not
        // handed on, it just closes, so the limit is reached as running work finishes
        if self.busy > self.slots {
            self.busy -= 1;
            return None;
        }
        let next = self.queue.pop_front();
        if next.is_none() {
            self.busy -= 1;
        }
        next
    }

    /// Changes the limits of a running gate (a live parameter edit). Running work is never cut:
    /// lowering `slots` below the slots in use only stops new work from starting until enough
    /// finish (see [`Gate::release`]); raising it starts waiting work at once, oldest first, and
    /// returns those waiters. A lower queue cap refuses new arrivals; it never evicts a waiter.
    pub fn set_limits(
        &mut self,
        slots: u32,
        cap: Option<u32>,
        overflow_status: u16,
    ) -> Vec<Waiter> {
        self.slots = slots;
        self.cap = cap;
        self.overflow_status = overflow_status;
        let mut started = Vec::new();
        while self.busy < self.slots {
            let Some(w) = self.queue.pop_front() else {
                break;
            };
            self.busy += 1;
            started.push(w);
        }
        started
    }

    /// Removes a queued waiter (its request failed); the others keep their order.
    pub fn cancel(&mut self, w: Waiter) {
        self.queue.retain(|&e| e != w);
    }

    /// Slots in use.
    pub fn busy(&self) -> u32 {
        self.busy
    }

    /// Waiters in the queue.
    pub fn queued(&self) -> usize {
        self.queue.len()
    }

    /// Slot count.
    pub fn slots(&self) -> u32 {
        self.slots
    }

    /// Status of the failure when the queue is full.
    pub fn overflow_status(&self) -> u16 {
        self.overflow_status
    }
}
