//! Implementation of  [`ProcessControlBlock`]

use super::id::RecycleAllocator;
use super::manager::insert_into_pid2process;
use super::TaskControlBlock;
use super::{add_task, SignalFlags};
use super::{pid_alloc, PidHandle};
use crate::config::{DEADLOCK_SIGNAL, MAX_LOCK, MAX_THREAD, WAIT_SIGNAL};
use crate::console::print;
use crate::fs::{File, Stdin, Stdout};
use crate::mm::{translated_refmut, MemorySet, KERNEL_SPACE};
use crate::sync::{Condvar, Mutex, MutexBlocking, MutexSpin, Semaphore, UPSafeCell};
use crate::trap::{trap_handler, TrapContext};
use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::{Arc, Weak};
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefMut;
use log::__private_api::loc;
use riscv::register::mstatus::set_fs;

/// Process Control Block
pub struct ProcessControlBlock {
    /// immutable
    pub pid: PidHandle,
    /// mutable
    inner: UPSafeCell<ProcessControlBlockInner>,
}

/// Inner of Process Control Block
pub struct ProcessControlBlockInner {
    /// is zombie?
    pub is_zombie: bool,
    /// memory set(address space)
    pub memory_set: MemorySet,
    /// parent process
    pub parent: Option<Weak<ProcessControlBlock>>,
    /// children process
    pub children: Vec<Arc<ProcessControlBlock>>,
    /// exit code
    pub exit_code: i32,
    /// file descriptor table
    pub fd_table: Vec<Option<Arc<dyn File + Send + Sync>>>,
    /// signal flags
    pub signals: SignalFlags,
    /// tasks(also known as threads)
    pub tasks: Vec<Option<Arc<TaskControlBlock>>>,
    /// task resource allocator
    pub task_res_allocator: RecycleAllocator,
    /// mutex list
    pub mutex_list: Vec<Option<Arc<dyn Mutex>>>,
    /// semaphore list
    pub semaphore_list: Vec<Option<Arc<Semaphore>>>,
    /// condvar list
    pub condvar_list: Vec<Option<Arc<Condvar>>>,
    /// Used for deadlock detect
    pub deadlock_detector: DeadlockDetector,
    /// enable deadlock
    pub enable_deadlock_detector: bool,
}

pub struct DeadlockDetector {
    inner: DeadlockDetectorInner,
}

pub struct DeadlockDetectorInner {
    allocator: RecycleAllocator,
    available: [usize; MAX_LOCK],
    cond_vars: Vec<Condvar>,
    locker: Vec<Arc<MutexBlocking>>,
    allocation: [[usize; MAX_LOCK]; MAX_THREAD],
    need: [[usize; MAX_LOCK]; MAX_THREAD],
}

impl ProcessControlBlockInner {
    #[allow(unused)]
    /// get the address of app's page table
    pub fn get_user_token(&self) -> usize {
        self.memory_set.token()
    }
    /// allocate a new file descriptor
    pub fn alloc_fd(&mut self) -> usize {
        if let Some(fd) = (0..self.fd_table.len()).find(|fd| self.fd_table[*fd].is_none()) {
            fd
        } else {
            self.fd_table.push(None);
            self.fd_table.len() - 1
        }
    }
    /// allocate a new task id
    pub fn alloc_tid(&mut self) -> usize {
        self.task_res_allocator.alloc()
    }
    /// deallocate a task id
    pub fn dealloc_tid(&mut self, tid: usize) {
        self.task_res_allocator.dealloc(tid)
    }
    /// the count of tasks(threads) in this process
    pub fn thread_count(&self) -> usize {
        self.tasks.len()
    }
    /// get a task with tid in this process
    pub fn get_task(&self, tid: usize) -> Arc<TaskControlBlock> {
        self.tasks[tid].as_ref().unwrap().clone()
    }

    /// enable_detect_deadlock
    pub fn enable_detect_deadlock(&mut self) {
        self.enable_deadlock_detector = true
    }

    /// create_mutex
    pub fn create_mutex(&mut self, tid: usize, blocking: bool) -> isize {
        let mutex_id = self.deadlock_detector.alloc_id(tid, 1);
        let mutex: Option<Arc<dyn Mutex>> = if !blocking {
            Some(Arc::new(MutexSpin::new(mutex_id)))
        } else {
            Some(Arc::new(MutexBlocking::new(mutex_id)))
        };
        if let Some(id) = self
            .mutex_list
            .iter()
            .enumerate()
            .find(|(_, item)| item.is_none())
            .map(|(id, _)| id)
        {
            self.mutex_list[id] = mutex;
            id as isize
        } else {
            self.mutex_list.push(mutex);
            self.mutex_list.len() as isize - 1
        }
    }

