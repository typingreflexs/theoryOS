ENTRY(_start)

PAGE_SIZE = 0x1000;

PHDRS
{
    limine_requests PT_LOAD FLAGS(4);
    text           PT_LOAD FLAGS(5);
    rodata         PT_LOAD FLAGS(4);
    data           PT_LOAD FLAGS(6);
}

SECTIONS
{
    . = 0xffffffff80000000;

    .limine_requests : {
        KEEP(*(.limine_requests_start))
        KEEP(*(.limine_requests))
        KEEP(*(.limine_requests_end))
    } :limine_requests

    . = ALIGN(PAGE_SIZE);

    .text : {
        KEEP(*(.text.boot))
        KEEP(*(.text.trampoline))
        *(.text.interrupt_stubs)
        *(.text .text.*)
    } :text

    . = ALIGN(PAGE_SIZE);

    .rodata : {
        *(.rodata .rodata.*)
        *(.srodata .srodata.*)
    } :rodata

    . = ALIGN(PAGE_SIZE);

    .data : {
        *(.data .data.*)
        *(.sdata .sdata.*)
    } :data

    .bss : {
        *(.bss .bss.*)
        *(COMMON)
        *(.sbss .sbss.*)
    } :data

    /DISCARD/ : {
        *(.eh_frame*)
        *(.note .note.*)
    }
}
