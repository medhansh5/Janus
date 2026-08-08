use core::arch::asm;

const PIC1_COMMAND: u16 = 0x20;
const PIC1_DATA: u16    = 0x21;
const PIC2_COMMAND: u16 = 0xA0;
const PIC2_DATA: u16    = 0xA1;

const PIC_EOI: u8 = 0x20;

const ICW1_ICW4: u8 = 0x01; // ICW4 (not) needed
const ICW1_INIT: u8 = 0x10; // Initialization - required!
const ICW4_8086: u8 = 0x01; // 8086/88 (MCS-80/85) mode

#[inline(always)]
unsafe fn outb(port: u16, val: u8) {
    asm!("out dx, al", in("dx") port, in("al") val, options(nomem, nostack, preserves_flags));
}

#[inline(always)]
unsafe fn inb(port: u16) -> u8 {
    let mut val: u8;
    asm!("in al, dx", out("al") val, in("dx") port, options(nomem, nostack, preserves_flags));
    val
}

pub unsafe fn init() {
    let offset1 = 32;
    let offset2 = 40;

    // Save masks
    let a1 = inb(PIC1_DATA);
    let a2 = inb(PIC2_DATA);

    // Starts the initialization sequence (in cascade mode)
    outb(PIC1_COMMAND, ICW1_INIT | ICW1_ICW4);
    // io_wait()
    outb(PIC2_COMMAND, ICW1_INIT | ICW1_ICW4);
    // io_wait()

    // ICW2: Master PIC vector offset
    outb(PIC1_DATA, offset1);
    // io_wait()
    // ICW2: Slave PIC vector offset
    outb(PIC2_DATA, offset2);
    // io_wait()

    // ICW3: tell Master PIC that there is a slave PIC at IRQ2 (0000 0100)
    outb(PIC1_DATA, 4);
    // io_wait()
    // ICW3: tell Slave PIC its cascade identity (0000 0010)
    outb(PIC2_DATA, 2);
    // io_wait()

    // ICW4: have the PICs use 8086 mode (and not 8080 mode)
    outb(PIC1_DATA, ICW4_8086);
    // io_wait()
    outb(PIC2_DATA, ICW4_8086);
    // io_wait()

    // Restore saved masks
    outb(PIC1_DATA, a1);
    outb(PIC2_DATA, a2);
}

pub unsafe fn end_of_interrupt(irq: u8) {
    if irq >= 8 {
        outb(PIC2_COMMAND, PIC_EOI);
    }
    outb(PIC1_COMMAND, PIC_EOI);
}

pub unsafe fn unmask(irq: u8) {
    let port;
    let value;
    if irq < 8 {
        port = PIC1_DATA;
    } else {
        port = PIC2_DATA;
        // Unmask IRQ 2 on master PIC to allow cascade
        let v_master = inb(PIC1_DATA) & !(1 << 2);
        outb(PIC1_DATA, v_master);
    }
    value = inb(port) & !(1 << (irq % 8));
    outb(port, value);
}

pub unsafe fn mask(irq: u8) {
    let port;
    let value;
    if irq < 8 {
        port = PIC1_DATA;
    } else {
        port = PIC2_DATA;
    }
    value = inb(port) | (1 << (irq % 8));
    outb(port, value);
}
