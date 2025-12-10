use crate::sync::{Condvar, Mutex, MutexBlocking, MutexSpin, Semaphore};
use crate::task::{block_current_and_run_next, current_process, current_task, ProcessControlBlock};
use crate::timer::{add_timer, get_time_ms};
use alloc::sync::Arc;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Resource {
    Mutex(usize),
    Semaphore(usize),
}

fn check_deadlock(process: &Arc<ProcessControlBlock>, request: Resource) -> bool {
    let inner = process.inner_exclusive_access();
    if !inner.deadlock_detection_enabled {
        return false;
    }
    
    let mut available: BTreeMap<Resource, usize> = BTreeMap::new();
    let mut allocation: BTreeMap<usize, Vec<Resource>> = BTreeMap::new();
    let mut need: BTreeMap<usize, Option<Resource>> = BTreeMap::new();
    let mut tasks: Vec<usize> = Vec::new();

    // Collect tasks
    for (tid, task) in inner.tasks.iter().enumerate() {
        if task.is_some() {
            tasks.push(tid);
            allocation.insert(tid, Vec::new());
            need.insert(tid, None);
        }
    }

    // Collect Mutex info
    for (id, mutex) in inner.mutex_list.iter().enumerate() {
        if let Some(mutex) = mutex {
            if let Some(owner) = mutex.get_owner() {
                available.insert(Resource::Mutex(id), 0);
                if let Some(alloc_vec) = allocation.get_mut(&owner) {
                    alloc_vec.push(Resource::Mutex(id));
                }
            } else {
                available.insert(Resource::Mutex(id), 1);
            }
            
            for tid in mutex.get_waiting_tasks() {
                if let Some(n) = need.get_mut(&tid) {
                    *n = Some(Resource::Mutex(id));
                }
            }
        }
    }

    // Collect Semaphore info
    for (id, sem) in inner.semaphore_list.iter().enumerate() {
        if let Some(sem) = sem {
            let holders = sem.get_holders();
            let count = sem.inner.exclusive_access().count;
            available.insert(Resource::Semaphore(id), if count > 0 { count as usize } else { 0 });
            
            for tid in holders {
                if let Some(alloc_vec) = allocation.get_mut(&tid) {
                    alloc_vec.push(Resource::Semaphore(id));
                }
            }

            for tid in sem.get_waiting_tasks() {
                if let Some(n) = need.get_mut(&tid) {
                    *n = Some(Resource::Semaphore(id));
                }
            }
        }
    }

    // Set Need for current task
    let current_tid = current_task().unwrap().inner_exclusive_access().res.as_ref().unwrap().tid;
    if let Some(n) = need.get_mut(&current_tid) {
        *n = Some(request);
    }

    // Banker's Algorithm (Safety Check)
    let mut work = available.clone();
    let mut finish: BTreeSet<usize> = BTreeSet::new();

    loop {
        let mut found = false;
        for &tid in &tasks {
            if !finish.contains(&tid) {
                let satisfied = match need.get(&tid).unwrap() {
                    None => true,
                    Some(res) => *work.get(res).unwrap_or(&0) >= 1,
                };

                if satisfied {
                    // Assume task finishes and releases resources
                    if let Some(alloc_vec) = allocation.get(&tid) {
                        for res in alloc_vec {
                            *work.entry(*res).or_insert(0) += 1;
                        }
                    }
                    finish.insert(tid);
                    found = true;
                }
            }
        }
        if !found {
            break;
        }
    }

    finish.len() != tasks.len()
}

/// sleep syscall
pub fn sys_sleep(ms: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_sleep",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let expire_ms = get_time_ms() + ms;
    let task = current_task().unwrap();
    add_timer(expire_ms, task);
    block_current_and_run_next();
    0
}
/// mutex create syscall
pub fn sys_mutex_create(blocking: bool) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mutex: Option<Arc<dyn Mutex>> = if !blocking {
        Some(Arc::new(MutexSpin::new()))
    } else {
        Some(Arc::new(MutexBlocking::new()))
    };
    let mut process_inner = process.inner_exclusive_access();
    if let Some(id) = process_inner
        .mutex_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.mutex_list[id] = mutex;
        id as isize
    } else {
        process_inner.mutex_list.push(mutex);
        process_inner.mutex_list.len() as isize - 1
    }
}
/// mutex lock syscall
pub fn sys_mutex_lock(mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_lock",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mutex = {
        let inner = process.inner_exclusive_access();
        if mutex_id >= inner.mutex_list.len() || inner.mutex_list[mutex_id].is_none() {
            return -1;
        }
        inner.mutex_list[mutex_id].as_ref().unwrap().clone()
    };

    if check_deadlock(&process, Resource::Mutex(mutex_id)) {
        return -0xDEAD;
    }

    mutex.lock();
    0
}
/// mutex unlock syscall
pub fn sys_mutex_unlock(mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_unlock",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());
    drop(process_inner);
    drop(process);
    mutex.unlock();
    0
}
/// semaphore create syscall
pub fn sys_semaphore_create(res_count: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let id = if let Some(id) = process_inner
        .semaphore_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.semaphore_list[id] = Some(Arc::new(Semaphore::new(res_count)));
        id
    } else {
        process_inner
            .semaphore_list
            .push(Some(Arc::new(Semaphore::new(res_count))));
        process_inner.semaphore_list.len() - 1
    };
    id as isize
}
/// semaphore up syscall
pub fn sys_semaphore_up(sem_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_up",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let sem = Arc::clone(process_inner.semaphore_list[sem_id].as_ref().unwrap());
    drop(process_inner);
    sem.up();
    0
}
/// semaphore down syscall
pub fn sys_semaphore_down(sem_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_down",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let sem = {
        let inner = process.inner_exclusive_access();
        if sem_id >= inner.semaphore_list.len() || inner.semaphore_list[sem_id].is_none() {
            return -1;
        }
        inner.semaphore_list[sem_id].as_ref().unwrap().clone()
    };

    if check_deadlock(&process, Resource::Semaphore(sem_id)) {
        return -0xDEAD;
    }

    sem.down();
    0
}
/// condvar create syscall
pub fn sys_condvar_create() -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let id = if let Some(id) = process_inner
        .condvar_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.condvar_list[id] = Some(Arc::new(Condvar::new()));
        id
    } else {
        process_inner
            .condvar_list
            .push(Some(Arc::new(Condvar::new())));
        process_inner.condvar_list.len() - 1
    };
    id as isize
}
/// condvar signal syscall
pub fn sys_condvar_signal(condvar_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_signal",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let condvar = Arc::clone(process_inner.condvar_list[condvar_id].as_ref().unwrap());
    drop(process_inner);
    condvar.signal();
    0
}
/// condvar wait syscall
pub fn sys_condvar_wait(condvar_id: usize, mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_wait",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let condvar = Arc::clone(process_inner.condvar_list[condvar_id].as_ref().unwrap());
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());
    drop(process_inner);
    condvar.wait(mutex);
    0
}
/// enable deadlock detection syscall
///
/// YOUR JOB: Implement deadlock detection, but might not all in this syscall
pub fn sys_enable_deadlock_detect(enabled: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_enable_deadlock_detect",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut inner = process.inner_exclusive_access();
    inner.deadlock_detection_enabled = enabled != 0;
    0
}