    /// mutex_lock get mutex lock
    pub fn mutex_lock(&self, mutex_id: usize) -> Arc<dyn Mutex> {
        let mutex = self.mutex_list[mutex_id].as_ref().unwrap();
        let mutex = Arc::clone(mutex);
        mutex
    }

    /// create_semaphore
    pub fn create_semaphore(&mut self, tid: usize, res_count: usize) -> usize {
        let mutex_id = self.deadlock_detector.alloc_id(tid, res_count);
        let id = if let Some(id) = self
            .semaphore_list
            .iter()
            .enumerate()
            .find(|(_, item)| item.is_none())
            .map(|(id, _)| id)
        {
            self.semaphore_list[id] = Some(Arc::new(Semaphore::new(mutex_id, res_count)));
            id
        } else {
            self.semaphore_list
                .push(Some(Arc::new(Semaphore::new(mutex_id, res_count))));
            self.semaphore_list.len() - 1
        };
        id
    }

    pub fn semaphore(&self, sem_id: usize) -> Arc<Semaphore> {
        Arc::clone(self.semaphore_list[sem_id].as_ref().unwrap())
    }
}

impl ProcessControlBlock {
    /// inner_exclusive_access
    pub fn inner_exclusive_access(&self) -> RefMut<'_, ProcessControlBlockInner> {
        self.inner.exclusive_access()
    }
    /// new process from elf file
    pub fn new(elf_data: &[u8]) -> Arc<Self> {
        trace!("kernel: ProcessControlBlock::new");
        // memory_set with elf program headers/trampoline/trap context/user stack
        let (memory_set, ustack_base, entry_point) = MemorySet::from_elf(elf_data);
        // allocate a pid
        let pid_handle = pid_alloc();
        let process = Arc::new(Self {
            pid: pid_handle,
            inner: unsafe {
                UPSafeCell::new(ProcessControlBlockInner {
                    is_zombie: false,
                    memory_set,
                    parent: None,
                    children: Vec::new(),
                    exit_code: 0,
                    fd_table: vec![
                        // 0 -> stdin
                        Some(Arc::new(Stdin)),
                        // 1 -> stdout
                        Some(Arc::new(Stdout)),
                        // 2 -> stderr
                        Some(Arc::new(Stdout)),
                    ],
                    signals: SignalFlags::empty(),
                    tasks: Vec::new(),
                    task_res_allocator: RecycleAllocator::new(),
                    mutex_list: Vec::new(),
                    semaphore_list: Vec::new(),
                    condvar_list: Vec::new(),
                    deadlock_detector: DeadlockDetector::new(),
                    enable_deadlock_detector: false,
                })
            },
        });
        // create a main thread, we should allocate ustack and trap_cx here
        let task = Arc::new(TaskControlBlock::new(
            Arc::clone(&process),
            ustack_base,
            true,
        ));
        // prepare trap_cx of main thread
        let task_inner = task.inner_exclusive_access();
        let trap_cx = task_inner.get_trap_cx();
        let ustack_top = task_inner.res.as_ref().unwrap().ustack_top();
        let kstack_top = task.kstack.get_top();
        drop(task_inner);
        *trap_cx = TrapContext::app_init_context(
            entry_point,
            ustack_top,
            KERNEL_SPACE.exclusive_access().token(),
            kstack_top,
            trap_handler as usize,
        );
        // add main thread to the process
        let mut process_inner = process.inner_exclusive_access();
        process_inner.tasks.push(Some(Arc::clone(&task)));
        drop(process_inner);
        insert_into_pid2process(process.getpid(), Arc::clone(&process));
        // add main thread to scheduler
        add_task(task);
        process
    }

    /// Only support processes with a single thread.
    pub fn exec(self: &Arc<Self>, elf_data: &[u8], args: Vec<String>) {
        trace!("kernel: exec");
        assert_eq!(self.inner_exclusive_access().thread_count(), 1);
        // memory_set with elf program headers/trampoline/trap context/user stack
        trace!("kernel: exec .. MemorySet::from_elf");
        let (memory_set, ustack_base, entry_point) = MemorySet::from_elf(elf_data);
        let new_token = memory_set.token();
        // substitute memory_set
        trace!("kernel: exec .. substitute memory_set");
        self.inner_exclusive_access().memory_set = memory_set;
        // then we alloc user resource for main thread again
        // since memory_set has been changed
        trace!("kernel: exec .. alloc user resource for main thread again");
        let task = self.inner_exclusive_access().get_task(0);
        let mut task_inner = task.inner_exclusive_access();
        task_inner.res.as_mut().unwrap().ustack_base = ustack_base;
        task_inner.res.as_mut().unwrap().alloc_user_res();
        task_inner.trap_cx_ppn = task_inner.res.as_mut().unwrap().trap_cx_ppn();
        // push arguments on user stack
        trace!("kernel: exec .. push arguments on user stack");
        let mut user_sp = task_inner.res.as_mut().unwrap().ustack_top();
        user_sp -= (args.len() + 1) * core::mem::size_of::<usize>();
        let argv_base = user_sp;
        let mut argv: Vec<_> = (0..=args.len())
            .map(|arg| {
                translated_refmut(
                    new_token,
                    (argv_base + arg * core::mem::size_of::<usize>()) as *mut usize,
                )
            })
            .collect();
        *argv[args.len()] = 0;
        for i in 0..args.len() {
            user_sp -= args[i].len() + 1;
            *argv[i] = user_sp;
            let mut p = user_sp;
            for c in args[i].as_bytes() {
                *translated_refmut(new_token, p as *mut u8) = *c;
                p += 1;
            }
            *translated_refmut(new_token, p as *mut u8) = 0;
        }
        // make the user_sp aligned to 8B for k210 platform
        user_sp -= user_sp % core::mem::size_of::<usize>();
        // initialize trap_cx
        trace!("kernel: exec .. initialize trap_cx");
        let mut trap_cx = TrapContext::app_init_context(
            entry_point,
            user_sp,
            KERNEL_SPACE.exclusive_access().token(),
            task.kstack.get_top(),
            trap_handler as usize,
        );
        trap_cx.x[10] = args.len();
        trap_cx.x[11] = argv_base;
        *task_inner.get_trap_cx() = trap_cx;
    }

    /// Only support processes with a single thread.
    pub fn fork(self: &Arc<Self>) -> Arc<Self> {
        trace!("kernel: fork");
        let mut parent = self.inner_exclusive_access();
        assert_eq!(parent.thread_count(), 1);
        // clone parent's memory_set completely including trampoline/ustacks/trap_cxs
        let memory_set = MemorySet::from_existed_user(&parent.memory_set);
        // alloc a pid
        let pid = pid_alloc();
        // copy fd table
        let mut new_fd_table: Vec<Option<Arc<dyn File + Send + Sync>>> = Vec::new();
        for fd in parent.fd_table.iter() {
            if let Some(file) = fd {
                new_fd_table.push(Some(file.clone()));
            } else {
                new_fd_table.push(None);
            }
        }
        // create child process pcb
        let child = Arc::new(Self {
            pid,
            inner: unsafe {
                UPSafeCell::new(ProcessControlBlockInner {
                    is_zombie: false,
                    memory_set,
                    parent: Some(Arc::downgrade(self)),
                    children: Vec::new(),
                    exit_code: 0,
                    fd_table: new_fd_table,
                    signals: SignalFlags::empty(),
                    tasks: Vec::new(),
                    task_res_allocator: RecycleAllocator::new(),
                    mutex_list: Vec::new(),
                    semaphore_list: Vec::new(),
                    condvar_list: Vec::new(),
                    deadlock_detector: DeadlockDetector::new(),
                    enable_deadlock_detector: false,
                })
            },
        });
        // add child
        parent.children.push(Arc::clone(&child));
        // create main thread of child process
        let task = Arc::new(TaskControlBlock::new(
            Arc::clone(&child),
            parent
                .get_task(0)
                .inner_exclusive_access()
                .res
                .as_ref()
                .unwrap()
                .ustack_base(),
            // here we do not allocate trap_cx or ustack again
            // but mention that we allocate a new kstack here
            false,
        ));
        // attach task to child process
        let mut child_inner = child.inner_exclusive_access();
        child_inner.tasks.push(Some(Arc::clone(&task)));
        drop(child_inner);
        // modify kstack_top in trap_cx of this thread
        let task_inner = task.inner_exclusive_access();
        let trap_cx = task_inner.get_trap_cx();
        trap_cx.kernel_sp = task.kstack.get_top();
        drop(task_inner);
        insert_into_pid2process(child.getpid(), Arc::clone(&child));
        // add this thread to scheduler
        add_task(task);
        child
    }
    /// get pid
    pub fn getpid(&self) -> usize {
        self.pid.0
    }
}

