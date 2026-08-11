use crate::memory::FrameAllocator as PhysFrameAllocator;
use x86_64::structures::paging::page_table::PageTableEntry;
use x86_64::structures::paging::{PageTable, PageTableFlags, PhysFrame};
use x86_64::{PhysAddr, VirtAddr};

pub const PHYS_MEMORY_OFFSET: u64 = 0xFFFF900000000000;

pub unsafe fn init_root_page_table(allocator: &mut PhysFrameAllocator) {
    // Read current CR3 to get the active PML4 (provided by bootloader/UEFI)
    let cr3_frame = x86_64::registers::control::Cr3::read().0;
    let old_pml4_ptr = cr3_frame.start_address().as_u64() as *const PageTable;
    let old_pml4 = &*old_pml4_ptr;

    // Allocate a new frame for our kernel-owned PML4
    let new_pml4_paddr = allocator
        .allocate_frame()
        .expect("Failed to allocate new PML4");
    // Since we are currently running under UEFI's identity map, we can access it directly
    let new_pml4_ptr = new_pml4_paddr as *mut PageTable;
    core::ptr::write_bytes(new_pml4_ptr as *mut u8, 0, 4096);
    let new_pml4 = &mut *new_pml4_ptr;

    // 1. Copy ALL mappings initially (including lower-half) to keep the UEFI stack alive
    // during the CR3 switch. We will unmap the lower-half once we switch to a new stack.
    for i in 0..512 {
        new_pml4[i] = old_pml4[i].clone();
    }

    // 2. Map all physical memory at PHYS_MEMORY_OFFSET
    // We will use a PDPT (Page Directory Pointer Table) and map using 1GB huge pages.
    // NOTE: QEMU qemu64 CPU may not support 1GB pages, but we can use 2MB pages easily.
    // 4GB / 2MB = 2048 pages. A PD has 512 entries, so we need 4 PDs and 1 PDPT.
    let pdpt_paddr = allocator.allocate_frame().expect("Failed to alloc PDPT");
    let pdpt = unsafe { &mut *(pdpt_paddr as *mut PageTable) };
    core::ptr::write_bytes(pdpt as *mut PageTable as *mut u8, 0, 4096);

    for i in 0..4 {
        let pd_paddr = allocator.allocate_frame().expect("Failed to alloc PD");
        let pd = unsafe { &mut *(pd_paddr as *mut PageTable) };
        core::ptr::write_bytes(pd as *mut PageTable as *mut u8, 0, 4096);

        for j in 0..512 {
            let paddr = (i as u64 * 512 + j as u64) * 0x200000; // 2MB increments
            let mut entry = PageTableEntry::new();
            let flags = PageTableFlags::PRESENT
                | PageTableFlags::WRITABLE
                | PageTableFlags::HUGE_PAGE
                | PageTableFlags::NO_EXECUTE;
            entry.set_addr(PhysAddr::new(paddr), flags);
            pd[j] = entry;
        }

        let mut pdpt_entry = PageTableEntry::new();
        pdpt_entry.set_addr(
            PhysAddr::new(pd_paddr),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE,
        );
        pdpt[i] = pdpt_entry;
    }

    // Link the PDPT into the new PML4
    let offset_vaddr = VirtAddr::new(PHYS_MEMORY_OFFSET);
    let pml4_index = offset_vaddr.p4_index();

    let mut pml4_entry = PageTableEntry::new();
    pml4_entry.set_addr(
        PhysAddr::new(pdpt_paddr),
        PageTableFlags::PRESENT | PageTableFlags::WRITABLE,
    );
    new_pml4[pml4_index] = pml4_entry;

    // 3. Switch CR3 to the new PML4
    let new_cr3_frame = PhysFrame::containing_address(PhysAddr::new(new_pml4_paddr));
    x86_64::registers::control::Cr3::write(
        new_cr3_frame,
        x86_64::registers::control::Cr3Flags::empty(),
    );

    // Update allocator offset now that mapping is active
    allocator.set_phys_offset(PHYS_MEMORY_OFFSET);

    // 5. Unmap lower-half and switch to a kernel stack
    // We only need to unmap lower-half because we don't want to use it.
    // We'll leave it for a moment, just returning. We will do the stack switch in _start!
}

/// Unmaps the lower-half of the active PML4 and switches to a new stack.
/// This function never returns, but instead jumps to the provided `stage2` entry point.
pub unsafe fn unmap_lower_half_and_switch_stack(
    new_rsp: u64,
    stage2: extern "sysv64" fn() -> !,
) -> ! {
    // 1. Get current PML4 physical address from CR3
    let cr3_frame = x86_64::registers::control::Cr3::read().0;
    let pml4_paddr = cr3_frame.start_address().as_u64();

    // 2. Clear the lower-half (indices 0..256) of the PML4
    // We must use the PHYS_MEMORY_OFFSET mapping to access the PML4 now.
    let pml4_vaddr = pml4_paddr + PHYS_MEMORY_OFFSET;

    // 3. Switch RSP, clear lower-half, reload CR3 (TLB flush), and jump to stage2
    core::arch::asm!(
        "mov rsp, {0}",
        "mov rbp, rsp", // Reset RBP for the new stack frame

        "mov rcx, 256",
        "xor rax, rax",
        "mov rdi, {1}",
        "rep stosq",

        "mov rax, cr3",
        "mov cr3, rax",

        "jmp {2}",
        in(reg) new_rsp,
        in(reg) pml4_vaddr,
        in(reg) stage2,
        options(noreturn)
    );
}
