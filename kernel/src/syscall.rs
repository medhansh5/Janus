use x86_64::registers::model_specific::{Efer, EferFlags};
use x86_64::registers::model_specific::{LStar, Star, SFMask};
use x86_64::registers::rflags::RFlags;
use crate::gdt;
use core::arch::global_asm;

/// Initializes the MSRs required for the `syscall` / `sysret` instructions.
pub unsafe fn init() {
    let selectors = gdt::SELECTORS.get().expect("GDT must be initialized before syscalls");
    Efer::write(Efer::read() | EferFlags::SYSTEM_CALL_EXTENSIONS);

    // Enable EFER.SCE for syscall/sysret
    Efer::write(Efer::read() | EferFlags::SYSTEM_CALL_EXTENSIONS);

    // STAR: Syscall Target Address Register
    // Bits 47:32 = Kernel CS/SS for syscall
    // Bits 63:48 = User CS/SS for sysret
    Star::write(selectors.user_code, selectors.user_data, selectors.kernel_code, selectors.kernel_data).unwrap();
    
    // LSTAR: Syscall Entry Point
    LStar::write(x86_64::VirtAddr::new(syscall_entry as u64));
    
    // FMASK: Flags to clear on syscall (Clear IF to disable interrupts during kernel entry)
    SFMask::write(RFlags::INTERRUPT_FLAG);
}

/// The naked assembly entry point for the `syscall` instruction.
/// The `syscall` instruction does NOT change RSP. It leaves it pointing to the user stack.
/// It saves the old RIP into RCX and old RFLAGS into R11.
global_asm!(
    ".global syscall_entry",
    "syscall_entry:",
    "mov [USER_RSP_SCRATCH], rsp",
    "mov rsp, [KERNEL_RSP_SCRATCH]",
    
    // Construct the hardware-like Interrupt Frame (for iretq)
    // 0x1B = User Data Segment | Ring 3 (Index 3)
    // 0x23 = User Code Segment | Ring 3 (Index 4)
    "push 0x1B",                 // SS
    "push [USER_RSP_SCRATCH]",   // RSP
    "push r11",                  // RFLAGS (syscall saves RFLAGS in R11)
    "push 0x23",                 // CS
    "push rcx",                  // RIP (syscall saves RIP in RCX)
    
    // Push general purpose registers in the order of TaskContext
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
    
    // Syscall arguments:
    // rdi = syscall number
    // rsi = arg1
    // rdx = arg2
    // r10 = arg3 (sysv64 uses rcx for arg4, but user passed it in r10)
    // r8  = arg4
    // r9  = arg5
    "mov rcx, r10", // Move arg3 to rcx for sysv64 ABI
    
    // Pass a pointer to the TaskContext (which is currently at RSP) as an additional arg?
    // Not needed for now, but good to know it's there.
    "call syscall_handler",
    
    // syscall_handler returns result in RAX.
    // The pushed RAX is at [rsp + 14 * 8] (since we pushed 15 regs, rax was first).
    // Let's just pop everything up to RAX, then set RAX, then iretq.
    
    // Pop general purpose registers
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
    
    // We don't want to pop the old RAX. We want to keep the new RAX (the return value).
    // So we just add 8 to RSP to skip the saved RAX.
    "add rsp, 8",
    
    // Return to user space via iretq
    "iretq"
);

extern "C" {
    fn syscall_entry();
}

// These are used by the assembly stub to switch stacks.
// In a real OS, these would be in the TSS or a per-CPU data structure pointed to by GS base.
#[no_mangle]
pub static mut USER_RSP_SCRATCH: u64 = 0;
#[no_mangle]
pub static mut KERNEL_RSP_SCRATCH: u64 = 0;

#[no_mangle]
pub extern "sysv64" fn syscall_handler(sys_num: u64, arg1: u64, arg2: u64, arg3: u64, arg4: u64, arg5: u64) -> u64 {
    crate::ipc::handle_syscall(sys_num, arg1, arg2, arg3, arg4, arg5)
}