impl DeadlockDetector {
    pub unsafe fn new() -> DeadlockDetector {
        let mut allocator = RecycleAllocator::new();
        let inner_lock_id = allocator.alloc();
        let mut cond_vars = vec![];
        let mut lockers = vec![];
        for i in 0..MAX_LOCK {
            cond_vars.push(Condvar::new());
            lockers.push(Arc::new(MutexBlocking::new(i)));
        }

        let detector = DeadlockDetector {
            inner: DeadlockDetectorInner {
                allocator,
                available: [0; MAX_THREAD],
                cond_vars,
                locker: lockers,
                allocation: [[0; MAX_LOCK]; MAX_THREAD],
                need: [[0; MAX_LOCK]; MAX_THREAD],
            },
        };
        detector
    }

    pub fn push_thread_available(&mut self, _tid: usize) {
        // {
        //     // TODO delete me
        //     println!("push thread tid [{}]", tid);
        // }
        //
        // self.lock();
        // let mut inner = self.inner.exclusive_access();
        // for _ in inner.allocation.len()..=tid {
        //     inner.allocation.push(vec![]);
        //     inner.need.push(vec![]);
        //     {
        //         // TODO delete me
        //         println!(
        //             "push thread [{}], allocation len = [{}], need len = [{}]",
        //             tid,
        //             inner.allocation.len(),
        //             inner.need.len()
        //         );
        //     }
        // }
        // self.unlock();
    }

