//! Process management syscalls
//!
use crate::mm::MapPermission;
use crate::timer::get_time_us;
use crate::util::io::{read, write, SerializeToBytes};
use crate::util::mm::{ceil, mmap, unmap};
use crate::{
    fs::{open_file, OpenFlags},
    mm::{translated_ref, translated_refmut, translated_str},
    task::{
        current_process, current_task, current_user_token, exit_current_and_run_next, pid2process,
        suspend_current_and_run_next, SignalFlags,
    },
    util};
use alloc::{string::String, sync::Arc, vec::Vec};

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// exit syscall
///
/// exit the current task and run the next task in task list
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
    trace!(
        "kernel:pid[{}] sys_exit",
        current_task().unwrap().process.upgrade().unwrap().getpid()
    );
    exit_current_and_run_next(exit_code);
    panic!("Unreachable in sys_exit!");
}
/// yield syscall
pub fn sys_yield() -> isize {
    //trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}
/// getpid syscall
pub fn sys_getpid() -> isize {
    trace!(
        "kernel: sys_getpid pid:{}",
        current_task().unwrap().process.upgrade().unwrap().getpid()
    );
    current_task().unwrap().process.upgrade().unwrap().getpid() as isize
}
/// fork child process syscall
pub fn sys_fork() -> isize {
    trace!(
        "kernel:pid[{}] sys_fork",
        current_task().unwrap().process.upgrade().unwrap().getpid()
    );
    let current_process = current_process();
    let new_process = current_process.fork();
    let new_pid = new_process.getpid();
    // modify trap context of new_task, because it returns immediately after switching
    let new_process_inner = new_process.inner_exclusive_access();
    let task = new_process_inner.tasks[0].as_ref().unwrap();
    let trap_cx = task.inner_exclusive_access().get_trap_cx();
    // we do not have to move to next instruction since we have done it before
    // for child process, fork returns 0
    trap_cx.x[10] = 0;
    new_pid as isize
}

/// exec syscall
pub fn sys_exec(path: *const u8, mut args: *const usize) -> isize {
    trace!(
        "kernel:pid[{}] sys_exec",
        current_task().unwrap().process.upgrade().unwrap().getpid()
    );
    let token = current_user_token();
    let path = translated_str(token, path);
    let mut args_vec: Vec<String> = Vec::new();

    loop {

        let arg_str_ptr = *translated_ref(token, args);
        if arg_str_ptr == 0 {
            break;
        }
        args_vec.push(translated_str(token, arg_str_ptr as *const u8));
        unsafe {
            args = args.add(1);
        }
    }

    if let Some(app_inode) = open_file(path.as_str(), OpenFlags::RDONLY) {
        let all_data = app_inode.read_all();
        let process = current_process();
        let argc = args_vec.len();
        process.exec(all_data.as_slice(), args_vec);
        // return argc because cx.x[10] will be covered with it later
        argc as isize
    } else {
        -1
    }
}

/// waitpid syscall
///
/// If there is not a child process whose pid is same as given, return -1.
/// Else if there is a child process, but it is still running, return -2.
pub fn sys_waitpid(pid: isize, exit_code_ptr: *mut i32) -> isize {
    //trace!("kernel: sys_waitpid");
    let process = current_process();
    trace!(
        "kernel::pid[{}] sys_waitpid [{}]",
        process.pid.0,
        pid
    );
    let mut inner = process.inner_exclusive_access();
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
        p.inner_exclusive_access().is_zombie && (pid == -1 || pid as usize == p.getpid())
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

/// kill syscall
pub fn sys_kill(pid: usize, signal: u32) -> isize {
    trace!(
        "kernel:pid[{}] sys_kill",
        current_task().unwrap().process.upgrade().unwrap().getpid()
    );
    if let Some(process) = pid2process(pid) {
        if let Some(flag) = SignalFlags::from_bits(signal) {
            process.inner_exclusive_access().signals |= flag;
            0
        } else {
            -1
        }
    } else {
        -1
    }
}

/// get_time syscall
///
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
// pub fn sys_sbrk(size: i32) -> isize {
//     trace!("kernel:pid[{}] sys_sbrk", current_task().unwrap().process.upgrade().unwrap().getpid());
//     if let Some(old_brk) = current_task().unwrap().change_program_brk(size) {
//         old_brk as isize
//     } else {
//     -1
// }

pub fn sys_spawn(path: *const u8) -> isize {
    let current_task = current_task().unwrap();
    let process = current_task.process.upgrade().unwrap();
    trace!("kernel:pid[{}] sys_spawn", process.pid.0);
    let token = current_user_token();
    let _path = translated_str(token, path);
    0

    // if let Some(app_inode) = open_file(path.as_str(), OpenFlags::RDONLY) {
    //     let new_task = current_task.spawn(app_inode.read_all().as_slice());
    //     let new_pid = new_task.pid.0;
    //     add_task(new_task);
    //     new_pid as isize
    // } else {
    //     -1
    // }
}

pub fn sys_set_priority(prio: isize) -> isize {
    let process = current_task().unwrap().process.upgrade();
    trace!(
        "kernel:pid[{}] sys_set_priority NOT IMPLEMENTED",
        process.unwrap().pid.0
    );

    if prio < 2 {
        -1
    } else {
        prio
    }
}
