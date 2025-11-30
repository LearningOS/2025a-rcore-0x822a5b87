//! utilities for memory management

use crate::mm::{PTEFlags, PageTable, PageTableEntry, StepByOne, VirtAddr, VirtPageNum};
use alloc::vec::Vec;

/// Translate a virtual address to a physical address through page table
pub fn translate_va_to_pa(token: usize, ptr: *const u8, len: usize) -> Vec<&'static mut [u8]> {
    let mut v = Vec::new();
    let page_table = PageTable::from_token(token);
    let mut start = ptr as usize;
    let end = start + len;

    while start < end {
        let start_va = VirtAddr::from(start);
        let mut vpn = VirtPageNum::from(start_va.floor());
        let ppn = page_table.translate(vpn).unwrap().ppn();
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
        let pte = page_table.translate(vpn).unwrap();
        v.push(pte);
        vpn.step();
        let end_va = VirtAddr::from(end.min(VirtAddr::from(vpn).into()));
        start = end_va.into();
    }
    v
}
