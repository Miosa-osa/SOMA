.section .text
.global _start
.type _start, %function
_start:
    mov x0, #1
    adr x1, sentinel
    mov x2, #13
    mov x8, #64
    svc #0
1:
    wfe
    b 1b

.section .rodata
sentinel:
    .ascii "SOMA_ARM64_OK"
