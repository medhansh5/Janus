use crate::gdt;
use core::arch::asm;

#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct IdtEntry {
    offset_low: u16,
    segment_selector: u16,
    options: u16,
    offset_mid: u16,
    offset_high: u32,
    reserved: u32,
}

impl IdtEntry {
    pub const fn empty() -> Self {
        IdtEntry {
            offset_low: 0,
            segment_selector: 0,
            options: 0,
            offset_mid: 0,
            offset_high: 0,
            reserved: 0,
        }
    }

    pub fn set_handler_fn(&mut self, handler: u64) {
        self.offset_low = handler as u16;
        self.segment_selector = gdt::SELECTORS.get().unwrap().kernel_code.0;
        self.options = 0x8E00; // Present (1) | DPL (00) | Storage (0) | Type (1110 = 0xE = Interrupt Gate) -> 0x8E00
        self.offset_mid = (handler >> 16) as u16;
        self.offset_high = (handler >> 32) as u32;
        self.reserved = 0;
    }

    pub fn set_handler_fn_ist(&mut self, handler: u64, ist_index: u16) {
        self.set_handler_fn(handler);
        // IST index is 1-based in IDT options (bits 0..2).
        // Array index 0 in TSS corresponds to IST value 1 in the IDT gate.
        self.options = 0x8E00 | (ist_index + 1);
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct IdtPtr {
    limit: u16,
    base: u64,
}

// Global IDT
static mut IDT: [IdtEntry; 256] = [IdtEntry::empty(); 256];
static mut IDT_PTR: IdtPtr = IdtPtr { limit: 0, base: 0 };

extern "C" {
    fn double_fault_stub();
    fn general_protection_fault_stub();
    fn page_fault_stub();
    fn timer_stub();
    fn irq10_stub();
    fn irq11_stub();
    fn irq14_stub();
    fn irq15_stub();
}

pub fn init() {
    unsafe {
        // Initialize common exception handlers
        IDT[8].set_handler_fn_ist(
            double_fault_stub as *const () as usize as u64,
            gdt::DOUBLE_FAULT_IST_INDEX,
        ); // #DF (Double Fault)
        IDT[13].set_handler_fn(general_protection_fault_stub as *const () as usize as u64); // #GP
        IDT[14].set_handler_fn(page_fault_stub as *const () as usize as u64); // #PF

        // We will add the Timer handler here later (Vector 32)
        IDT[32].set_handler_fn(timer_stub as *const () as usize as u64); // IRQ0 (Timer)

        // IRQ10, IRQ11, IRQ14, IRQ15 (virtio-blk might be any of these)
        IDT[42].set_handler_fn(irq10_stub as *const () as usize as u64);
        IDT[43].set_handler_fn(irq11_stub as *const () as usize as u64);
        IDT[46].set_handler_fn(irq14_stub as *const () as usize as u64);
        IDT[47].set_handler_fn(irq15_stub as *const () as usize as u64);

        IDT_PTR.limit = (core::mem::size_of::<[IdtEntry; 256]>() - 1) as u16;
        IDT_PTR.base = core::ptr::addr_of!(IDT) as u64;

        core::arch::asm!("lidt [{}]", in(reg) core::ptr::addr_of!(IDT_PTR), options(readonly, nostack, preserves_flags));
    }
}

// Exception assembly stubs that push error codes and call Rust handlers.
// The CPU pushes: SS, RSP, RFLAGS, CS, RIP, (ErrorCode - for some exceptions).
// #GP (13) and #PF (14) push an error code.
// #DF (8) also pushes a hardware error code (value is always 0).

core::arch::global_asm!(
    ".global double_fault_stub",
    "double_fault_stub:",
    // Hardware already pushed error code (0) for #DF
    "push 8", // Push vector number
    "jmp exception_common"
);

core::arch::global_asm!(
    ".global general_protection_fault_stub",
    "general_protection_fault_stub:",
    // Hardware already pushed error code
    "push 13", // Push vector number
    "jmp exception_common"
);

core::arch::global_asm!(
    ".global page_fault_stub",
    "page_fault_stub:",
    // Hardware already pushed error code
    "push 14", // Push vector number
    "jmp exception_common"
);

core::arch::global_asm!(
    ".global timer_stub",
    "timer_stub:",
    // Hardware DOES NOT push an error code for IRQ0
    "push 0",  // Dummy error code
    "push 32", // Push vector number
    "jmp exception_common"
);

core::arch::global_asm!(
    ".global irq10_stub",
    "irq10_stub:",
    "push 0",
    "push 42",
    "jmp exception_common"
);

core::arch::global_asm!(
    ".global irq11_stub",
    "irq11_stub:",
    "push 0",
    "push 43",
    "jmp exception_common"
);

core::arch::global_asm!(
    ".global irq14_stub",
    "irq14_stub:",
    "push 0",
    "push 46",
    "jmp exception_common"
);

core::arch::global_asm!(
    ".global irq15_stub",
    "irq15_stub:",
    "push 0",
    "push 47",
    "jmp exception_common"
);

core::arch::global_asm!(
    "exception_common:",
    // Save GP registers to construct an InterruptFrame
    "push rax",
    "push rcx",
    "push rdx",
    "push rbx",
    "push rbp",
    "push rsi",
    "push rdi",
    "push r8",
    "push r9",
    "push r10",
    "push r11",
    "push r12",
    "push r13",
    "push r14",
    "push r15",
    // RSP now points to the top of our extended InterruptFrame.
    // Call the Rust handler with a pointer to the frame.
    "mov rdi, rsp",
    "call exception_handler",
    // For IRQ0 (Timer) and IRQ14, the handler might return.
    // If it does, we need to restore context.
    // Notice that `exception_handler` returns if vector == 32 or 46!
    "pop r15",
    "pop r14",
    "pop r13",
    "pop r12",
    "pop r11",
    "pop r10",
    "pop r9",
    "pop r8",
    "pop rdi",
    "pop rsi",
    "pop rbp",
    "pop rbx",
    "pop rdx",
    "pop rcx",
    "pop rax",
    "add rsp, 16", // Skip vector and error_code
    "iretq"
);

// This matches the stack layout of `exception_common`
#[repr(C)]
#[derive(Debug)]
pub struct ExceptionFrame {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rbp: u64,
    pub rbx: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rax: u64,

    pub vector: u64,
    pub error_code: u64,

    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

static mut TICK_COUNT: u64 = 0;

/// Main exception handling dispatcher.
///
/// # Safety
/// `frame` must be a valid pointer to an ExceptionFrame allocated on the stack by exception stubs.
#[no_mangle]
pub unsafe extern "C" fn exception_handler(frame: *const ExceptionFrame) {
    let frame = &*frame;

    if frame.vector == 32 {
        // Timer Interrupt
        unsafe {
            TICK_COUNT += 1;

            crate::pic::end_of_interrupt(0);

            // Force preemption immediately for the test
            crate::task::schedule();
        }
        return;
    } else if (42..=47).contains(&frame.vector) {
        // IRQ10=42, IRQ11=43, IRQ14=46, IRQ15=47
        let irq_num = frame.vector - 32;

        let mut serial = crate::SerialPort(0x3F8);
        serial.write_str("[KERNEL IRQ] Fired IRQ ");
        serial.write_hex(irq_num);
        serial.write_str("\n");

        unsafe {
            crate::pic::end_of_interrupt(irq_num as u8);
        }

        // Wake tasks waiting on this IRQ
        crate::ipc::wake_irq(irq_num as u8);

        return;
    }

    let mut serial = crate::SerialPort(0x3F8);

    serial.write_str("\n[KERNEL PANIC] Exception ");
    serial.write_hex(frame.vector);
    serial.write_str(" (Error Code: ");
    serial.write_hex(frame.error_code);
    serial.write_str(")\n");

    if frame.vector == 8 {
        serial.write_str(
            "[KERNEL PANIC] Double Fault (#DF) triggered! Dedicated IST stack active.\n",
        );
    }

    if frame.vector == 14 {
        let cr2: u64;
        unsafe { asm!("mov {}, cr2", out(reg) cr2) };
        serial.write_str("Faulting Address (CR2): ");
        serial.write_hex(cr2);
        serial.write_str("\n");
    }

    serial.write_str("RIP: ");
    serial.write_hex(frame.rip);
    serial.write_str(" RSP: ");
    serial.write_hex(frame.rsp);
    serial.write_str("\n");

    serial.write_str("System halted.\n");
    loop {
        unsafe { asm!("hlt") };
    }
}
