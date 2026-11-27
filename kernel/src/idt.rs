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
    fn irq1_stub();
    fn irq2_stub();
    fn irq3_stub();
    fn irq4_stub();
    fn irq5_stub();
    fn irq6_stub();
    fn irq7_stub();
    fn irq8_stub();
    fn irq9_stub();
    fn irq10_stub();
    fn irq11_stub();
    fn irq12_stub();
    fn irq13_stub();
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

        // Hardware IRQ handlers (Vectors 32..47 -> IRQ 0..15)
        IDT[32].set_handler_fn(timer_stub as *const () as usize as u64);
        IDT[33].set_handler_fn(irq1_stub as *const () as usize as u64);
        IDT[34].set_handler_fn(irq2_stub as *const () as usize as u64);
        IDT[35].set_handler_fn(irq3_stub as *const () as usize as u64);
        IDT[36].set_handler_fn(irq4_stub as *const () as usize as u64);
        IDT[37].set_handler_fn(irq5_stub as *const () as usize as u64);
        IDT[38].set_handler_fn(irq6_stub as *const () as usize as u64);
        IDT[39].set_handler_fn(irq7_stub as *const () as usize as u64);
        IDT[40].set_handler_fn(irq8_stub as *const () as usize as u64);
        IDT[41].set_handler_fn(irq9_stub as *const () as usize as u64);
        IDT[42].set_handler_fn(irq10_stub as *const () as usize as u64);
        IDT[43].set_handler_fn(irq11_stub as *const () as usize as u64);
        IDT[44].set_handler_fn(irq12_stub as *const () as usize as u64);
        IDT[45].set_handler_fn(irq13_stub as *const () as usize as u64);
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
    "push 0",
    "push 32",
    "jmp exception_common"
);

core::arch::global_asm!(
    ".global irq1_stub",
    "irq1_stub:",
    "push 0",
    "push 33",
    "jmp exception_common"
);

core::arch::global_asm!(
    ".global irq2_stub",
    "irq2_stub:",
    "push 0",
    "push 34",
    "jmp exception_common"
);

core::arch::global_asm!(
    ".global irq3_stub",
    "irq3_stub:",
    "push 0",
    "push 35",
    "jmp exception_common"
);

core::arch::global_asm!(
    ".global irq4_stub",
    "irq4_stub:",
    "push 0",
    "push 36",
    "jmp exception_common"
);

core::arch::global_asm!(
    ".global irq5_stub",
    "irq5_stub:",
    "push 0",
    "push 37",
    "jmp exception_common"
);

core::arch::global_asm!(
    ".global irq6_stub",
    "irq6_stub:",
    "push 0",
    "push 38",
    "jmp exception_common"
);

core::arch::global_asm!(
    ".global irq7_stub",
    "irq7_stub:",
    "push 0",
    "push 39",
    "jmp exception_common"
);

core::arch::global_asm!(
    ".global irq8_stub",
    "irq8_stub:",
    "push 0",
    "push 40",
    "jmp exception_common"
);

core::arch::global_asm!(
    ".global irq9_stub",
    "irq9_stub:",
    "push 0",
    "push 41",
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
    ".global irq12_stub",
    "irq12_stub:",
    "push 0",
    "push 44",
    "jmp exception_common"
);

core::arch::global_asm!(
    ".global irq13_stub",
    "irq13_stub:",
    "push 0",
    "push 45",
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

pub fn get_tick_count() -> u64 {
    unsafe { TICK_COUNT }
}

/// Main exception handling dispatcher.
///
/// # Safety
/// `frame` must be a valid pointer to an ExceptionFrame allocated on the stack by exception stubs.
#[no_mangle]
pub unsafe extern "C" fn exception_handler(frame: *const ExceptionFrame) {
    let frame = &*frame;

    if frame.vector == 32 {
        // Timer Interrupt (IRQ 0)
        unsafe {
            TICK_COUNT = TICK_COUNT.wrapping_add(1);

            crate::task::wake_sleeping_tasks(TICK_COUNT);

            #[cfg(not(test))]
            crate::console::poll_input();

            crate::pic::end_of_interrupt(0);

            crate::task::schedule();
        }
        return;
    } else if (33..=47).contains(&frame.vector) {
        let irq_num = (frame.vector - 32) as u8;

        unsafe {
            if crate::pic::is_spurious(irq_num) {
                if irq_num == 15 {
                    crate::pic::end_of_interrupt(7);
                }
                return;
            }

            #[cfg(not(test))]
            if irq_num == 1 || irq_num == 4 {
                crate::console::poll_input();
            }

            crate::notification::handle_hardware_irq(irq_num);
            crate::ipc::wake_irq(irq_num);

            crate::pic::mask(irq_num);
            crate::notification::mark_irq_masked(irq_num);
            crate::pic::end_of_interrupt(irq_num);
        }
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
