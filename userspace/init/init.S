.section .text
.global _start
_start:
    /* write(1, msg, len) */
    mov $1, %rax
    mov $1, %rdi
    lea msg(%rip), %rsi
    mov $len, %rdx
    syscall

    /* execve("/bin/ui", NULL, NULL) */
    mov $59, %rax
    lea uipath(%rip), %rdi
    xor %rsi, %rsi
    xor %rdx, %rdx
    syscall

    /* execve("/bin/sh", NULL, NULL) fallback */
    mov $59, %rax
    lea shpath(%rip), %rdi
    xor %rsi, %rsi
    xor %rdx, %rdx
    syscall

    /* exit(1) */
    mov $60, %rax
    mov $1, %rdi
    syscall

msg:
    .ascii "Theory OS init (PID 1)\n"
len = . - msg
uipath:
    .ascii "/bin/ui\0"
shpath:
    .ascii "/bin/sh\0"
