//! utilities for memory management

use crate::mm::{
    MapPermission, PTEFlags, PageTable, PageTableEntry, StepByOne, VirtAddr, VirtPageNum,
};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use crate::config::PAGE_SIZE;

pub fn mmap(start: *const u8, len: usize, map_perm: MapPermission) -> Result<(), String> {
    let len = VirtAddr::from(len);
    if !len.aligned() {
        let e = format!("[kernel]: sys_mmap with unaligned length {}", len.0);
        return Err(e);
    }
    let result = VirtAddr::new_aligned_va(start as usize);
    if result.is_err() {
        let e = format!("[kernel]: sys_mmap with unaligned start {}", start as usize);
        return Err(e);
    }

    let start_va: VirtAddr = result?;
    let end_va: VirtAddr = start_va + len;

    crate::task::mmap(start_va, end_va, map_perm)
}

pub fn unmap(start: *const u8, len: usize) -> Result<(), String> {
    let len = VirtAddr::from(len);
    if !len.aligned() {
        let e = format!("[kernel]: unmap with unaligned length {}", len.0);
        return Err(e);
    }
    let result = VirtAddr::new_aligned_va(start as usize);
    if result.is_err() {
        let e = format!("[kernel]: sys_mmap with unaligned start {}", start as usize);
        return Err(e);
    }

    let start_va = result?;
    let end_va = start_va + len;

    crate::task::munmap(start_va, end_va, MapPermission::empty())
}

/// ceil the address to page boundary
pub fn ceil(v: usize) -> usize {
    let va = VirtAddr::from(v);
    va.ceil().0 * PAGE_SIZE
}

/// floor the address to page boundary
#[allow(dead_code)]
pub fn floor(v: usize) -> usize {
    let va = VirtAddr::from(v);
    va.floor().0 * PAGE_SIZE
}

/// Translate a virtual address to a physical address through page table
pub fn translate_va_to_pa(token: usize, ptr: *const u8, len: usize) -> Vec<&'static mut [u8]> {
    let mut v = Vec::new();
    let page_table = PageTable::from_token(token);
    let mut start = ptr as usize;
    let end = start + len;

    while start < end {
        let start_va = VirtAddr::from(start);
        let mut vpn = VirtPageNum::from(start_va.floor());
        let ppn = page_table.translate(vpn);
        if ppn.is_none() {
            error!("error translating va to pa {:x}, vpn = {:x}", start, vpn.0);
            return Vec::new();
        }
        let ppn = ppn.unwrap().ppn();
        vpn.step();
        let end_va = VirtAddr::from(end.min(VirtAddr::from(vpn).into()));
        if end_va.aligned() {
            v.push(&mut ppn.get_bytes_array()[start_va.page_offset()..]);
        } else {
            v.push(&mut ppn.get_bytes_array()[start_va.page_offset()..end_va.page_offset()]);
        }
        start = end_va.into();
    }
    v
}

/// Check whether a virtual address range has the required permissions
pub fn auth_check(token: usize, ptr: *const u8, len: usize, auth_flags: PTEFlags) -> bool {
    let pte_list = translate_entries(token, ptr, len);

    if pte_list.is_empty() {
        return false;
    }

    for pte in pte_list {
        if pte.flags() & auth_flags != auth_flags {
            return false;
        }
    }
    true
}

/// Get all entries of a virtual address range
pub fn translate_entries(token: usize, ptr: *const u8, len: usize) -> Vec<PageTableEntry> {
    let mut v = Vec::new();
    let page_table = PageTable::from_token(token);
    let mut start = ptr as usize;
    let end = start + len;

    while start < end {
        let start_va = VirtAddr::from(start);
        let mut vpn = VirtPageNum::from(start_va.floor());
        let pte = page_table.translate(vpn);
        if pte.is_none() {
            error!("error translating va {:x}, vpn = {:x}", start, vpn.0);
            return Vec::new();
        }
        let pte = pte.unwrap();
        v.push(pte);
        vpn.step();
        let end_va = VirtAddr::from(end.min(VirtAddr::from(vpn).into()));
        start = end_va.into();
    }
    v
}
