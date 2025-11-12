    // 声明 .text.entry 段
    .section .text.entry
    .globl _start
_start:
    // 初始化栈指针
    la sp, boot_stack_top
    // 跳转rust的main函数
    call rust_main

    // 声明 .bss.stack 段
    .section .bss.stack
    .globl boot_stack_lower_bound
boot_stack_lower_bound:
    // 填充空间 64k
    .space 4096 * 16

    .globl boot_stack_top
boot_stack_top: