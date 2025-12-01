//! Process management syscalls

use crate::mm::{MapPermission};
use crate::task::{
    change_program_brk, current_user_token, exit_current_and_run_next, get_syscall_call,
    suspend_current_and_run_next,
};
use crate::timer::get_time_us;
use crate::util;
use crate::util::io::{read, write, SerializeToBytes};
use crate::util::mm::{ceil, mmap, unmap};

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
pub fn sys_exit(_exit_code: i32) -> ! {
    trace!("kernel: sys_exit");
    exit_current_and_run_next();
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

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
    let map_perm : Result<MapPermission, MMapProtError> = prot.try_into();
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
        },
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
        },
    }}
/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel: sys_sbrk");
    if let Some(old_brk) = change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}
