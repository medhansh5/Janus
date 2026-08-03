ENTRY(_start)

SECTIONS {
    /* Higher-half virtual address for the kernel (top 2GB for 32S relocations) */
    . = 0xFFFFFFFF80000000;

    .text : ALIGN(4K) {
        *(.text .text.*)
    }

    .rodata : ALIGN(4K) {
        *(.rodata .rodata.*)
    }

    .data : ALIGN(4K) {
        *(.data .data.*)
    }

    .bss : ALIGN(4K) {
        *(.bss .bss.*)
    }
}
