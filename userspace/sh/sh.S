.section .text
.global _start
_start:
loop:
    /* write(1, prompt, 2) */
    mov $1, %rax
    mov $1, %rdi
    lea prompt(%rip), %rsi
    mov $2, %rdx
    syscall

    /* read(0, buf, 127) */
    xor %rax, %rax
    xor %rdi, %rdi
    lea buf(%rip), %rsi
    mov $127, %rdx
    syscall
    test %rax, %rax
    jle loop

    /* write(1, buf, rax) */
    mov %rax, %rdx
    mov $1, %rax
    mov $1, %rdi
    lea buf(%rip), %rsi
    syscall
    jmp loop

prompt:
    .ascii "$ "
buf:
    .space 128
