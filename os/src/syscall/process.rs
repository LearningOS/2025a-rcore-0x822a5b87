//! Process management syscalls
use crate::{
    task::{exit_current_and_run_next, suspend_current_and_run_next},
    timer::get_time_us,
};
use crate::task::get_trace_stat;

#[repr(C)]
pub enum TraceRequest {
    Read,
    Write,
    Syscall,
}

impl TraceRequest {
    pub fn from_usize(n: usize) -> Option<Self> {
        match n {
            0 => Some(Self::Read),
            1 => Some(Self::Write),
            2 => Some(Self::Syscall),
            _ => None,
        }
    }
}

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// task exits and submit an exit code
pub fn sys_exit(exit_code: i32) -> ! {
    trace!("[kernel] Application exited with code {}", exit_code);
    exit_current_and_run_next();
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

/// get time with second and microsecond
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel: sys_get_time");
    let us = get_time_us();
    unsafe {
        *ts = TimeVal {
            sec: us / 1_000_000,
            usec: us % 1_000_000,
        };
    }
    0
}

/// If `_trace_request` equals 0, `_id` should be treated as a `*const u8` (constant pointer to `u8`).
/// We read the value pointed to by `_id` and return it.
///
/// If `_trace_request` equals 1, `_id` should be treated as a `*mut u8` (mutable pointer to `u8`).
/// We write `_data` to the address `_id` and return 0.
///
/// If `_trace_request` equals 2, return the total number of times `sys_trace` has been called by the current task,
/// including the current call.
pub fn sys_trace(_trace_request: usize, _id: usize, _data: usize) -> isize {
    match TraceRequest::from_usize(_trace_request) {
        Some(TraceRequest::Read) => {
            let x = _id as *const u8;
            unsafe {
                (*x) as isize
            }
        },
        Some(TraceRequest::Write) => {
            let x = _id as *mut u8;
            unsafe {
                *x = (_data & 0xFF) as u8;
                0
            }
        },
        Some(TraceRequest::Syscall) => {
            get_trace_stat(_id as u8)
        },
        _ => {
            panic!("Not illegal _trace_request : {}", _trace_request)
        }
    }
}
