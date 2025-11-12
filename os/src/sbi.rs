//! SBI call wrappers

use core::arch::asm;

// SBI function IDs
const SBI_CONSOLE_PUTCHAR: usize = 1;

/// general sbi call
#[inline(always)]
fn sbi_call(which: usize, arg0: usize, arg1: usize, arg2: usize) -> usize {
    let mut ret;
    unsafe {
        asm!(
            "li a6, 0",                        // set SBI extension ID to 0
            "ecall",                            // trap into supervisor mode
            inlateout("a0") arg0 => ret,       // x10 is both input and output: arg0 and return value
            in("a1") arg1,                     // x11: arg1
            in("a2") arg2,                     // x12: arg2
            in("a7") which,                    // x17: SBI function ID
        );
    }
    ret
}

/// use sbi call to putchar in console (qemu uart handler)
pub fn console_putchar(c: usize) {
    sbi_call(SBI_CONSOLE_PUTCHAR, c, 0, 0);
}

use crate::board::QEMUExit;
/// use sbi call to shut down the kernel
pub fn shutdown() -> ! {
    crate::board::QEMU_EXIT_HANDLE.exit_failure();
}
