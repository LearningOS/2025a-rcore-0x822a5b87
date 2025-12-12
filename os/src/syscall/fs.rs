//! File and filesystem-related syscalls

use crate::fs::{fstat, linkat, open_file, unlinkat, OpenFlags, Stat, StatMode, AT_FDCWD};
use crate::mm::{translated_byte_buffer, translated_refmut, translated_str, UserBuffer};
use crate::task::{current_task, current_user_token};

pub fn sys_write(fd: usize, buf: *const u8, len: usize) -> isize {
    trace!("kernel:pid[{}] sys_write", current_task().unwrap().pid.0);
    let token = current_user_token();
    let task = current_task().unwrap();
    let inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    if let Some(file) = &inner.fd_table[fd] {
        if !file.writable() {
            return -1;
        }
        let file = file.clone();
        // release current task TCB manually to avoid multi-borrow
        drop(inner);
        file.write(UserBuffer::new(translated_byte_buffer(token, buf, len))) as isize
    } else {
        -1
    }
}

pub fn sys_read(fd: usize, buf: *const u8, len: usize) -> isize {
    trace!("kernel:pid[{}] sys_read", current_task().unwrap().pid.0);
    let token = current_user_token();
    let task = current_task().unwrap();
    let inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    if let Some(file) = &inner.fd_table[fd] {
        let file = file.clone();
        if !file.readable() {
            return -1;
        }
        // release current task TCB manually to avoid multi-borrow
        drop(inner);
        trace!("kernel: sys_read .. file.read");
        file.read(UserBuffer::new(translated_byte_buffer(token, buf, len))) as isize
    } else {
        -1
    }
}

pub fn sys_open(path: *const u8, flags: u32) -> isize {
    trace!("kernel:pid[{}] sys_open", current_task().unwrap().pid.0);
    let task = current_task().unwrap();
    let token = current_user_token();
    let path = translated_str(token, path);
    if let Some(inode) = open_file(path.as_str(), OpenFlags::from_bits(flags).unwrap()) {
        let mut inner = task.inner_exclusive_access();
        let fd = inner.alloc_fd();
        inner.fd_table[fd] = Some(inode);
        fd as isize
    } else {
        -1
    }
}

pub fn sys_close(fd: usize) -> isize {
    trace!("kernel:pid[{}] sys_close", current_task().unwrap().pid.0);
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    if inner.fd_table[fd].is_none() {
        return -1;
    }
    inner.fd_table[fd].take();
    0
}

pub fn sys_fstat(fd: usize, st: *mut Stat) -> isize {
    let task = current_task().unwrap();
    trace!("kernel:pid[{}] sys_fstat", task.pid.0);
    let inner = task.inner_exclusive_access();
    let fd_item = inner.fd_table.get(fd).and_then(|opt | opt.clone());
    drop(inner);
    if fd_item.is_none() {
        return -1
    }

    let stat = fstat(&fd_item.unwrap());
    let st = translated_refmut(current_user_token(), st);
    if stat.is_none() {
        st.mode = StatMode::NULL;
    } else {
        let stat = stat.unwrap();
        st.ino = stat.inode_id as u64;
        st.nlink = stat.ref_count;
        if stat.is_dir {
            st.mode = StatMode::DIR;
        } else {
            st.mode = StatMode::FILE;
        }
    }
    0
}

pub fn sys_linkat(old_name: *const u8, new_name: *const u8) -> isize {
    let pid = current_task().unwrap().pid.0;
    trace!("kernel:pid[{}] sys_linkat", pid);
    linkat(AT_FDCWD, old_name, AT_FDCWD, new_name, 0) as isize
}

pub fn sys_unlinkat(name: *const u8) -> isize {
    let pid = current_task().unwrap().pid.0;
    trace!("kernel:pid[{}] sys_unlinkat", pid);
    unlinkat(name) as isize
}
