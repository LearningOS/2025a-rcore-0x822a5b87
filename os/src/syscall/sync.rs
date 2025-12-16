use crate::config::{DEADLOCK_SIGNAL, WAIT_SIGNAL};
use crate::sync::Condvar;
use crate::task::{block_current_and_run_next, current_process, current_task, suspend_current_and_run_next};
use crate::timer::{add_timer, get_time_ms};
use alloc::sync::Arc;
use crate::syscall::sys_trace_context;

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
    let (tid, _) = sys_trace_context();
    let process = current_process();
    let mut inner = process.inner_exclusive_access();
    let mutex = inner.create_mutex(tid, blocking);
    mutex
}
/// mutex lock syscall
pub fn sys_mutex_lock(mutex_id: usize) -> isize {
    let (tid, _) = sys_trace_context();
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let mutex = process_inner.mutex_lock(mutex_id);

    let id = mutex.id();
    let deadlock = process_inner.deadlock_detector.detect_deadlock(tid, id, false);
    drop(process_inner);
    drop(process);
    if deadlock != DEADLOCK_SIGNAL {
        mutex.lock();
        0
    } else {
        DEADLOCK_SIGNAL as isize
    }
}
/// mutex unlock syscall
pub fn sys_mutex_unlock(mutex_id: usize) -> isize {
    let (tid, _) = sys_trace_context();
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let mutex = process_inner.mutex_lock(mutex_id);
    process_inner.deadlock_detector.release(tid, mutex.id());
    drop(process_inner);
    drop(process);
    mutex.unlock();
    0
}
/// semaphore create syscall
pub fn sys_semaphore_create(res_count: usize) -> isize {
    let (tid, _) = sys_trace_context();
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let id = process_inner.create_semaphore(tid, res_count);
    id as isize
}
/// semaphore up syscall
pub fn sys_semaphore_up(sem_id: usize) -> isize {
    let (tid, _) = sys_trace_context();
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let sem = process_inner.semaphore(sem_id);
    process_inner.deadlock_detector.release(tid, sem.id());
    drop(process_inner);
    sem.up();
    0
}

/// semaphore down syscall
pub fn sys_semaphore_down(sem_id: usize) -> isize {
    let (tid, _) = sys_trace_context();
    let mut retry = false;
    loop {
        let process = current_process();
        let mut process_inner = process.inner_exclusive_access();
        let sem = process_inner.semaphore(sem_id);
        let deadlock = process_inner.deadlock_detector.detect_deadlock(tid, sem.id(), retry);
        retry = true;
        drop(process_inner);
        if deadlock == WAIT_SIGNAL {
            suspend_current_and_run_next();
            continue
        }
        return if deadlock != DEADLOCK_SIGNAL {
            sem.down();
            0
        } else {
            DEADLOCK_SIGNAL as isize
        }
    }
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
pub fn sys_enable_deadlock_detect(_enabled: usize) -> isize {
    let task = current_task().unwrap();
    let process = current_process();
    trace!("kernel: pid-[{}] tid-[{}] sys_enable_deadlock_detect", process.getpid(), task.get_tid().unwrap());
    let mut inner = process.inner_exclusive_access();
    inner.enable_detect_deadlock();
    0
}