    fn push_lock_available(&mut self, tid: usize, id: usize, count: usize) {
        self.inner.available[id] = count;
        self.inner.allocation[tid][id] = 0;
        self.inner.need[tid][id] = 0;
    }

    // /// detect deadlock, return -0xdead if deadlock is detected
    pub fn detect_deadlock(&mut self, tid: usize, id: usize, retry:bool) -> i32 {
        // self.lock();
        let inner = &mut self.inner;

        let finishes = &mut [false; MAX_THREAD];
        let mut work = inner.available.clone();
        let local_allocation = inner.allocation.clone();

        if !retry {
            inner.add_need(tid, id);
        }
        let local_need = inner.need.clone();

        let mut run = true;
        while run {
            run = false;
            for (thread_id, finish) in finishes.iter_mut().enumerate() {
                if *finish {
                    continue;
                }
                let thread_need = &local_need[thread_id];
                let mut all_need_satisfied = true;
                for (mutex_id, &need) in thread_need.iter().enumerate() {
                    if need > work[mutex_id] {
                        all_need_satisfied = false;
                        break;
                    }
                }
                if !all_need_satisfied {
                    continue;
                }

                run = true;

                let thread_allocation = local_allocation[thread_id];
                for (mutex_id, available) in work.iter_mut().enumerate() {
                    *available += thread_allocation[mutex_id];
                }
                *finish = true;
            }
        }
        let mut deadlock = 0;
        for (_, finish) in finishes.iter().enumerate() {
            if !*finish {
                deadlock = DEADLOCK_SIGNAL;
                break;
            }
        }

        if deadlock == DEADLOCK_SIGNAL {
            return deadlock;
        }

        if inner.available[id] < 1 {
            return WAIT_SIGNAL;
        }

        if deadlock != DEADLOCK_SIGNAL {
            inner.add_allocation(tid, id);
            inner.sub_need(tid, id);
        }

        // self.unlock();
        deadlock
    }

    #[allow(unused)]
    pub fn release(&mut self, tid: usize, id: usize) {
        // self.lock();
        let mut inner = &mut self.inner;
        inner.sub_allocation(tid, id);
        // self.unlock();
    }

    pub fn alloc_id(&mut self, tid: usize, count: usize) -> usize {
        // self.lock();
        let inner = &mut self.inner;
        let id = inner.allocator.alloc();
        self.push_lock_available(tid, id, count);
        // self.unlock();
        id
    }

    // fn lock(&self) {
    //     self.locker.lock();
    // }
    //
    // fn unlock(&self) {
    //     self.locker.unlock();
    // }
}

impl DeadlockDetectorInner {
    fn add_allocation(&mut self, tid: usize, id: usize) {
        let thread_alloc = &mut self.allocation[tid];
        thread_alloc[id] += 1;
        self.available[id] -= 1;
    }

    pub fn sub_allocation(&mut self, tid: usize, id: usize) {
        let thread_alloc = &mut self.allocation[tid];
        thread_alloc[id] -= 1;
        self.available[id] += 1;
        self.cond_vars[id].signal();
    }

    #[allow(unused)]
    pub fn add_need(&mut self, tid: usize, id: usize) {
        let thread_alloc = &mut self.need[tid];
        thread_alloc[id] += 1;
    }

    #[allow(unused)]
    fn sub_need(&mut self, tid: usize, id: usize) {
        let thread_alloc = &mut self.need[tid];
        thread_alloc[id] -= 1;
    }
}
