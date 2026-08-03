use core::mem::size_of;
use core::arch::asm;
use crate::gdt;

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
        self.segment_selector = unsafe { gdt::SELECTORS.get().unwrap().kernel_code.0 };
        self.options = 0x8E00; // Present (1) | DPL (00) | Storage (0) | Type (1110 = 0xE = Interrupt Gate) -> 0x8E00
        self.offset_mid = (handler >> 16) as u16;
        self.offset_high = (handler >> 32) as u32;
        self.reserved = 0;
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
    fn general_protection_fault_stub();
    fn page_fault_stub();
    fn timer_stub();
}

pub fn init() {
    unsafe {
        // Initialize common exception handlers
        IDT[13].set_handler_fn(general_protection_fault_stub as u64); // #GP
        IDT[14].set_handler_fn(page_fault_stub as u64); // #PF
        
        // We will add the Timer handler here later (Vector 32)
        IDT[32].set_handler_fn(timer_stub as u64); // IRQ0 (Timer)
        
        IDT_PTR.limit = (size_of::<[IdtEntry; 256]>() - 1) as u16;
        IDT_PTR.base = IDT.as_ptr() as u64;

        asm!("lidt [{}]", in(reg) &IDT_PTR, options(readonly, nostack, preserves_flags));
    }
}

// Exception assembly stubs that push error codes and call Rust handlers.
// The CPU pushes: SS, RSP, RFLAGS, CS, RIP, (ErrorCode - for some exceptions).
// #GP (13) and #PF (14) push an error code.

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
    "push 0", // Dummy error code
    "push 32", // Push vector number
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
    
    // For IRQ0 (Timer), the handler might return.
    // If it does, we need to restore context.
    // Notice that `exception_handler` returns if vector == 32!
    
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
    pub r9:  u64,
    pub r8:  u64,
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
    pub cs:  u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss:  u64,
}

static mut TICK_COUNT: u64 = 0;

#[no_mangle]
pub extern "C" fn exception_handler(frame: *const ExceptionFrame) {
    let frame = unsafe { &*frame };
    
    if frame.vector == 32 {
        // Timer Interrupt
        unsafe {
            TICK_COUNT += 1;
            // Print every tick to prove it's firing and preempting
            let mut serial = crate::SerialPort(0x3F8);
            serial.write_str("[KERNEL TIMER] tick ");
            serial.write_hex(TICK_COUNT);
            if let Some(id) = crate::task::current_task_id() {
                serial.write_str(", preempting Task ");
                serial.write_hex(id);
            }
            serial.write_str(" at RIP=");
            serial.write_hex(frame.rip);
            serial.write_str("\n");
            
            crate::pic::end_of_interrupt(0);
            
            // Force preemption immediately for the test
            crate::task::schedule();
        }
        return;
    }
    
    let mut serial = crate::SerialPort(0x3F8);
    
    serial.write_str("\n[KERNEL PANIC] Exception ");
    serial.write_hex(frame.vector);
    serial.write_str(" (Error Code: ");
    serial.write_hex(frame.error_code);
    serial.write_str(")\n");
    
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
