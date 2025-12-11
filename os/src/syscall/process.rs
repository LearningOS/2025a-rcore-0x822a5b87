//! Process management syscalls
//!
use alloc::sync::Arc;

use crate::mm::MapPermission;
use crate::task::get_syscall_call;
use crate::timer::get_time_us;
use crate::util;
use crate::util::io::{read, write, SerializeToBytes};
use crate::util::mm::{ceil, mmap, unmap};
use crate::{
    fs::{open_file, OpenFlags},
    mm::{translated_refmut, translated_str},
    task::{
        add_task, current_task, current_user_token, exit_current_and_run_next,
        suspend_current_and_run_next,
    },
};

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

impl SerializeToBytes for TimeVal {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MMapProtError {
    InvalidBits(usize),
}

bitflags! {
    /// map permission flags for mmap
    pub struct MMapProt: u8 {
        /// Readable
        const R = 1 << 0;
        /// Writable
        const W = 1 << 1;
        /// Executable
        const X = 1 << 2;
    }
}

impl TryInto<MapPermission> for MMapProt {
    type Error = MMapProtError;

    fn try_into(self) -> Result<MapPermission, Self::Error> {
        let mut perm = MapPermission::empty();
        if self.contains(MMapProt::R) {
            perm |= MapPermission::R;
        }
        if self.contains(MMapProt::W) {
            perm |= MapPermission::W;
        }
        if self.contains(MMapProt::X) {
            perm |= MapPermission::X;
        }

        perm |= MapPermission::U;

        Ok(perm)
    }
}

/// try to parse MMapProt from usize
impl TryFrom<usize> for MMapProt {
    type Error = MMapProtError;

