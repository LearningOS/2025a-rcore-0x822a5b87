# enable alternate macro syntax
.altmacro
# Store the n-th register into the stack pointer at an offset of n * 8.
.macro SAVE_GP n
    sd x\n, \n*8(sp)
.endm
# load register from stack
.macro LOAD_GP n
    ld x\n, \n*8(sp)
.endm

    .section .text
    .globl __alltraps
    .globl __restore
    .align 2
__alltraps:
    # swap sp and sscratch in order to avoid using sp of user
    csrrw sp, sscratch, sp
    # now sp->kernel stack, sscratch->user stack
    # allocate a TrapContext on kernel stack, which includes 32 general-purpose registers, sstatus and sepc
    addi sp, sp, -34*8
    # save general-purpose registers
    sd x1, 1*8(sp)
    # skip sp(x2), we will save it later
    sd x3, 3*8(sp)
    # skip tp(x4), application does not use it
    # save x5~x31
    .set n, 5
    .rept 27
        SAVE_GP %n
        .set n, n+1
    .endr

    # Now the user's context is completely stored on the kernel stack, so we can use t0/t1/t2 freely.
    # Here's another tip: sstatus and sepc are not user-purpose registers, which means they can only be
    # operated via csrr rather than ld.
    # On the other hand, sstatus and sepc cannot be directly transferred from system registers to memory.
    # Therefore, using user-purpose registers as an intermediate buffer is necessary.
    csrr t0, sstatus
    csrr t1, sepc
    sd t0, 32*8(sp)
    sd t1, 33*8(sp)

    # read user stack from sscratch and save it on the kernel stack
    csrr t2, sscratch
    sd t2, 2*8(sp)
    # set input argument of trap_handler(cx: &mut TrapContext)
    mv a0, sp
    call trap_handler

__restore:
    # case1: start running app by __restore
    # case2: back to U after handling trap
    mv sp, a0
    # now sp points to kernel stack(after allocated), sscratch points to user stack
    # restore sstatus/sepc

    # t0 equals sstatus, t1 equals sepc, t2 equals sscratch
    ld t0, 32*8(sp)
    ld t1, 33*8(sp)
    ld t2, 2*8(sp)
    csrw sstatus, t0
    csrw sepc, t1
    csrw sscratch, t2

    # restore general-purpuse registers except sp/tp
    ld x1, 1*8(sp)
    ld x3, 3*8(sp)
    .set n, 5
    .rept 27
        LOAD_GP %n
        .set n, n+1
    .endr

    # release TrapContext on kernel stack
    addi sp, sp, 34*8
    # now sp->kernel stack, sscratch->user stack
    csrrw sp, sscratch, sp
    sret
