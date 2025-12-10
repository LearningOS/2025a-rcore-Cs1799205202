//! Semaphore

use crate::sync::UPSafeCell;
use crate::task::{block_current_and_run_next, current_task, wakeup_task, TaskControlBlock};
use alloc::{collections::VecDeque, sync::Arc};
use alloc::vec::Vec;

/// semaphore structure
pub struct Semaphore {
    /// semaphore inner
    pub inner: UPSafeCell<SemaphoreInner>,
}

pub struct SemaphoreInner {
    pub count: isize,
    pub wait_queue: VecDeque<Arc<TaskControlBlock>>,
    pub holders: Vec<usize>,
}

impl Semaphore {
    /// Create a new semaphore
    pub fn new(res_count: usize) -> Self {
        trace!("kernel: Semaphore::new");
        Self {
            inner: unsafe {
                UPSafeCell::new(SemaphoreInner {
                    count: res_count as isize,
                    wait_queue: VecDeque::new(),
                    holders: Vec::new(),
                })
            },
        }
    }

    /// up operation of semaphore
    pub fn up(&self) {
        trace!("kernel: Semaphore::up");
        let mut inner = self.inner.exclusive_access();
        inner.count += 1;
        // Remove one instance of current task from holders if present
        // If not present, it means it's a pure production, so we don't remove anything.
        // But wait, if we don't remove, Allocation matrix will be wrong if we assume up() releases resource.
        // If up() is called by someone who holds it, we should remove.
        // If up() is called by someone who doesn't hold it, we shouldn't remove.
        // Let's try to remove the current task.
        let tid = current_task().unwrap().inner_exclusive_access().res.as_ref().unwrap().tid;
        if let Some(pos) = inner.holders.iter().position(|&x| x == tid) {
            inner.holders.remove(pos);
        }

        if inner.count <= 0 {
            if let Some(task) = inner.wait_queue.pop_front() {
                // The woken task will "hold" the resource effectively?
                // No, down() decrements count.
                // If count <= 0, it means there are waiters.
                // up() increments count. If count <= 0, it wakes up one waiter.
                // The waiter continues from down().
                // In down(), if count < 0, it blocks.
                // When woken up, it should be considered as having acquired the resource?
                // In standard semaphore, when woken up, it consumes the unit produced by up().
                // So we should add the woken task to holders.
                let woken_tid = task.inner_exclusive_access().res.as_ref().unwrap().tid;
                inner.holders.push(woken_tid);
                wakeup_task(task);
            }
        }
    }

    /// down operation of semaphore
    pub fn down(&self) {
        trace!("kernel: Semaphore::down");
        let mut inner = self.inner.exclusive_access();
        inner.count -= 1;
        if inner.count < 0 {
            inner.wait_queue.push_back(current_task().unwrap());
            drop(inner);
            block_current_and_run_next();
        } else {
            inner.holders.push(current_task().unwrap().inner_exclusive_access().res.as_ref().unwrap().tid);
        }
    }

    /// Get holders
    pub fn get_holders(&self) -> Vec<usize> {
        self.inner.exclusive_access().holders.clone()
    }

    /// Get waiting tasks
    pub fn get_waiting_tasks(&self) -> Vec<usize> {
        self.inner.exclusive_access().wait_queue.iter().map(|task| task.inner_exclusive_access().res.as_ref().unwrap().tid).collect()
    }
}
