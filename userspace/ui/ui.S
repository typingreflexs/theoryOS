.section .text
.global _start

/* PID 1 UI — keeps desktop session alive; kernel timer refreshes the clock. */
_start:
    /* nanosleep({500ms, 0}, NULL) in a loop */
1:
    lea ts(%rip), %rdi
    xor %rsi, %rsi
    mov $35, %rax
    syscall
    jmp 1b

.section .rodata
.align 8
ts:
    .quad 0
    .quad 500000000