    /// implementation of try_from
    fn try_from(val: usize) -> Result<Self, Self::Error> {
        if val == 0 {
            return Err(MMapProtError::InvalidBits(val));
        }

        const MASK: usize = 0x7;
        if (val & !MASK) != 0 {
            return Err(MMapProtError::InvalidBits(val));
        }

        Ok(Self {
            bits: (val & MASK) as u8,
        })
    }
}

/// task exits and submit an exit code
pub fn sys_exit(exit_code: i32) -> ! {
    trace!("kernel:pid[{}] sys_exit", current_task().unwrap().pid.0);
    exit_current_and_run_next(exit_code);
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel:pid[{}] sys_yield", current_task().unwrap().pid.0);
    suspend_current_and_run_next();
    0
}

pub fn sys_getpid() -> isize {
    trace!("kernel: sys_getpid pid:{}", current_task().unwrap().pid.0);
    current_task().unwrap().pid.0 as isize
}

pub fn sys_fork() -> isize {
    trace!("kernel:pid[{}] sys_fork", current_task().unwrap().pid.0);
    let current_task = current_task().unwrap();
    let new_task = current_task.fork();
    let new_pid = new_task.pid.0;
    // modify trap context of new_task, because it returns immediately after switching
    let trap_cx = new_task.inner_exclusive_access().get_trap_cx();
    // we do not have to move to next instruction since we have done it before
    // for child process, fork returns 0
    trap_cx.x[10] = 0;
    // add new task to scheduler
    add_task(new_task);
    new_pid as isize
}

pub fn sys_exec(path: *const u8) -> isize {
    trace!("kernel:pid[{}] sys_exec", current_task().unwrap().pid.0);
    let token = current_user_token();
    let path = translated_str(token, path);
    if let Some(app_inode) = open_file(path.as_str(), OpenFlags::RDONLY) {
        let all_data = app_inode.read_all();
        let task = current_task().unwrap();
        task.exec(all_data.as_slice());
        0
    } else {
        -1
    }
}

/// If there is not a child process whose pid is same as given, return -1.
/// Else if there is a child process, but it is still running, return -2.
pub fn sys_waitpid(pid: isize, exit_code_ptr: *mut i32) -> isize {
    trace!(
        "kernel::pid[{}] sys_waitpid [{}]",
        current_task().unwrap().pid.0,
        pid
    );
    let task = current_task().unwrap();
    // find a child process

    // ---- access current PCB exclusively
    let mut inner = task.inner_exclusive_access();
    if !inner
        .children
        .iter()
        .any(|p| pid == -1 || pid as usize == p.getpid())
    {
        return -1;
        // ---- release current PCB
    }
    let pair = inner.children.iter().enumerate().find(|(_, p)| {
        // ++++ temporarily access child PCB exclusively
        p.inner_exclusive_access().is_zombie() && (pid == -1 || pid as usize == p.getpid())
        // ++++ release child PCB
    });
    if let Some((idx, _)) = pair {
        let child = inner.children.remove(idx);
        // confirm that child will be deallocated after being removed from children list
        assert_eq!(Arc::strong_count(&child), 1);
        let found_pid = child.getpid();
        // ++++ temporarily access child PCB exclusively
        let exit_code = child.inner_exclusive_access().exit_code;
        // ++++ release child PCB
        *translated_refmut(inner.memory_set.token(), exit_code_ptr) = exit_code;
        found_pid as isize
    } else {
        -2
    }
    // ---- release current PCB automatically
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is split by two pages ?
pub fn sys_get_time(_ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel: sys_get_time");
    let us = get_time_us();
    let pa = util::mm::translate_va_to_pa(
        current_user_token(),
        _ts as *const u8,
        core::mem::size_of::<TimeVal>(),
    );
    let t = &TimeVal {
        sec: us / 1_000_000,
        usec: us % 1_000_000,
    };
    let _ = util::io::serialize_struct(t, pa);
    0
}


#[allow(unused)]
pub fn sys_trace(trace_request: usize, id: usize, data: usize) -> isize {
    let len = core::mem::size_of::<u8>();
    let data = data as u8;
    match trace_request {
        0 => {
            let r: Result<u8, &str> = read(current_user_token(), id as *const u8, len);
            match r {
                Ok(v) => v as isize,
                _ => -1,
            }
        }
        1 => {
            let r = write(&data, current_user_token(), id as *const u8, len);
            match r {
                Ok(_) => 0,
                _ => -1,
            }
        }
        2 => get_syscall_call(id as u8),
        _ => {
            trace!(
                "kernel: sys_trace with unknown trace_request {}",
                trace_request
            );
            -1
        }
    }
}

/// `sys_mmap` will allocate a range of physical pages and map them to a given virtual address range.
pub fn sys_mmap(start: usize, len: usize, prot: usize) -> isize {
    let len = ceil(len);

    let prot_result = MMapProt::try_from(prot);
    if prot_result.is_err() {
        trace!("[kernel]: sys_mmap with malformed prot {}", prot);
        return -1;
    }
    let prot = prot_result.unwrap();
    let map_perm: Result<MapPermission, MMapProtError> = prot.try_into();
    if map_perm.is_err() {
        trace!("[kernel]: sys_mmap with invalid prot bits {}", prot.bits);
        return -1;
    }

    let map_perm = map_perm.unwrap();

    let ret = mmap(start as *const u8, len, map_perm);
    match ret {
        Ok(_) => 0,
        Err(e) => {
            trace!("[kernel]: sys_mmap failed: {}", e);
            -1
        }
    }
}

pub fn sys_munmap(start: usize, len: usize) -> isize {
    let len = ceil(len);
    let ret = unmap(start as *const u8, len);
    match ret {
        Ok(_) => 0,
        Err(e) => {
            trace!("[kernel]: sys_munmap failed: {}", e);
            -1
        }
    }
}

/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel:pid[{}] sys_sbrk", current_task().unwrap().pid.0);
    if let Some(old_brk) = current_task().unwrap().change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}

pub fn sys_spawn(path: *const u8) -> isize {
    let current_task = current_task().unwrap();
    trace!("kernel:pid[{}] sys_spawn", current_task.pid.0);
    let token = current_user_token();
    let path = translated_str(token, path);

    if let Some(app_inode) = open_file(path.as_str(), OpenFlags::RDONLY) {
        let new_task = current_task.spawn(app_inode.read_all().as_slice());
        let new_pid = new_task.pid.0;
        add_task(new_task);
        new_pid as isize
    } else {
        -1
    }
}

pub fn sys_set_priority(prio: isize) -> isize {
    trace!(
        "kernel:pid[{}] sys_set_priority NOT IMPLEMENTED",
        current_task().unwrap().pid.0
    );

    if prio < 2 {
        -1
    } else {
        let task = current_task().unwrap();
        task.inner_exclusive_access().prio = prio as usize;
        prio
    }
}
