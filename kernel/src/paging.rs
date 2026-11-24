use crate::memory::FrameAllocator as PhysFrameAllocator;
use core::sync::atomic::{AtomicU64, Ordering};
use x86_64::structures::paging::page_table::PageTableEntry;
use x86_64::structures::paging::{PageTable, PageTableFlags, PhysFrame};
use x86_64::{PhysAddr, VirtAddr};

pub const PHYS_MEMORY_OFFSET: u64 = 0xFFFF900000000000;

/// Authoritative physical address of the master kernel root page table (PML4).
///
/// Established once during boot initialization by `init_root_page_table`.
/// This address is permanent and strictly distinct from the dynamic CPU CR3 register contents.
static KERNEL_PML4_PADDR: AtomicU64 = AtomicU64::new(0);

/// Returns the authoritative physical address of the master kernel root page table (PML4).
pub fn kernel_pml4_paddr() -> u64 {
    KERNEL_PML4_PADDR.load(Ordering::Acquire)
}

/// Initializes the root page table (PML4) and maps physical memory.
///
/// # Safety
/// Must be called with valid identity-mapped memory provided by the bootloader.
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

    // Establish authoritative master kernel PML4 physical address.
    // Atomic compare_exchange ensures initialization happens once during boot and cannot be overwritten.
    let _ =
        KERNEL_PML4_PADDR.compare_exchange(0, new_pml4_paddr, Ordering::Release, Ordering::Relaxed);

    // Update allocator offset now that mapping is active
    allocator.set_phys_offset(PHYS_MEMORY_OFFSET);

    // 5. Unmap lower-half and switch to a kernel stack
    // We only need to unmap lower-half because we don't want to use it.
    // We'll leave it for a moment, just returning. We will do the stack switch in _start!
}

/// Unmaps the lower-half of the active PML4 and switches to a new stack.
/// This function never returns, but instead jumps to the provided `stage2` entry point.
///
/// # Safety
/// `new_rsp` must point to a valid stack and `stage2` must be a valid function entry point.
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

/// Errors that can occur when safely copying memory from userspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserCopyError {
    /// Pointer was null (0x0).
    NullPointer,
    /// Address resides in the higher-half kernel range.
    KernelAddress,
    /// Address is non-canonical or outside the valid lower-half user address space.
    NonCanonical,
    /// Address arithmetic overflowed.
    ArithmeticOverflow,
    /// A page in the requested range is not marked PRESENT.
    NotMapped,
    /// A page table entry in the translation path lacks the USER_ACCESSIBLE permission.
    SupervisorOnly,
    /// The translated physical address exceeds the valid direct-mapped memory window or is unaligned.
    InvalidPhysicalAddress,
    /// A page table entry contains reserved bits that violate x86_64 architectural requirements.
    InvalidPageTableEntry,
}

/// Internal representation of a translated virtual page.
#[derive(Debug, Clone, Copy)]
struct TranslatedPage {
    phys_base: u64,
    page_offset: u64,
    page_size: u64,
}

pub const DIRECT_MAP_LIMIT: u64 = 0x1_0000_0000; // 4 GiB direct-mapped physical ceiling
const USER_SPACE_END: u64 = 0x0000_8000_0000_0000;
const KERNEL_SPACE_START: u64 = 0xFFFF_8000_0000_0000;

/// Trait abstracting physical memory reads for page-table walks and data copying.
///
/// # Safety
/// Implementations must ensure that:
/// - Successful `read_table` returns a valid, aligned reference to a `PageTable`.
/// - Successful `read_bytes` safely copies `buf.len()` bytes from physical address `paddr` into `buf`.
///   Implementations must handle potentially overlapping source and destination regions safely
///   (e.g., using `core::ptr::copy` rather than `copy_nonoverlapping`).
pub(crate) unsafe trait PageReader {
    fn read_table(&self, paddr: u64) -> Result<&PageTable, UserCopyError>;
    fn read_bytes(&self, paddr: u64, buf: &mut [u8]) -> Result<(), UserCopyError>;
}

/// Kernel reader operating on the higher-half direct physical map (`PHYS_MEMORY_OFFSET`).
struct KernelDirectMap;

unsafe impl PageReader for KernelDirectMap {
    fn read_table(&self, paddr: u64) -> Result<&PageTable, UserCopyError> {
        let end_paddr = paddr
            .checked_add(4096)
            .ok_or(UserCopyError::ArithmeticOverflow)?;
        if end_paddr > DIRECT_MAP_LIMIT || (paddr & 0xFFF) != 0 {
            return Err(UserCopyError::InvalidPhysicalAddress);
        }
        let vaddr = paddr
            .checked_add(PHYS_MEMORY_OFFSET)
            .ok_or(UserCopyError::ArithmeticOverflow)?;
        Ok(unsafe { &*(vaddr as *const PageTable) })
    }

    fn read_bytes(&self, paddr: u64, buf: &mut [u8]) -> Result<(), UserCopyError> {
        let len = buf.len() as u64;
        let end_paddr = paddr
            .checked_add(len)
            .ok_or(UserCopyError::ArithmeticOverflow)?;
        if end_paddr > DIRECT_MAP_LIMIT {
            return Err(UserCopyError::InvalidPhysicalAddress);
        }
        let vaddr = paddr
            .checked_add(PHYS_MEMORY_OFFSET)
            .ok_or(UserCopyError::ArithmeticOverflow)?;
        // Use core::ptr::copy (memmove semantics) to safely handle any potential
        // overlap between source direct-map virtual addresses and destination buffers.
        unsafe {
            core::ptr::copy(vaddr as *const u8, buf.as_mut_ptr(), buf.len());
        }
        Ok(())
    }
}

/// Walks the page table hierarchy starting at `pml4` using `reader` to validate and translate
/// a single user virtual address `vaddr`.
///
/// Invariants:
/// - Rejects kernel addresses (`p4_index >= 256`).
/// - Enforces `PRESENT` and `USER_ACCESSIBLE` flags at every level (PML4, PDPT, PD, PT).
/// - Enforces 1 GiB and 2 MiB huge-page base alignment and checks architectural reserved bits.
/// - Does not interpret the PAT bit (bit 12) as part of the physical base address.
fn translate_user_page<R: PageReader>(
    reader: &R,
    pml4: &PageTable,
    vaddr: u64,
) -> Result<TranslatedPage, UserCopyError> {
    if vaddr >= KERNEL_SPACE_START {
        return Err(UserCopyError::KernelAddress);
    }
    if vaddr >= USER_SPACE_END {
        return Err(UserCopyError::NonCanonical);
    }

    let virt_addr = VirtAddr::new(vaddr);
    let p4_idx = virt_addr.p4_index();

    // User space mappings must strictly reside in PML4 entries 0..255
    if u16::from(p4_idx) >= 256 {
        return Err(UserCopyError::KernelAddress);
    }

    let p4_entry = &pml4[p4_idx];
    if !p4_entry.flags().contains(PageTableFlags::PRESENT) {
        return Err(UserCopyError::NotMapped);
    }
    if !p4_entry.flags().contains(PageTableFlags::USER_ACCESSIBLE) {
        return Err(UserCopyError::SupervisorOnly);
    }

    // In standard x86_64 4-level paging, bit 7 (Page Size / HUGE_PAGE) in a PML4 entry
    // is reserved and must be zero. If set, it triggers an architectural page fault (#PF(RSVD)).
    if p4_entry.flags().contains(PageTableFlags::HUGE_PAGE) {
        return Err(UserCopyError::InvalidPageTableEntry);
    }

    let pdpt_paddr = p4_entry.addr().as_u64();
    let pdpt = reader.read_table(pdpt_paddr)?;

    let p3_idx = virt_addr.p3_index();
    let p3_entry = &pdpt[p3_idx];
    if !p3_entry.flags().contains(PageTableFlags::PRESENT) {
        return Err(UserCopyError::NotMapped);
    }
    if !p3_entry.flags().contains(PageTableFlags::USER_ACCESSIBLE) {
        return Err(UserCopyError::SupervisorOnly);
    }

    // Check for 1 GiB huge page at Level 3
    if p3_entry.flags().contains(PageTableFlags::HUGE_PAGE) {
        let raw_addr = p3_entry.addr().as_u64();
        // In x86_64 1 GiB page entries, bits 13..29 are reserved and must be 0
        if (raw_addr & 0x3FFF_E000) != 0 {
            return Err(UserCopyError::InvalidPageTableEntry);
        }
        // Base address is 1 GiB aligned (mask bits 0..29, clearing bit 12 PAT)
        let phys_base = raw_addr & !0x3FFF_FFFF;
        const GIB_1: u64 = 1024 * 1024 * 1024;
        let end_phys = phys_base
            .checked_add(GIB_1)
            .ok_or(UserCopyError::ArithmeticOverflow)?;
        if end_phys > DIRECT_MAP_LIMIT {
            return Err(UserCopyError::InvalidPhysicalAddress);
        }
        let page_offset = vaddr & (GIB_1 - 1);
        return Ok(TranslatedPage {
            phys_base,
            page_offset,
            page_size: GIB_1,
        });
    }

    let pd_paddr = p3_entry.addr().as_u64();
    let pd = reader.read_table(pd_paddr)?;

    let p2_idx = virt_addr.p2_index();
    let p2_entry = &pd[p2_idx];
    if !p2_entry.flags().contains(PageTableFlags::PRESENT) {
        return Err(UserCopyError::NotMapped);
    }
    if !p2_entry.flags().contains(PageTableFlags::USER_ACCESSIBLE) {
        return Err(UserCopyError::SupervisorOnly);
    }

    // Check for 2 MiB huge page at Level 2
    if p2_entry.flags().contains(PageTableFlags::HUGE_PAGE) {
        let raw_addr = p2_entry.addr().as_u64();
        // In x86_64 2 MiB page entries, bits 13..20 are reserved and must be 0
        if (raw_addr & 0x001F_E000) != 0 {
            return Err(UserCopyError::InvalidPageTableEntry);
        }
        // Base address is 2 MiB aligned (mask bits 0..20, clearing bit 12 PAT)
        let phys_base = raw_addr & !0x001F_FFFF;
        const MIB_2: u64 = 2 * 1024 * 1024;
        let end_phys = phys_base
            .checked_add(MIB_2)
            .ok_or(UserCopyError::ArithmeticOverflow)?;
        if end_phys > DIRECT_MAP_LIMIT {
            return Err(UserCopyError::InvalidPhysicalAddress);
        }
        let page_offset = vaddr & (MIB_2 - 1);
        return Ok(TranslatedPage {
            phys_base,
            page_offset,
            page_size: MIB_2,
        });
    }

    let pt_paddr = p2_entry.addr().as_u64();
    let pt = reader.read_table(pt_paddr)?;

    let p1_idx = virt_addr.p1_index();
    let p1_entry = &pt[p1_idx];
    if !p1_entry.flags().contains(PageTableFlags::PRESENT) {
        return Err(UserCopyError::NotMapped);
    }
    if !p1_entry.flags().contains(PageTableFlags::USER_ACCESSIBLE) {
        return Err(UserCopyError::SupervisorOnly);
    }

    let phys_base = p1_entry.addr().as_u64();
    const KIB_4: u64 = 4096;
    let end_phys = phys_base
        .checked_add(KIB_4)
        .ok_or(UserCopyError::ArithmeticOverflow)?;
    if end_phys > DIRECT_MAP_LIMIT {
        return Err(UserCopyError::InvalidPhysicalAddress);
    }
    let page_offset = vaddr & (KIB_4 - 1);
    Ok(TranslatedPage {
        phys_base,
        page_offset,
        page_size: KIB_4,
    })
}

/// Helper function to copy memory from user space using a generic `PageReader`.
///
/// Pre-validates the entire virtual range `[user_vaddr .. user_vaddr + buf.len())` before
/// performing any copy. In Janus's single-core uniprocessor execution model where syscalls run
/// with interrupts masked (SFMask.IF = 0), user address space mappings cannot be concurrently
/// modified between phases. If virtual address validation fails on any page in the range,
/// this function returns immediately without modifying `buf`. If a failure occurs during the
/// physical memory read phase, previously copied bytes in `buf` remain modified.
fn copy_from_user_with_reader<R: PageReader>(
    reader: &R,
    pml4: &PageTable,
    user_vaddr: u64,
    buf: &mut [u8],
) -> Result<(), UserCopyError> {
    if buf.is_empty() {
        return Ok(());
    }
    if user_vaddr == 0 {
        return Err(UserCopyError::NullPointer);
    }

    let end_vaddr = user_vaddr
        .checked_add(buf.len() as u64)
        .ok_or(UserCopyError::ArithmeticOverflow)?;

    if user_vaddr >= KERNEL_SPACE_START || end_vaddr > KERNEL_SPACE_START {
        return Err(UserCopyError::KernelAddress);
    }
    if user_vaddr >= USER_SPACE_END || end_vaddr > USER_SPACE_END {
        return Err(UserCopyError::NonCanonical);
    }

    let total_len = buf.len();

    // Phase 1: Pre-validation.
    // Verify that every page spanned by the requested range is valid, present,
    // and user-accessible before modifying any byte of `buf`.
    let mut check_offset = 0usize;
    while check_offset < total_len {
        let curr_vaddr = user_vaddr
            .checked_add(check_offset as u64)
            .ok_or(UserCopyError::ArithmeticOverflow)?;
        let trans = translate_user_page(reader, pml4, curr_vaddr)?;
        let remaining_in_page = (trans.page_size - trans.page_offset) as usize;
        let chunk_size = core::cmp::min(remaining_in_page, total_len - check_offset);
        check_offset += chunk_size;
    }

    // Phase 2: Copy verified pages into `buf`.
    let mut copied = 0usize;
    while copied < total_len {
        let curr_vaddr = user_vaddr
            .checked_add(copied as u64)
            .ok_or(UserCopyError::ArithmeticOverflow)?;
        let trans = translate_user_page(reader, pml4, curr_vaddr)?;
        let remaining_in_page = (trans.page_size - trans.page_offset) as usize;
        let chunk_size = core::cmp::min(remaining_in_page, total_len - copied);

        let src_phys = trans
            .phys_base
            .checked_add(trans.page_offset)
            .ok_or(UserCopyError::ArithmeticOverflow)?;

        reader.read_bytes(src_phys, &mut buf[copied..copied + chunk_size])?;
        copied += chunk_size;
    }

    Ok(())
}

/// Safely copies `buf.len()` bytes from user virtual address `user_vaddr` in the active address space
/// (from CR3) into kernel buffer `buf`.
///
/// # Failure Guarantees
/// - Pre-validation: All virtual page translations (PML4..PT hierarchy, permissions, and huge-page
///   alignment) are verified across `[user_vaddr .. user_vaddr + buf.len())` before any copy begins.
///   If virtual translation, permissions, or physical address bounds fail on any page in the range,
///   the function returns `Err` immediately without modifying `buf`.
/// - Copy phase: In Janus's single-core uniprocessor model with syscall interrupt masking (`SFMask.IF = 0`),
///   page tables cannot be concurrently modified during execution. However, if a failure occurs during
///   the subsequent physical read phase (e.g. invalid physical memory backing), earlier chunks already
///   copied into `buf` remain modified. This function does not provide transactional roll-back.
pub fn copy_from_user(user_vaddr: u64, buf: &mut [u8]) -> Result<(), UserCopyError> {
    let cr3_frame = x86_64::registers::control::Cr3::read().0;
    let pml4_paddr = cr3_frame.start_address().as_u64();
    let reader = KernelDirectMap;
    let pml4 = reader.read_table(pml4_paddr)?;
    copy_from_user_with_reader(&reader, pml4, user_vaddr, buf)
}

/// Reads a 4-byte `u32` value from the user virtual address in the active address space.
///
/// Uses `copy_from_user` to validate and read the 4-byte sequence.
pub fn copy_user_u32(user_vaddr: u64) -> Result<u32, UserCopyError> {
    let mut bytes = [0u8; 4];
    copy_from_user(user_vaddr, &mut bytes)?;
    Ok(u32::from_ne_bytes(bytes))
}

/// Errors that can occur during dynamic user address-space page mapping and management.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PagingError {
    /// Physical frame allocator ran out of memory.
    OutOfMemory,
    /// Requested virtual address violates kernel higher-half boundary (PML4 index >= 256).
    KernelAddressViolation,
    /// Address is non-canonical or outside the valid lower-half user address space.
    NonCanonicalAddress,
    /// Virtual or physical address is not 4 KiB aligned.
    MisalignedAddress,
    /// Physical address exceeds the direct-mapped memory window (DIRECT_MAP_LIMIT).
    PhysicalAddressOutOfBounds,
    /// Target virtual page is already mapped in the page table.
    AlreadyMapped,
    /// Target virtual page is not mapped.
    NotMapped,
    /// Page table flags violate W^X policy (page is WRITABLE without NO_EXECUTE).
    WxViolation,
    /// A page table entry contains reserved bits or huge-page flags where unpermitted.
    InvalidPageTableEntry,
    /// A user mapping request lacks USER_ACCESSIBLE permission.
    SupervisorOnly,
}

/// Abstract frame allocator interface to enable mock testing with failure injection.
pub trait FrameAllocatorTrait {
    fn allocate_frame(&mut self) -> Option<u64>;
    fn free_frame(&mut self, paddr: u64);
}

impl FrameAllocatorTrait for PhysFrameAllocator {
    fn allocate_frame(&mut self) -> Option<u64> {
        self.allocate_frame()
    }
    fn free_frame(&mut self, paddr: u64) {
        self.free_frame(paddr);
    }
}

/// Abstract interface for accessing, zeroing, and invalidating physical page tables.
pub(crate) trait PageTableAccessor {
    /// Obtains a mutable reference to the `PageTable` located at physical address `paddr`.
    ///
    /// # Safety
    /// Caller must ensure `paddr` is 4 KiB aligned and represents a valid page table frame.
    #[allow(clippy::mut_from_ref)]
    unsafe fn get_table_mut(&self, paddr: u64) -> Result<&mut PageTable, PagingError>;

    /// Zeroes an entire 4096-byte frame at physical address `paddr`.
    ///
    /// # Safety
    /// Caller must ensure `paddr` is 4 KiB aligned and represents a valid physical frame.
    unsafe fn zero_frame(&self, paddr: u64) -> Result<(), PagingError>;

    /// Returns the authoritative physical address of the master kernel root page table (PML4).
    fn kernel_root_pml4(&self) -> u64;

    /// Returns the physical address of the currently active root page table loaded in CPU CR3.
    fn active_pml4(&self) -> u64;

    /// Flushes TLB for a virtual address if the specified `pml4_paddr` is active.
    fn flush_tlb_if_active(&self, pml4_paddr: u64, vaddr: u64);
}

/// Production accessor operating on the direct physical map (`PHYS_MEMORY_OFFSET`).
pub(crate) struct DirectMapAccessor;

impl PageTableAccessor for DirectMapAccessor {
    #[allow(clippy::mut_from_ref)]
    unsafe fn get_table_mut(&self, paddr: u64) -> Result<&mut PageTable, PagingError> {
        if (paddr & 0xFFF) != 0 {
            return Err(PagingError::MisalignedAddress);
        }
        let end_paddr = paddr
            .checked_add(4096)
            .ok_or(PagingError::PhysicalAddressOutOfBounds)?;
        if end_paddr > DIRECT_MAP_LIMIT {
            return Err(PagingError::PhysicalAddressOutOfBounds);
        }
        let vaddr = paddr
            .checked_add(PHYS_MEMORY_OFFSET)
            .ok_or(PagingError::PhysicalAddressOutOfBounds)?;
        Ok(&mut *(vaddr as *mut PageTable))
    }

    unsafe fn zero_frame(&self, paddr: u64) -> Result<(), PagingError> {
        if (paddr & 0xFFF) != 0 {
            return Err(PagingError::MisalignedAddress);
        }
        let end_paddr = paddr
            .checked_add(4096)
            .ok_or(PagingError::PhysicalAddressOutOfBounds)?;
        if end_paddr > DIRECT_MAP_LIMIT {
            return Err(PagingError::PhysicalAddressOutOfBounds);
        }
        let vaddr = paddr
            .checked_add(PHYS_MEMORY_OFFSET)
            .ok_or(PagingError::PhysicalAddressOutOfBounds)?;
        core::ptr::write_bytes(vaddr as *mut u8, 0, 4096);
        Ok(())
    }

    fn kernel_root_pml4(&self) -> u64 {
        kernel_pml4_paddr()
    }

    fn active_pml4(&self) -> u64 {
        #[cfg(all(target_arch = "x86_64", not(test)))]
        {
            x86_64::registers::control::Cr3::read()
                .0
                .start_address()
                .as_u64()
        }
        #[cfg(any(not(target_arch = "x86_64"), test))]
        {
            0
        }
    }

    fn flush_tlb_if_active(&self, pml4_paddr: u64, vaddr: u64) {
        #[cfg(all(target_arch = "x86_64", not(test)))]
        {
            let cr3_frame = x86_64::registers::control::Cr3::read().0;
            if cr3_frame.start_address().as_u64() == pml4_paddr {
                x86_64::instructions::tlb::flush(VirtAddr::new(vaddr));
            }
        }
        #[cfg(any(not(target_arch = "x86_64"), test))]
        {
            let _ = (pml4_paddr, vaddr);
        }
    }
}

/// Creates a new user address space (PML4) by copying kernel higher-half mappings.
///
/// Allocates a new 4 KiB frame for the PML4, zeroes the lower-half entries (0..256),
/// and shallow-copies higher-half entries (256..512) from the active kernel page table.
pub fn create_user_pml4(allocator: &mut PhysFrameAllocator) -> Result<u64, PagingError> {
    let accessor = DirectMapAccessor;
    create_user_pml4_inner(allocator, &accessor)
}

/// Maps a 4 KiB physical leaf frame into a user address space.
///
/// Enforces canonical user bounds, 4 KiB alignment, direct-map bounds, and W^X policy.
/// Intermediate page tables (PDPT, PD, PT) are allocated and zeroed on demand.
/// If an allocation failure occurs midway, all intermediate tables allocated during
/// this call are rolled back cleanly.
pub fn map_user_page(
    pml4_paddr: u64,
    vaddr: u64,
    paddr: u64,
    flags: PageTableFlags,
    allocator: &mut PhysFrameAllocator,
) -> Result<(), PagingError> {
    let accessor = DirectMapAccessor;
    map_user_page_inner(pml4_paddr, vaddr, paddr, flags, allocator, &accessor)
}

/// Unmaps a 4 KiB virtual page from a user address space.
///
/// Removes the leaf PT mapping, invalidates the TLB if the target PML4 is currently active,
/// and reclaims newly empty intermediate page tables.
/// Returns the physical address of the unmapped leaf frame without freeing it.
pub fn unmap_user_page(
    pml4_paddr: u64,
    vaddr: u64,
    allocator: &mut PhysFrameAllocator,
) -> Result<u64, PagingError> {
    let accessor = DirectMapAccessor;
    unmap_user_page_inner(pml4_paddr, vaddr, allocator, &accessor)
}

/// Destroys a user address space, reclaiming all user-space intermediate page tables.
///
/// Traverses only lower-half PML4 entries (0..256). Shared higher-half kernel entries
/// (256..512) and leaf data frames are NEVER modified or freed.
///
/// # Root Protection Policy
/// This function refuses to destroy and safely ignores attempts to destroy:
/// 1. The authoritative master kernel root PML4 (`kernel_pml4_paddr()`), regardless of what is loaded in CR3.
/// 2. The currently active root PML4 loaded in the CPU CR3 register.
pub fn destroy_user_pml4(pml4_paddr: u64, allocator: &mut PhysFrameAllocator) {
    let accessor = DirectMapAccessor;
    destroy_user_pml4_inner(pml4_paddr, allocator, &accessor);
}

pub(crate) fn create_user_pml4_inner<A: FrameAllocatorTrait, M: PageTableAccessor>(
    allocator: &mut A,
    accessor: &M,
) -> Result<u64, PagingError> {
    let pml4_paddr = allocator.allocate_frame().ok_or(PagingError::OutOfMemory)?;
    if let Err(e) = unsafe { accessor.zero_frame(pml4_paddr) } {
        allocator.free_frame(pml4_paddr);
        return Err(e);
    }

    let kernel_pml4_paddr = accessor.kernel_root_pml4();
    if kernel_pml4_paddr != 0 {
        let mut higher_half = [const { PageTableEntry::new() }; 256];
        {
            let kernel_pml4 = match unsafe { accessor.get_table_mut(kernel_pml4_paddr) } {
                Ok(t) => t,
                Err(e) => {
                    allocator.free_frame(pml4_paddr);
                    return Err(e);
                }
            };
            for i in 256..512 {
                higher_half[i - 256] = kernel_pml4[i].clone();
            }
        }
        let new_pml4 = match unsafe { accessor.get_table_mut(pml4_paddr) } {
            Ok(t) => t,
            Err(e) => {
                allocator.free_frame(pml4_paddr);
                return Err(e);
            }
        };
        for i in 256..512 {
            new_pml4[i] = higher_half[i - 256].clone();
        }
    }

    Ok(pml4_paddr)
}

pub(crate) fn map_user_page_inner<A: FrameAllocatorTrait, M: PageTableAccessor>(
    pml4_paddr: u64,
    vaddr: u64,
    paddr: u64,
    flags: PageTableFlags,
    allocator: &mut A,
    accessor: &M,
) -> Result<(), PagingError> {
    // 1. Virtual address bounds & canonical validation
    if vaddr >= KERNEL_SPACE_START {
        return Err(PagingError::KernelAddressViolation);
    }
    if vaddr >= USER_SPACE_END {
        return Err(PagingError::NonCanonicalAddress);
    }
    if (vaddr & 0xFFF) != 0 {
        return Err(PagingError::MisalignedAddress);
    }
    let virt_addr = VirtAddr::new(vaddr);
    let p4_idx = virt_addr.p4_index();
    if u16::from(p4_idx) >= 256 {
        return Err(PagingError::KernelAddressViolation);
    }

    // 2. Physical address bounds & alignment validation
    if (paddr & 0xFFF) != 0 {
        return Err(PagingError::MisalignedAddress);
    }
    let end_paddr = paddr
        .checked_add(4096)
        .ok_or(PagingError::PhysicalAddressOutOfBounds)?;
    if end_paddr > DIRECT_MAP_LIMIT {
        return Err(PagingError::PhysicalAddressOutOfBounds);
    }

    // 3. Flags validation & W^X enforcement
    if !flags.contains(PageTableFlags::PRESENT) || !flags.contains(PageTableFlags::USER_ACCESSIBLE)
    {
        return Err(PagingError::SupervisorOnly);
    }
    if flags.contains(PageTableFlags::HUGE_PAGE) {
        return Err(PagingError::InvalidPageTableEntry);
    }
    if flags.contains(PageTableFlags::WRITABLE) && !flags.contains(PageTableFlags::NO_EXECUTE) {
        return Err(PagingError::WxViolation);
    }

    let p3_idx = virt_addr.p3_index();
    let p2_idx = virt_addr.p2_index();
    let p1_idx = virt_addr.p1_index();

    let mut alloc_pdpt: Option<u64> = None;
    let mut alloc_pd: Option<u64> = None;
    let mut alloc_pt: Option<u64> = None;

    // Traverse / allocate Level 4 -> PDPT
    let pml4 = unsafe { accessor.get_table_mut(pml4_paddr)? };
    let pdpt_paddr = if pml4[p4_idx].flags().contains(PageTableFlags::PRESENT) {
        if pml4[p4_idx].flags().contains(PageTableFlags::HUGE_PAGE) {
            return Err(PagingError::InvalidPageTableEntry);
        }
        pml4[p4_idx].addr().as_u64()
    } else {
        let frame = allocator.allocate_frame().ok_or(PagingError::OutOfMemory)?;
        if let Err(e) = unsafe { accessor.zero_frame(frame) } {
            allocator.free_frame(frame);
            return Err(e);
        }
        pml4[p4_idx].set_addr(
            PhysAddr::new(frame),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE,
        );
        alloc_pdpt = Some(frame);
        frame
    };

    // Traverse / allocate Level 3 -> PD
    let pdpt = match unsafe { accessor.get_table_mut(pdpt_paddr) } {
        Ok(t) => t,
        Err(e) => {
            if let Some(f_pdpt) = alloc_pdpt {
                let pml4 = unsafe { accessor.get_table_mut(pml4_paddr).unwrap() };
                pml4[p4_idx] = PageTableEntry::new();
                allocator.free_frame(f_pdpt);
            }
            return Err(e);
        }
    };
    let pd_paddr = if pdpt[p3_idx].flags().contains(PageTableFlags::PRESENT) {
        if pdpt[p3_idx].flags().contains(PageTableFlags::HUGE_PAGE) {
            if let Some(f_pdpt) = alloc_pdpt {
                let pml4 = unsafe { accessor.get_table_mut(pml4_paddr).unwrap() };
                pml4[p4_idx] = PageTableEntry::new();
                allocator.free_frame(f_pdpt);
            }
            return Err(PagingError::InvalidPageTableEntry);
        }
        pdpt[p3_idx].addr().as_u64()
    } else {
        let frame = match allocator.allocate_frame() {
            Some(f) => f,
            None => {
                if let Some(f_pdpt) = alloc_pdpt {
                    let pml4 = unsafe { accessor.get_table_mut(pml4_paddr).unwrap() };
                    pml4[p4_idx] = PageTableEntry::new();
                    allocator.free_frame(f_pdpt);
                }
                return Err(PagingError::OutOfMemory);
            }
        };
        if let Err(e) = unsafe { accessor.zero_frame(frame) } {
            allocator.free_frame(frame);
            if let Some(f_pdpt) = alloc_pdpt {
                let pml4 = unsafe { accessor.get_table_mut(pml4_paddr).unwrap() };
                pml4[p4_idx] = PageTableEntry::new();
                allocator.free_frame(f_pdpt);
            }
            return Err(e);
        }
        pdpt[p3_idx].set_addr(
            PhysAddr::new(frame),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE,
        );
        alloc_pd = Some(frame);
        frame
    };

    // Traverse / allocate Level 2 -> PT
    let pd = match unsafe { accessor.get_table_mut(pd_paddr) } {
        Ok(t) => t,
        Err(e) => {
            if let Some(f_pd) = alloc_pd {
                let pdpt = unsafe { accessor.get_table_mut(pdpt_paddr).unwrap() };
                pdpt[p3_idx] = PageTableEntry::new();
                allocator.free_frame(f_pd);
            }
            if let Some(f_pdpt) = alloc_pdpt {
                let pml4 = unsafe { accessor.get_table_mut(pml4_paddr).unwrap() };
                pml4[p4_idx] = PageTableEntry::new();
                allocator.free_frame(f_pdpt);
            }
            return Err(e);
        }
    };
    let pt_paddr = if pd[p2_idx].flags().contains(PageTableFlags::PRESENT) {
        if pd[p2_idx].flags().contains(PageTableFlags::HUGE_PAGE) {
            if let Some(f_pd) = alloc_pd {
                let pdpt = unsafe { accessor.get_table_mut(pdpt_paddr).unwrap() };
                pdpt[p3_idx] = PageTableEntry::new();
                allocator.free_frame(f_pd);
            }
            if let Some(f_pdpt) = alloc_pdpt {
                let pml4 = unsafe { accessor.get_table_mut(pml4_paddr).unwrap() };
                pml4[p4_idx] = PageTableEntry::new();
                allocator.free_frame(f_pdpt);
            }
            return Err(PagingError::InvalidPageTableEntry);
        }
        pd[p2_idx].addr().as_u64()
    } else {
        let frame = match allocator.allocate_frame() {
            Some(f) => f,
            None => {
                if let Some(f_pd) = alloc_pd {
                    let pdpt = unsafe { accessor.get_table_mut(pdpt_paddr).unwrap() };
                    pdpt[p3_idx] = PageTableEntry::new();
                    allocator.free_frame(f_pd);
                }
                if let Some(f_pdpt) = alloc_pdpt {
                    let pml4 = unsafe { accessor.get_table_mut(pml4_paddr).unwrap() };
                    pml4[p4_idx] = PageTableEntry::new();
                    allocator.free_frame(f_pdpt);
                }
                return Err(PagingError::OutOfMemory);
            }
        };
        if let Err(e) = unsafe { accessor.zero_frame(frame) } {
            allocator.free_frame(frame);
            if let Some(f_pd) = alloc_pd {
                let pdpt = unsafe { accessor.get_table_mut(pdpt_paddr).unwrap() };
                pdpt[p3_idx] = PageTableEntry::new();
                allocator.free_frame(f_pd);
            }
            if let Some(f_pdpt) = alloc_pdpt {
                let pml4 = unsafe { accessor.get_table_mut(pml4_paddr).unwrap() };
                pml4[p4_idx] = PageTableEntry::new();
                allocator.free_frame(f_pdpt);
            }
            return Err(e);
        }
        pd[p2_idx].set_addr(
            PhysAddr::new(frame),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE,
        );
        alloc_pt = Some(frame);
        frame
    };

    // Traverse Level 1 -> Leaf PT entry
    let pt = match unsafe { accessor.get_table_mut(pt_paddr) } {
        Ok(t) => t,
        Err(e) => {
            if let Some(f_pt) = alloc_pt {
                let pd = unsafe { accessor.get_table_mut(pd_paddr).unwrap() };
                pd[p2_idx] = PageTableEntry::new();
                allocator.free_frame(f_pt);
            }
            if let Some(f_pd) = alloc_pd {
                let pdpt = unsafe { accessor.get_table_mut(pdpt_paddr).unwrap() };
                pdpt[p3_idx] = PageTableEntry::new();
                allocator.free_frame(f_pd);
            }
            if let Some(f_pdpt) = alloc_pdpt {
                let pml4 = unsafe { accessor.get_table_mut(pml4_paddr).unwrap() };
                pml4[p4_idx] = PageTableEntry::new();
                allocator.free_frame(f_pdpt);
            }
            return Err(e);
        }
    };

    if pt[p1_idx].flags().contains(PageTableFlags::PRESENT) {
        // Already mapped: roll back newly allocated tables
        if let Some(f_pt) = alloc_pt {
            let pd = unsafe { accessor.get_table_mut(pd_paddr).unwrap() };
            pd[p2_idx] = PageTableEntry::new();
            allocator.free_frame(f_pt);
        }
        if let Some(f_pd) = alloc_pd {
            let pdpt = unsafe { accessor.get_table_mut(pdpt_paddr).unwrap() };
            pdpt[p3_idx] = PageTableEntry::new();
            allocator.free_frame(f_pd);
        }
        if let Some(f_pdpt) = alloc_pdpt {
            let pml4 = unsafe { accessor.get_table_mut(pml4_paddr).unwrap() };
            pml4[p4_idx] = PageTableEntry::new();
            allocator.free_frame(f_pdpt);
        }
        return Err(PagingError::AlreadyMapped);
    }

    // Commit leaf mapping
    pt[p1_idx].set_addr(PhysAddr::new(paddr), flags);
    accessor.flush_tlb_if_active(pml4_paddr, vaddr);
    Ok(())
}

pub(crate) fn unmap_user_page_inner<A: FrameAllocatorTrait, M: PageTableAccessor>(
    pml4_paddr: u64,
    vaddr: u64,
    allocator: &mut A,
    accessor: &M,
) -> Result<u64, PagingError> {
    if vaddr >= KERNEL_SPACE_START {
        return Err(PagingError::KernelAddressViolation);
    }
    if vaddr >= USER_SPACE_END {
        return Err(PagingError::NonCanonicalAddress);
    }
    if (vaddr & 0xFFF) != 0 {
        return Err(PagingError::MisalignedAddress);
    }
    let virt_addr = VirtAddr::new(vaddr);
    let p4_idx = virt_addr.p4_index();
    if u16::from(p4_idx) >= 256 {
        return Err(PagingError::KernelAddressViolation);
    }

    let p3_idx = virt_addr.p3_index();
    let p2_idx = virt_addr.p2_index();
    let p1_idx = virt_addr.p1_index();

    let pml4 = unsafe { accessor.get_table_mut(pml4_paddr)? };
    if !pml4[p4_idx].flags().contains(PageTableFlags::PRESENT) {
        return Err(PagingError::NotMapped);
    }
    if pml4[p4_idx].flags().contains(PageTableFlags::HUGE_PAGE) {
        return Err(PagingError::InvalidPageTableEntry);
    }
    let pdpt_paddr = pml4[p4_idx].addr().as_u64();

    let pdpt = unsafe { accessor.get_table_mut(pdpt_paddr)? };
    if !pdpt[p3_idx].flags().contains(PageTableFlags::PRESENT) {
        return Err(PagingError::NotMapped);
    }
    if pdpt[p3_idx].flags().contains(PageTableFlags::HUGE_PAGE) {
        return Err(PagingError::InvalidPageTableEntry);
    }
    let pd_paddr = pdpt[p3_idx].addr().as_u64();

    let pd = unsafe { accessor.get_table_mut(pd_paddr)? };
    if !pd[p2_idx].flags().contains(PageTableFlags::PRESENT) {
        return Err(PagingError::NotMapped);
    }
    if pd[p2_idx].flags().contains(PageTableFlags::HUGE_PAGE) {
        return Err(PagingError::InvalidPageTableEntry);
    }
    let pt_paddr = pd[p2_idx].addr().as_u64();

    let pt = unsafe { accessor.get_table_mut(pt_paddr)? };
    if !pt[p1_idx].flags().contains(PageTableFlags::PRESENT) {
        return Err(PagingError::NotMapped);
    }

    let leaf_paddr = pt[p1_idx].addr().as_u64();
    pt[p1_idx] = PageTableEntry::new();

    // TLB invalidation if active
    accessor.flush_tlb_if_active(pml4_paddr, vaddr);

    // Intermediate table reclamation when empty
    let mut pt_empty = true;
    for i in 0..512 {
        if pt[i].flags().contains(PageTableFlags::PRESENT) {
            pt_empty = false;
            break;
        }
    }
    if pt_empty {
        let pd = unsafe { accessor.get_table_mut(pd_paddr)? };
        pd[p2_idx] = PageTableEntry::new();
        allocator.free_frame(pt_paddr);

        let mut pd_empty = true;
        for i in 0..512 {
            if pd[i].flags().contains(PageTableFlags::PRESENT) {
                pd_empty = false;
                break;
            }
        }
        if pd_empty {
            let pdpt = unsafe { accessor.get_table_mut(pdpt_paddr)? };
            pdpt[p3_idx] = PageTableEntry::new();
            allocator.free_frame(pd_paddr);

            let mut pdpt_empty = true;
            for i in 0..512 {
                if pdpt[i].flags().contains(PageTableFlags::PRESENT) {
                    pdpt_empty = false;
                    break;
                }
            }
            if pdpt_empty {
                let pml4 = unsafe { accessor.get_table_mut(pml4_paddr)? };
                pml4[p4_idx] = PageTableEntry::new();
                allocator.free_frame(pdpt_paddr);
            }
        }
    }

    Ok(leaf_paddr)
}

pub(crate) fn destroy_user_pml4_inner<A: FrameAllocatorTrait, M: PageTableAccessor>(
    pml4_paddr: u64,
    allocator: &mut A,
    accessor: &M,
) {
    // Never allow destroying the permanent authoritative kernel root PML4,
    // nor the currently active root loaded in CR3.
    let kernel_root = accessor.kernel_root_pml4();
    let active_root = accessor.active_pml4();
    if (kernel_root != 0 && pml4_paddr == kernel_root)
        || (active_root != 0 && pml4_paddr == active_root)
    {
        return;
    }

    let pml4 = match unsafe { accessor.get_table_mut(pml4_paddr) } {
        Ok(t) => t,
        Err(_) => return,
    };

    // Traverse ONLY user entries 0..256
    for p4_idx in 0..256 {
        if !pml4[p4_idx].flags().contains(PageTableFlags::PRESENT)
            || pml4[p4_idx].flags().contains(PageTableFlags::HUGE_PAGE)
        {
            continue;
        }
        let pdpt_paddr = pml4[p4_idx].addr().as_u64();
        let pdpt = match unsafe { accessor.get_table_mut(pdpt_paddr) } {
            Ok(t) => t,
            Err(_) => continue,
        };

        for p3_idx in 0..512 {
            if !pdpt[p3_idx].flags().contains(PageTableFlags::PRESENT)
                || pdpt[p3_idx].flags().contains(PageTableFlags::HUGE_PAGE)
            {
                continue;
            }
            let pd_paddr = pdpt[p3_idx].addr().as_u64();
            let pd = match unsafe { accessor.get_table_mut(pd_paddr) } {
                Ok(t) => t,
                Err(_) => continue,
            };

            for p2_idx in 0..512 {
                if !pd[p2_idx].flags().contains(PageTableFlags::PRESENT)
                    || pd[p2_idx].flags().contains(PageTableFlags::HUGE_PAGE)
                {
                    continue;
                }
                let pt_paddr = pd[p2_idx].addr().as_u64();
                // PT table frame is freed; leaf frames are NOT freed!
                allocator.free_frame(pt_paddr);
            }
            allocator.free_frame(pd_paddr);
        }
        allocator.free_frame(pdpt_paddr);
    }

    allocator.free_frame(pml4_paddr);
}

#[cfg(test)]
#[allow(clippy::needless_range_loop)]
mod tests {
    use super::*;
    use x86_64::structures::paging::PageTableFlags;

    struct MockPageReader {
        tables: [(u64, PageTable); 8],
        table_count: usize,
        memory: [(u64, [u8; 64]); 8],
        memory_count: usize,
    }

    impl MockPageReader {
        fn new() -> Self {
            Self {
                tables: [
                    (0, PageTable::new()),
                    (0, PageTable::new()),
                    (0, PageTable::new()),
                    (0, PageTable::new()),
                    (0, PageTable::new()),
                    (0, PageTable::new()),
                    (0, PageTable::new()),
                    (0, PageTable::new()),
                ],
                table_count: 0,
                memory: [(0, [0u8; 64]); 8],
                memory_count: 0,
            }
        }

        fn add_table(&mut self, paddr: u64, table: PageTable) {
            assert!(self.table_count < self.tables.len());
            self.tables[self.table_count] = (paddr, table);
            self.table_count += 1;
        }

        fn add_memory(&mut self, paddr: u64, data: [u8; 64]) {
            assert!(self.memory_count < self.memory.len());
            self.memory[self.memory_count] = (paddr, data);
            self.memory_count += 1;
        }
    }

    impl Default for MockPageReader {
        fn default() -> Self {
            Self::new()
        }
    }

    unsafe impl PageReader for MockPageReader {
        fn read_table(&self, paddr: u64) -> Result<&PageTable, UserCopyError> {
            let end_paddr = paddr
                .checked_add(4096)
                .ok_or(UserCopyError::ArithmeticOverflow)?;
            if end_paddr > DIRECT_MAP_LIMIT || (paddr & 0xFFF) != 0 {
                return Err(UserCopyError::InvalidPhysicalAddress);
            }
            for i in 0..self.table_count {
                if self.tables[i].0 == paddr {
                    return Ok(&self.tables[i].1);
                }
            }
            Err(UserCopyError::NotMapped)
        }

        fn read_bytes(&self, paddr: u64, buf: &mut [u8]) -> Result<(), UserCopyError> {
            let len = buf.len() as u64;
            let end_paddr = paddr
                .checked_add(len)
                .ok_or(UserCopyError::ArithmeticOverflow)?;
            if end_paddr > DIRECT_MAP_LIMIT {
                return Err(UserCopyError::InvalidPhysicalAddress);
            }
            for i in 0..self.memory_count {
                let block_start = self.memory[i].0;
                let block_end = block_start
                    .checked_add(64)
                    .ok_or(UserCopyError::ArithmeticOverflow)?;
                if paddr >= block_start && end_paddr <= block_end {
                    let offset = (paddr - block_start) as usize;
                    buf.copy_from_slice(&self.memory[i].1[offset..offset + buf.len()]);
                    return Ok(());
                }
            }
            Err(UserCopyError::NotMapped)
        }
    }

    #[test]
    fn test_copy_null_pointer() {
        let reader = MockPageReader::new();
        let pml4 = PageTable::new();
        let mut buf = [0u8; 4];
        let res = copy_from_user_with_reader(&reader, &pml4, 0, &mut buf);
        assert_eq!(res, Err(UserCopyError::NullPointer));
    }

    #[test]
    fn test_copy_kernel_address() {
        let reader = MockPageReader::new();
        let pml4 = PageTable::new();
        let mut buf = [0u8; 4];
        let res = copy_from_user_with_reader(&reader, &pml4, 0xFFFFFFFF80000000, &mut buf);
        assert_eq!(res, Err(UserCopyError::KernelAddress));
    }

    #[test]
    fn test_copy_direct_map_address() {
        let reader = MockPageReader::new();
        let pml4 = PageTable::new();
        let mut buf = [0u8; 4];
        let res = copy_from_user_with_reader(&reader, &pml4, PHYS_MEMORY_OFFSET, &mut buf);
        assert_eq!(res, Err(UserCopyError::KernelAddress));
    }

    #[test]
    fn test_copy_non_canonical_address() {
        let reader = MockPageReader::new();
        let pml4 = PageTable::new();
        let mut buf = [0u8; 4];
        let res = copy_from_user_with_reader(&reader, &pml4, 0x0000_8000_0000_0000, &mut buf);
        assert_eq!(res, Err(UserCopyError::NonCanonical));
    }

    #[test]
    fn test_copy_arithmetic_overflow() {
        let reader = MockPageReader::new();
        let pml4 = PageTable::new();
        let mut buf = [0u8; 4];
        let res = copy_from_user_with_reader(&reader, &pml4, u64::MAX - 2, &mut buf);
        assert_eq!(res, Err(UserCopyError::ArithmeticOverflow));
    }

    #[test]
    fn test_copy_unmapped_address() {
        let reader = MockPageReader::new();
        let pml4 = PageTable::new();
        let mut buf = [0u8; 4];
        let res = copy_from_user_with_reader(&reader, &pml4, 0x400000, &mut buf);
        assert_eq!(res, Err(UserCopyError::NotMapped));
    }

    #[test]
    fn test_copy_supervisor_only_pml4() {
        let reader = MockPageReader::new();
        let mut pml4 = PageTable::new();
        let vaddr = VirtAddr::new(0x400000);
        pml4[vaddr.p4_index()].set_addr(
            PhysAddr::new(0x1000),
            PageTableFlags::PRESENT, // Missing USER_ACCESSIBLE
        );
        let mut buf = [0u8; 4];
        let res = copy_from_user_with_reader(&reader, &pml4, 0x400000, &mut buf);
        assert_eq!(res, Err(UserCopyError::SupervisorOnly));
    }

    #[test]
    fn test_copy_supervisor_only_pdpt() {
        let mut reader = MockPageReader::new();
        let mut pml4 = PageTable::new();
        let mut pdpt = PageTable::new();
        let vaddr = VirtAddr::new(0x400000);

        let flags_user = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
        pml4[vaddr.p4_index()].set_addr(PhysAddr::new(0x1000), flags_user);
        pdpt[vaddr.p3_index()].set_addr(PhysAddr::new(0x2000), PageTableFlags::PRESENT); // Missing USER_ACCESSIBLE

        reader.add_table(0x1000, pdpt);

        let mut buf = [0u8; 4];
        let res = copy_from_user_with_reader(&reader, &pml4, 0x400000, &mut buf);
        assert_eq!(res, Err(UserCopyError::SupervisorOnly));
    }

    #[test]
    fn test_copy_supervisor_only_pd() {
        let mut reader = MockPageReader::new();
        let mut pml4 = PageTable::new();
        let mut pdpt = PageTable::new();
        let mut pd = PageTable::new();
        let vaddr = VirtAddr::new(0x400000);

        let flags_user = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
        pml4[vaddr.p4_index()].set_addr(PhysAddr::new(0x1000), flags_user);
        pdpt[vaddr.p3_index()].set_addr(PhysAddr::new(0x2000), flags_user);
        pd[vaddr.p2_index()].set_addr(PhysAddr::new(0x3000), PageTableFlags::PRESENT); // Missing USER_ACCESSIBLE

        reader.add_table(0x1000, pdpt);
        reader.add_table(0x2000, pd);

        let mut buf = [0u8; 4];
        let res = copy_from_user_with_reader(&reader, &pml4, 0x400000, &mut buf);
        assert_eq!(res, Err(UserCopyError::SupervisorOnly));
    }

    #[test]
    fn test_copy_supervisor_only_pt() {
        let mut reader = MockPageReader::new();
        let mut pml4 = PageTable::new();
        let mut pdpt = PageTable::new();
        let mut pd = PageTable::new();
        let mut pt = PageTable::new();
        let vaddr = VirtAddr::new(0x400000);

        let flags_user = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
        pml4[vaddr.p4_index()].set_addr(PhysAddr::new(0x1000), flags_user);
        pdpt[vaddr.p3_index()].set_addr(PhysAddr::new(0x2000), flags_user);
        pd[vaddr.p2_index()].set_addr(PhysAddr::new(0x3000), flags_user);
        pt[vaddr.p1_index()].set_addr(PhysAddr::new(0x4000), PageTableFlags::PRESENT); // Missing USER_ACCESSIBLE

        reader.add_table(0x1000, pdpt);
        reader.add_table(0x2000, pd);
        reader.add_table(0x3000, pt);

        let mut buf = [0u8; 4];
        let res = copy_from_user_with_reader(&reader, &pml4, 0x400000, &mut buf);
        assert_eq!(res, Err(UserCopyError::SupervisorOnly));
    }

    #[test]
    fn test_copy_valid_mapped_page() {
        let mut reader = MockPageReader::new();
        let mut pml4 = PageTable::new();
        let mut pdpt = PageTable::new();
        let mut pd = PageTable::new();
        let mut pt = PageTable::new();
        let mut data = [0x42u8; 64];
        data[0] = 0xAA;
        data[1] = 0xBB;
        data[2] = 0xCC;
        data[3] = 0xDD;

        let vaddr = VirtAddr::new(0x400000);
        let flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
        pml4[vaddr.p4_index()].set_addr(PhysAddr::new(0x1000), flags);
        pdpt[vaddr.p3_index()].set_addr(PhysAddr::new(0x2000), flags);
        pd[vaddr.p2_index()].set_addr(PhysAddr::new(0x3000), flags);
        pt[vaddr.p1_index()].set_addr(PhysAddr::new(0x4000), flags);

        reader.add_table(0x1000, pdpt);
        reader.add_table(0x2000, pd);
        reader.add_table(0x3000, pt);
        reader.add_memory(0x4000, data);

        let mut buf = [0u8; 4];
        let res = copy_from_user_with_reader(&reader, &pml4, 0x400000, &mut buf);
        assert_eq!(res, Ok(()));
        assert_eq!(buf, [0xAA, 0xBB, 0xCC, 0xDD]);
    }

    #[test]
    fn test_copy_2mb_huge_page_with_pat() {
        let mut reader = MockPageReader::new();
        let mut pml4 = PageTable::new();
        let mut pdpt = PageTable::new();
        let mut pd = PageTable::new();
        let mut data = [0x55u8; 64];
        data[0] = 0x11;
        data[1] = 0x22;
        data[2] = 0x33;
        data[3] = 0x44;

        let vaddr = VirtAddr::new(0x400000);
        let flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
        pml4[vaddr.p4_index()].set_addr(PhysAddr::new(0x1000), flags);
        pdpt[vaddr.p3_index()].set_addr(PhysAddr::new(0x2000), flags);
        // Base is 0x200000 (2 MiB). In entry, bit 12 (PAT) is set (0x201000).
        pd[vaddr.p2_index()].set_addr(PhysAddr::new(0x201000), flags | PageTableFlags::HUGE_PAGE);

        reader.add_table(0x1000, pdpt);
        reader.add_table(0x2000, pd);
        // Memory is registered at the true physical base 0x200000
        reader.add_memory(0x200000, data);

        let mut buf = [0u8; 4];
        let res = copy_from_user_with_reader(&reader, &pml4, 0x400000, &mut buf);
        assert_eq!(res, Ok(()));
        assert_eq!(buf, [0x11, 0x22, 0x33, 0x44]);
    }

    #[test]
    fn test_copy_2mb_huge_page_reserved_bits() {
        let mut reader = MockPageReader::new();
        let mut pml4 = PageTable::new();
        let mut pdpt = PageTable::new();
        let mut pd = PageTable::new();

        let vaddr = VirtAddr::new(0x400000);
        let flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
        pml4[vaddr.p4_index()].set_addr(PhysAddr::new(0x1000), flags);
        pdpt[vaddr.p3_index()].set_addr(PhysAddr::new(0x2000), flags);
        // Base is 0x200000, but bit 13 is set (0x202000), violating reserved bits!
        pd[vaddr.p2_index()].set_addr(PhysAddr::new(0x202000), flags | PageTableFlags::HUGE_PAGE);

        reader.add_table(0x1000, pdpt);
        reader.add_table(0x2000, pd);

        let mut buf = [0u8; 4];
        let res = copy_from_user_with_reader(&reader, &pml4, 0x400000, &mut buf);
        assert_eq!(res, Err(UserCopyError::InvalidPageTableEntry));
    }

    #[test]
    fn test_copy_1gb_huge_page_with_pat() {
        let mut reader = MockPageReader::new();
        let mut pml4 = PageTable::new();
        let mut pdpt = PageTable::new();
        let mut data = [0x77u8; 64];
        data[0] = 0x55;
        data[1] = 0x66;
        data[2] = 0x77;
        data[3] = 0x88;

        let vaddr = VirtAddr::new(0x40000000); // 1 GiB virtual offset
        let flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
        pml4[vaddr.p4_index()].set_addr(PhysAddr::new(0x1000), flags);
        // Base is 0x40000000 (1 GiB). In entry, bit 12 (PAT) is set (0x40001000).
        pdpt[vaddr.p3_index()]
            .set_addr(PhysAddr::new(0x40001000), flags | PageTableFlags::HUGE_PAGE);

        reader.add_table(0x1000, pdpt);
        reader.add_memory(0x40000000, data);

        let mut buf = [0u8; 4];
        let res = copy_from_user_with_reader(&reader, &pml4, 0x40000000, &mut buf);
        assert_eq!(res, Ok(()));
        assert_eq!(buf, [0x55, 0x66, 0x77, 0x88]);
    }

    #[test]
    fn test_copy_1gb_huge_page_reserved_bits() {
        let mut reader = MockPageReader::new();
        let mut pml4 = PageTable::new();
        let mut pdpt = PageTable::new();

        let vaddr = VirtAddr::new(0x40000000);
        let flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
        pml4[vaddr.p4_index()].set_addr(PhysAddr::new(0x1000), flags);
        // Base is 0x40000000, but bit 14 is set (0x40004000), violating reserved bits!
        pdpt[vaddr.p3_index()]
            .set_addr(PhysAddr::new(0x40004000), flags | PageTableFlags::HUGE_PAGE);

        reader.add_table(0x1000, pdpt);

        let mut buf = [0u8; 4];
        let res = copy_from_user_with_reader(&reader, &pml4, 0x40000000, &mut buf);
        assert_eq!(res, Err(UserCopyError::InvalidPageTableEntry));
    }

    #[test]
    fn test_copy_cross_page_boundary() {
        let mut reader = MockPageReader::new();
        let mut pml4 = PageTable::new();
        let mut pdpt = PageTable::new();
        let mut pd = PageTable::new();
        let mut pt = PageTable::new();
        let page1 = [0xAAu8; 64];
        let page2 = [0xBBu8; 64];

        let vaddr = VirtAddr::new(0x400FFE); // 2 bytes in page 1, 2 bytes in page 2
        let flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
        pml4[vaddr.p4_index()].set_addr(PhysAddr::new(0x1000), flags);
        pdpt[vaddr.p3_index()].set_addr(PhysAddr::new(0x2000), flags);
        pd[vaddr.p2_index()].set_addr(PhysAddr::new(0x3000), flags);
        pt[vaddr.p1_index()].set_addr(PhysAddr::new(0x4000), flags);

        // Map page 2 at 0x401000 -> 0x5000
        let vaddr_next = VirtAddr::new(0x401000);
        pt[vaddr_next.p1_index()].set_addr(PhysAddr::new(0x5000), flags);

        reader.add_table(0x1000, pdpt);
        reader.add_table(0x2000, pd);
        reader.add_table(0x3000, pt);
        // Page 1 ends at 0x4FFF; last 2 bytes are at 0x4FFE..0x5000
        reader.add_memory(0x4FC0, page1);
        reader.add_memory(0x5000, page2);

        let mut buf = [0u8; 4];
        let res = copy_from_user_with_reader(&reader, &pml4, 0x400FFE, &mut buf);
        assert_eq!(res, Ok(()));
        assert_eq!(buf, [0xAA, 0xAA, 0xBB, 0xBB]);
    }

    #[test]
    fn test_copy_cross_page_boundary_atomic_no_partial_mutation() {
        let mut reader = MockPageReader::new();
        let mut pml4 = PageTable::new();
        let mut pdpt = PageTable::new();
        let mut pd = PageTable::new();
        let mut pt = PageTable::new();
        let page1 = [0xAAu8; 64];

        let vaddr = VirtAddr::new(0x400FFE); // 2 bytes in page 1, page 2 unmapped
        let flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
        pml4[vaddr.p4_index()].set_addr(PhysAddr::new(0x1000), flags);
        pdpt[vaddr.p3_index()].set_addr(PhysAddr::new(0x2000), flags);
        pd[vaddr.p2_index()].set_addr(PhysAddr::new(0x3000), flags);
        pt[vaddr.p1_index()].set_addr(PhysAddr::new(0x4000), flags);
        // Note: page 2 at 0x401000 is intentionally left unmapped!

        reader.add_table(0x1000, pdpt);
        reader.add_table(0x2000, pd);
        reader.add_table(0x3000, pt);
        reader.add_memory(0x4FC0, page1);

        let mut buf = [0xEEu8; 4];
        let res = copy_from_user_with_reader(&reader, &pml4, 0x400FFE, &mut buf);
        assert_eq!(res, Err(UserCopyError::NotMapped));
        // Verify atomicity: buf must remain completely unmodified
        assert_eq!(buf, [0xEE, 0xEE, 0xEE, 0xEE]);
    }

    #[test]
    fn test_copy_physical_address_exceeds_direct_map() {
        let mut reader = MockPageReader::new();
        let mut pml4 = PageTable::new();
        let mut pdpt = PageTable::new();
        let mut pd = PageTable::new();
        let mut pt = PageTable::new();

        let vaddr = VirtAddr::new(0x400000);
        let flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
        pml4[vaddr.p4_index()].set_addr(PhysAddr::new(0x1000), flags);
        pdpt[vaddr.p3_index()].set_addr(PhysAddr::new(0x2000), flags);
        pd[vaddr.p2_index()].set_addr(PhysAddr::new(0x3000), flags);
        // 0x1_0000_0000 is 4 GiB, exceeding DIRECT_MAP_LIMIT
        pt[vaddr.p1_index()].set_addr(PhysAddr::new(0x1_0000_0000), flags);

        reader.add_table(0x1000, pdpt);
        reader.add_table(0x2000, pd);
        reader.add_table(0x3000, pt);

        let mut buf = [0u8; 4];
        let res = copy_from_user_with_reader(&reader, &pml4, 0x400000, &mut buf);
        assert_eq!(res, Err(UserCopyError::InvalidPhysicalAddress));
    }

    #[test]
    fn test_reader_unaligned_page_table_address() {
        let reader = MockPageReader::new();
        // 0x1001 is not 4 KiB aligned; read_table must reject it immediately
        assert!(matches!(
            reader.read_table(0x1001),
            Err(UserCopyError::InvalidPhysicalAddress)
        ));
    }

    #[test]
    fn test_copy_pml4_huge_page_reserved() {
        let reader = MockPageReader::new();
        let mut pml4 = PageTable::new();
        let vaddr = VirtAddr::new(0x400000);
        let flags =
            PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE | PageTableFlags::HUGE_PAGE;
        // Level 4 PML4 does not support HUGE_PAGE; setting bit 7 is an architectural violation
        pml4[vaddr.p4_index()].set_addr(PhysAddr::new(0x1000), flags);

        let mut buf = [0u8; 4];
        let res = copy_from_user_with_reader(&reader, &pml4, 0x400000, &mut buf);
        assert_eq!(res, Err(UserCopyError::InvalidPageTableEntry));
    }

    // =========================================================================
    // Stage 3.0 Unit Test Infrastructure & Acceptance Tests
    // =========================================================================

    struct MockFrameAllocator {
        free_frames: [u64; 64],
        head: usize,
        freed_frames: [u64; 64],
        freed_count: usize,
        fail_after: Option<usize>,
        alloc_count: usize,
    }

    impl MockFrameAllocator {
        fn new() -> Self {
            let mut free_frames = [0u64; 64];
            for i in 0..64 {
                free_frames[i] = 0x1000 * (i as u64 + 1); // 0x1000, 0x2000, ...
            }
            Self {
                free_frames,
                head: 64,
                freed_frames: [0u64; 64],
                freed_count: 0,
                fail_after: None,
                alloc_count: 0,
            }
        }

        fn with_fail_after(limit: usize) -> Self {
            let mut a = Self::new();
            a.fail_after = Some(limit);
            a
        }

        fn net_allocated(&self) -> usize {
            self.alloc_count - self.freed_count
        }

        fn alloc_count(&self) -> usize {
            self.alloc_count
        }

        fn was_freed(&self, paddr: u64) -> bool {
            for i in 0..self.freed_count {
                if self.freed_frames[i] == paddr {
                    return true;
                }
            }
            false
        }
    }

    impl FrameAllocatorTrait for MockFrameAllocator {
        fn allocate_frame(&mut self) -> Option<u64> {
            if let Some(limit) = self.fail_after {
                if self.alloc_count >= limit {
                    return None;
                }
            }
            if self.head > 0 {
                self.head -= 1;
                let frame = self.free_frames[self.head];
                self.alloc_count += 1;
                Some(frame)
            } else {
                None
            }
        }

        fn free_frame(&mut self, paddr: u64) {
            assert!(self.freed_count < self.freed_frames.len());
            self.freed_frames[self.freed_count] = paddr;
            self.freed_count += 1;
            assert!(self.head < self.free_frames.len());
            self.free_frames[self.head] = paddr;
            self.head += 1;
        }
    }

    const EMPTY_TABLE_ENTRY: (u64, PageTable) = (0, PageTable::new());

    struct MockPageTableAccessor {
        tables: core::cell::UnsafeCell<[(u64, PageTable); 16]>,
        zeroed: core::cell::UnsafeCell<[bool; 16]>,
        table_count: core::cell::UnsafeCell<usize>,
        kernel_pml4_val: core::cell::Cell<u64>,
        active_pml4_val: core::cell::Cell<u64>,
        tlb_flush_count: core::cell::Cell<usize>,
        last_flushed_vaddr: core::cell::Cell<Option<u64>>,
        fail_table_access_for: core::cell::Cell<Option<u64>>,
    }

    impl MockPageTableAccessor {
        fn new() -> Self {
            Self {
                tables: core::cell::UnsafeCell::new([EMPTY_TABLE_ENTRY; 16]),
                zeroed: core::cell::UnsafeCell::new([false; 16]),
                table_count: core::cell::UnsafeCell::new(0),
                kernel_pml4_val: core::cell::Cell::new(0),
                active_pml4_val: core::cell::Cell::new(0),
                tlb_flush_count: core::cell::Cell::new(0),
                last_flushed_vaddr: core::cell::Cell::new(None),
                fail_table_access_for: core::cell::Cell::new(None),
            }
        }

        fn set_kernel_pml4(&self, paddr: u64, table: PageTable) {
            self.kernel_pml4_val.set(paddr);
            self.add_table(paddr, table);
        }

        fn set_active_pml4(&self, paddr: u64) {
            self.active_pml4_val.set(paddr);
        }

        fn set_fail_table_access_for(&self, paddr: u64) {
            self.fail_table_access_for.set(Some(paddr));
        }

        fn tlb_flush_count(&self) -> usize {
            self.tlb_flush_count.get()
        }

        fn last_flushed_vaddr(&self) -> Option<u64> {
            self.last_flushed_vaddr.get()
        }

        fn add_table(&self, paddr: u64, table: PageTable) {
            let tables = unsafe { &mut *self.tables.get() };
            let count = unsafe { &mut *self.table_count.get() };
            for i in 0..*count {
                if tables[i].0 == paddr {
                    tables[i].1 = table;
                    return;
                }
            }
            assert!(*count < tables.len());
            tables[*count] = (paddr, table);
            *count += 1;
        }

        fn is_zeroed(&self, paddr: u64) -> bool {
            let tables = unsafe { &*self.tables.get() };
            let zeroed = unsafe { &*self.zeroed.get() };
            let count = unsafe { *self.table_count.get() };
            for i in 0..count {
                if tables[i].0 == paddr {
                    return zeroed[i];
                }
            }
            false
        }

        fn table_count(&self) -> usize {
            unsafe { *self.table_count.get() }
        }

        fn table_paddr(&self, idx: usize) -> u64 {
            let tables = unsafe { &*self.tables.get() };
            tables[idx].0
        }
    }

    impl PageTableAccessor for MockPageTableAccessor {
        #[allow(clippy::mut_from_ref)]
        unsafe fn get_table_mut(&self, paddr: u64) -> Result<&mut PageTable, PagingError> {
            if (paddr & 0xFFF) != 0 {
                return Err(PagingError::MisalignedAddress);
            }
            if let Some(target) = self.fail_table_access_for.get() {
                if target == paddr {
                    return Err(PagingError::PhysicalAddressOutOfBounds);
                }
            }
            let tables = &mut *self.tables.get();
            let count = &mut *self.table_count.get();
            for i in 0..*count {
                if tables[i].0 == paddr {
                    return Ok(&mut tables[i].1);
                }
            }
            if *count < tables.len() {
                let idx = *count;
                tables[idx] = (paddr, PageTable::new());
                *count += 1;
                return Ok(&mut tables[idx].1);
            }
            Err(PagingError::OutOfMemory)
        }

        unsafe fn zero_frame(&self, paddr: u64) -> Result<(), PagingError> {
            if (paddr & 0xFFF) != 0 {
                return Err(PagingError::MisalignedAddress);
            }
            let tables = &mut *self.tables.get();
            let zeroed = &mut *self.zeroed.get();
            let count = &mut *self.table_count.get();
            for i in 0..*count {
                if tables[i].0 == paddr {
                    tables[i].1 = PageTable::new();
                    zeroed[i] = true;
                    return Ok(());
                }
            }
            if *count < tables.len() {
                let idx = *count;
                tables[idx] = (paddr, PageTable::new());
                zeroed[idx] = true;
                *count += 1;
                return Ok(());
            }
            Err(PagingError::OutOfMemory)
        }

        fn kernel_root_pml4(&self) -> u64 {
            self.kernel_pml4_val.get()
        }

        fn active_pml4(&self) -> u64 {
            self.active_pml4_val.get()
        }

        fn flush_tlb_if_active(&self, pml4_paddr: u64, vaddr: u64) {
            if self.active_pml4_val.get() == pml4_paddr {
                self.tlb_flush_count.set(self.tlb_flush_count.get() + 1);
                self.last_flushed_vaddr.set(Some(vaddr));
            }
        }
    }

    // 1. Every new page-table frame is zeroed before linking
    #[test]
    fn test_page_table_frames_zeroed_before_linking() {
        let mut allocator = MockFrameAllocator::new();
        let accessor = MockPageTableAccessor::new();

        let pml4 = create_user_pml4_inner(&mut allocator, &accessor).unwrap();
        assert!(accessor.is_zeroed(pml4));

        let flags = PageTableFlags::PRESENT
            | PageTableFlags::USER_ACCESSIBLE
            | PageTableFlags::WRITABLE
            | PageTableFlags::NO_EXECUTE;
        map_user_page_inner(pml4, 0x400000, 0x8000, flags, &mut allocator, &accessor).unwrap();

        // PDPT, PD, and PT frames were allocated; verify every one was zeroed
        for i in 0..accessor.table_count() {
            let paddr = accessor.table_paddr(i);
            if paddr != 0x8000 {
                // Table frames must be zeroed
                assert!(accessor.is_zeroed(paddr));
            }
        }
    }

    // 2. Mapping an already-present leaf returns AlreadyMapped
    #[test]
    fn test_map_already_mapped_returns_error() {
        let mut allocator = MockFrameAllocator::new();
        let accessor = MockPageTableAccessor::new();

        let pml4 = create_user_pml4_inner(&mut allocator, &accessor).unwrap();
        let flags = PageTableFlags::PRESENT
            | PageTableFlags::USER_ACCESSIBLE
            | PageTableFlags::WRITABLE
            | PageTableFlags::NO_EXECUTE;

        map_user_page_inner(pml4, 0x400000, 0x8000, flags, &mut allocator, &accessor).unwrap();

        let res = map_user_page_inner(pml4, 0x400000, 0x9000, flags, &mut allocator, &accessor);
        assert_eq!(res, Err(PagingError::AlreadyMapped));
    }

    // 3. Unmapping does not free leaf frames
    #[test]
    fn test_unmap_does_not_free_leaf_frame() {
        let mut allocator = MockFrameAllocator::new();
        let accessor = MockPageTableAccessor::new();

        let pml4 = create_user_pml4_inner(&mut allocator, &accessor).unwrap();
        let leaf_paddr = 0x8000;
        let flags = PageTableFlags::PRESENT
            | PageTableFlags::USER_ACCESSIBLE
            | PageTableFlags::WRITABLE
            | PageTableFlags::NO_EXECUTE;

        map_user_page_inner(pml4, 0x400000, leaf_paddr, flags, &mut allocator, &accessor).unwrap();

        let unmapped = unmap_user_page_inner(pml4, 0x400000, &mut allocator, &accessor).unwrap();
        assert_eq!(unmapped, leaf_paddr);

        // Allocator MUST NOT have freed the leaf frame
        assert!(!allocator.was_freed(leaf_paddr));
    }

    // 4. Address-space destruction does not free leaf frames
    #[test]
    fn test_destroy_user_pml4_does_not_free_leaf_frames() {
        let mut allocator = MockFrameAllocator::new();
        let accessor = MockPageTableAccessor::new();

        let pml4 = create_user_pml4_inner(&mut allocator, &accessor).unwrap();
        let leaf1 = 0x8000;
        let leaf2 = 0x9000;
        let flags = PageTableFlags::PRESENT
            | PageTableFlags::USER_ACCESSIBLE
            | PageTableFlags::WRITABLE
            | PageTableFlags::NO_EXECUTE;

        map_user_page_inner(pml4, 0x400000, leaf1, flags, &mut allocator, &accessor).unwrap();
        map_user_page_inner(pml4, 0x401000, leaf2, flags, &mut allocator, &accessor).unwrap();

        destroy_user_pml4_inner(pml4, &mut allocator, &accessor);

        // Table frames were freed, but leaf frames must NOT be freed!
        assert!(!allocator.was_freed(leaf1));
        assert!(!allocator.was_freed(leaf2));
    }

    // 5. Shared leaf frame remains mapped in another address space after one mapping is removed
    #[test]
    fn test_shared_leaf_frames_survive_independent_unmap() {
        let mut allocator = MockFrameAllocator::new();
        let accessor = MockPageTableAccessor::new();

        let pml4_a = create_user_pml4_inner(&mut allocator, &accessor).unwrap();
        let pml4_b = create_user_pml4_inner(&mut allocator, &accessor).unwrap();
        let shared_leaf = 0x8000;
        let flags = PageTableFlags::PRESENT
            | PageTableFlags::USER_ACCESSIBLE
            | PageTableFlags::WRITABLE
            | PageTableFlags::NO_EXECUTE;

        map_user_page_inner(
            pml4_a,
            0x400000,
            shared_leaf,
            flags,
            &mut allocator,
            &accessor,
        )
        .unwrap();
        map_user_page_inner(
            pml4_b,
            0x500000,
            shared_leaf,
            flags,
            &mut allocator,
            &accessor,
        )
        .unwrap();

        // Unmap from PML4 A
        let unmapped = unmap_user_page_inner(pml4_a, 0x400000, &mut allocator, &accessor).unwrap();
        assert_eq!(unmapped, shared_leaf);

        // In PML4 B, the mapping must still be present and valid
        let pt_b = unmap_user_page_inner(pml4_b, 0x500000, &mut allocator, &accessor).unwrap();
        assert_eq!(pt_b, shared_leaf);
    }

    // 6. Failure at each intermediate allocation point rolls back all newly allocated tables
    #[test]
    fn test_failure_at_intermediate_allocation_rolls_back() {
        let flags = PageTableFlags::PRESENT
            | PageTableFlags::USER_ACCESSIBLE
            | PageTableFlags::WRITABLE
            | PageTableFlags::NO_EXECUTE;

        // Test fail at PDPT: fail_after = 1 (PML4 allocated, PDPT allocation fails)
        {
            let mut allocator = MockFrameAllocator::with_fail_after(1);
            let accessor = MockPageTableAccessor::new();
            let pml4 = create_user_pml4_inner(&mut allocator, &accessor).unwrap();
            let res = map_user_page_inner(pml4, 0x400000, 0x8000, flags, &mut allocator, &accessor);
            assert_eq!(res, Err(PagingError::OutOfMemory));
            assert_eq!(allocator.net_allocated(), 1); // Only PML4 remains
        }

        // Test fail at PD: fail_after = 2 (PML4 allocated, PDPT allocated, PD allocation fails)
        {
            let mut allocator = MockFrameAllocator::with_fail_after(2);
            let accessor = MockPageTableAccessor::new();
            let pml4 = create_user_pml4_inner(&mut allocator, &accessor).unwrap();
            let res = map_user_page_inner(pml4, 0x400000, 0x8000, flags, &mut allocator, &accessor);
            assert_eq!(res, Err(PagingError::OutOfMemory));
            assert_eq!(allocator.net_allocated(), 1); // PDPT was rolled back; only PML4 remains
        }

        // Test fail at PT: fail_after = 3 (PML4 allocated, PDPT allocated, PD allocated, PT allocation fails)
        {
            let mut allocator = MockFrameAllocator::with_fail_after(3);
            let accessor = MockPageTableAccessor::new();
            let pml4 = create_user_pml4_inner(&mut allocator, &accessor).unwrap();
            let res = map_user_page_inner(pml4, 0x400000, 0x8000, flags, &mut allocator, &accessor);
            assert_eq!(res, Err(PagingError::OutOfMemory));
            assert_eq!(allocator.net_allocated(), 1); // PDPT and PD were rolled back; only PML4 remains
        }
    }

    // 7. Failed mapping preserves pre-existing page-table structures
    #[test]
    fn test_failed_mapping_preserves_existing_page_tables() {
        let flags = PageTableFlags::PRESENT
            | PageTableFlags::USER_ACCESSIBLE
            | PageTableFlags::WRITABLE
            | PageTableFlags::NO_EXECUTE;

        let mut allocator = MockFrameAllocator::new();
        let accessor = MockPageTableAccessor::new();
        let pml4 = create_user_pml4_inner(&mut allocator, &accessor).unwrap();

        // Successfully map page 1 (allocates PDPT, PD, PT)
        map_user_page_inner(pml4, 0x400000, 0x8000, flags, &mut allocator, &accessor).unwrap();
        let baseline_allocated = allocator.net_allocated();

        // Now set fail_after to current alloc count to fail during next mapping
        allocator.fail_after = Some(allocator.alloc_count());

        // Attempting to map page in a DIFFERENT PDPT entry requires allocating a new PDPT entry / PD
        // which fails!
        let res = map_user_page_inner(pml4, 0x4000_0000, 0x9000, flags, &mut allocator, &accessor);
        assert_eq!(res, Err(PagingError::OutOfMemory));
        assert_eq!(allocator.net_allocated(), baseline_allocated);

        // The pre-existing page 1 must remain intact and accessible!
        let unmapped = unmap_user_page_inner(pml4, 0x400000, &mut allocator, &accessor).unwrap();
        assert_eq!(unmapped, 0x8000);
    }

    // 8. Kernel-half and noncanonical virtual addresses are rejected
    #[test]
    fn test_kernel_half_and_noncanonical_addresses_rejected() {
        let mut allocator = MockFrameAllocator::new();
        let accessor = MockPageTableAccessor::new();
        let pml4 = create_user_pml4_inner(&mut allocator, &accessor).unwrap();
        let flags = PageTableFlags::PRESENT
            | PageTableFlags::USER_ACCESSIBLE
            | PageTableFlags::WRITABLE
            | PageTableFlags::NO_EXECUTE;

        // Kernel-space address
        let res1 = map_user_page_inner(
            pml4,
            0xFFFF_8000_0000_0000,
            0x8000,
            flags,
            &mut allocator,
            &accessor,
        );
        assert_eq!(res1, Err(PagingError::KernelAddressViolation));

        // Non-canonical address
        let res2 = map_user_page_inner(
            pml4,
            0x0000_8000_0000_0000,
            0x8000,
            flags,
            &mut allocator,
            &accessor,
        );
        assert_eq!(res2, Err(PagingError::NonCanonicalAddress));

        // PML4 index >= 256
        let res3 = map_user_page_inner(
            pml4,
            0x0000_8000_0000_0000,
            0x8000,
            flags,
            &mut allocator,
            &accessor,
        );
        assert_eq!(res3, Err(PagingError::NonCanonicalAddress));
    }

    // 9. User mappings cannot obtain supervisor access
    #[test]
    fn test_user_mappings_cannot_obtain_supervisor_access() {
        let mut allocator = MockFrameAllocator::new();
        let accessor = MockPageTableAccessor::new();
        let pml4 = create_user_pml4_inner(&mut allocator, &accessor).unwrap();

        // Missing USER_ACCESSIBLE
        let flags_supervisor =
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE;
        let res = map_user_page_inner(
            pml4,
            0x400000,
            0x8000,
            flags_supervisor,
            &mut allocator,
            &accessor,
        );
        assert_eq!(res, Err(PagingError::SupervisorOnly));

        // Missing PRESENT
        let flags_not_present = PageTableFlags::USER_ACCESSIBLE | PageTableFlags::WRITABLE;
        let res2 = map_user_page_inner(
            pml4,
            0x400000,
            0x8000,
            flags_not_present,
            &mut allocator,
            &accessor,
        );
        assert_eq!(res2, Err(PagingError::SupervisorOnly));
    }

    // 10. Virtual W^X violations are rejected
    #[test]
    fn test_virtual_wx_violations_rejected() {
        let mut allocator = MockFrameAllocator::new();
        let accessor = MockPageTableAccessor::new();
        let pml4 = create_user_pml4_inner(&mut allocator, &accessor).unwrap();

        // WRITABLE without NO_EXECUTE
        let flags_wx =
            PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE | PageTableFlags::WRITABLE;
        let res = map_user_page_inner(pml4, 0x400000, 0x8000, flags_wx, &mut allocator, &accessor);
        assert_eq!(res, Err(PagingError::WxViolation));
    }

    // 11. Teardown preserves all higher-half kernel entries and their referenced structures
    #[test]
    fn test_teardown_preserves_higher_half_kernel_entries() {
        let mut allocator = MockFrameAllocator::new();
        let accessor = MockPageTableAccessor::new();

        // Create a mock kernel root PML4 with higher-half entries
        let kernel_pml4_paddr = 0xF000;
        let mut kernel_pml4 = PageTable::new();
        for i in 256..512 {
            let mut entry = PageTableEntry::new();
            entry.set_addr(
                PhysAddr::new(0x1000 * i as u64),
                PageTableFlags::PRESENT | PageTableFlags::WRITABLE,
            );
            kernel_pml4[i] = entry;
        }
        accessor.set_kernel_pml4(kernel_pml4_paddr, kernel_pml4);

        // Create user PML4 (copies higher half)
        let user_pml4 = create_user_pml4_inner(&mut allocator, &accessor).unwrap();

        // Map some user pages
        let flags = PageTableFlags::PRESENT
            | PageTableFlags::USER_ACCESSIBLE
            | PageTableFlags::WRITABLE
            | PageTableFlags::NO_EXECUTE;
        map_user_page_inner(
            user_pml4,
            0x400000,
            0x8000,
            flags,
            &mut allocator,
            &accessor,
        )
        .unwrap();

        // Teardown user PML4
        destroy_user_pml4_inner(user_pml4, &mut allocator, &accessor);

        // Verify kernel root PML4 is completely untouched!
        let kernel_table = unsafe { accessor.get_table_mut(kernel_pml4_paddr).unwrap() };
        for i in 256..512 {
            assert!(kernel_table[i].flags().contains(PageTableFlags::PRESENT));
            assert_eq!(kernel_table[i].addr().as_u64(), 0x1000 * i as u64);
        }
        // Kernel root PML4 was not freed
        assert!(!allocator.was_freed(kernel_pml4_paddr));
    }

    // 12. Repeated map/unmap cycles do not leak intermediate page-table frames
    #[test]
    fn test_repeated_map_unmap_no_leaks() {
        let mut allocator = MockFrameAllocator::new();
        let accessor = MockPageTableAccessor::new();

        let pml4 = create_user_pml4_inner(&mut allocator, &accessor).unwrap();
        let baseline_allocated = allocator.net_allocated(); // 1 (PML4)

        let flags = PageTableFlags::PRESENT
            | PageTableFlags::USER_ACCESSIBLE
            | PageTableFlags::WRITABLE
            | PageTableFlags::NO_EXECUTE;

        for _ in 0..10 {
            map_user_page_inner(pml4, 0x400000, 0x8000, flags, &mut allocator, &accessor).unwrap();
            let unmapped =
                unmap_user_page_inner(pml4, 0x400000, &mut allocator, &accessor).unwrap();
            assert_eq!(unmapped, 0x8000);
            // Intermediate tables (PT, PD, PDPT) were reclaimed on empty!
            assert_eq!(allocator.net_allocated(), baseline_allocated);
        }
    }

    // 13. Unaligned addresses are rejected
    #[test]
    fn test_unaligned_addresses_rejected() {
        let mut allocator = MockFrameAllocator::new();
        let accessor = MockPageTableAccessor::new();
        let pml4 = create_user_pml4_inner(&mut allocator, &accessor).unwrap();
        let flags = PageTableFlags::PRESENT
            | PageTableFlags::USER_ACCESSIBLE
            | PageTableFlags::WRITABLE
            | PageTableFlags::NO_EXECUTE;

        // Unaligned vaddr
        let res1 = map_user_page_inner(pml4, 0x400001, 0x8000, flags, &mut allocator, &accessor);
        assert_eq!(res1, Err(PagingError::MisalignedAddress));

        // Unaligned paddr
        let res2 = map_user_page_inner(pml4, 0x400000, 0x8001, flags, &mut allocator, &accessor);
        assert_eq!(res2, Err(PagingError::MisalignedAddress));

        // Unaligned unmap vaddr
        let res3 = unmap_user_page_inner(pml4, 0x400001, &mut allocator, &accessor);
        assert_eq!(res3, Err(PagingError::MisalignedAddress));
    }

    // 14. Physical address direct-map limit check
    #[test]
    fn test_physical_address_direct_map_limit() {
        let mut allocator = MockFrameAllocator::new();
        let accessor = MockPageTableAccessor::new();
        let pml4 = create_user_pml4_inner(&mut allocator, &accessor).unwrap();
        let flags = PageTableFlags::PRESENT
            | PageTableFlags::USER_ACCESSIBLE
            | PageTableFlags::WRITABLE
            | PageTableFlags::NO_EXECUTE;

        // 0x1_0000_0000 is 4 GiB (exceeds DIRECT_MAP_LIMIT)
        let res = map_user_page_inner(
            pml4,
            0x400000,
            0x1_0000_0000,
            flags,
            &mut allocator,
            &accessor,
        );
        assert_eq!(res, Err(PagingError::PhysicalAddressOutOfBounds));
    }

    // 15. Empty address space destruction
    #[test]
    fn test_empty_address_space_destruction() {
        let mut allocator = MockFrameAllocator::new();
        let accessor = MockPageTableAccessor::new();

        let pml4 = create_user_pml4_inner(&mut allocator, &accessor).unwrap();
        assert_eq!(allocator.net_allocated(), 1);

        destroy_user_pml4_inner(pml4, &mut allocator, &accessor);
        assert_eq!(allocator.net_allocated(), 0);
    }

    // 16. Unmapping missing intermediate entries returns NotMapped
    #[test]
    fn test_unmap_missing_intermediate_entries() {
        let mut allocator = MockFrameAllocator::new();
        let accessor = MockPageTableAccessor::new();

        let pml4 = create_user_pml4_inner(&mut allocator, &accessor).unwrap();
        let res = unmap_user_page_inner(pml4, 0x400000, &mut allocator, &accessor);
        assert_eq!(res, Err(PagingError::NotMapped));
    }

    // 17. Huge page encounters return InvalidPageTableEntry
    #[test]
    fn test_huge_page_encounters_during_map_and_unmap() {
        let mut allocator = MockFrameAllocator::new();
        let accessor = MockPageTableAccessor::new();

        let pml4 = create_user_pml4_inner(&mut allocator, &accessor).unwrap();
        let flags_huge =
            PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE | PageTableFlags::HUGE_PAGE;

        // Passing HUGE_PAGE flag to map_user_page is rejected
        let res = map_user_page_inner(
            pml4,
            0x400000,
            0x8000,
            flags_huge,
            &mut allocator,
            &accessor,
        );
        assert_eq!(res, Err(PagingError::InvalidPageTableEntry));
    }

    // 18. Allocation failure during create_user_pml4
    #[test]
    fn test_allocation_failure_during_create_user_pml4() {
        let mut allocator = MockFrameAllocator::with_fail_after(0); // Fails immediately
        let accessor = MockPageTableAccessor::new();

        let res = create_user_pml4_inner(&mut allocator, &accessor);
        assert_eq!(res, Err(PagingError::OutOfMemory));
        assert_eq!(allocator.net_allocated(), 0);
    }

    // 19. Multiple mappings in independent address spaces
    #[test]
    fn test_multiple_mappings_in_independent_address_spaces() {
        let mut allocator = MockFrameAllocator::new();
        let accessor = MockPageTableAccessor::new();

        let pml4_a = create_user_pml4_inner(&mut allocator, &accessor).unwrap();
        let pml4_b = create_user_pml4_inner(&mut allocator, &accessor).unwrap();

        let flags = PageTableFlags::PRESENT
            | PageTableFlags::USER_ACCESSIBLE
            | PageTableFlags::WRITABLE
            | PageTableFlags::NO_EXECUTE;

        // Same virtual address mapped to different physical frames in independent PML4s
        map_user_page_inner(pml4_a, 0x400000, 0x8000, flags, &mut allocator, &accessor).unwrap();
        map_user_page_inner(pml4_b, 0x400000, 0x9000, flags, &mut allocator, &accessor).unwrap();

        let unmap_a = unmap_user_page_inner(pml4_a, 0x400000, &mut allocator, &accessor).unwrap();
        let unmap_b = unmap_user_page_inner(pml4_b, 0x400000, &mut allocator, &accessor).unwrap();

        assert_eq!(unmap_a, 0x8000);
        assert_eq!(unmap_b, 0x9000);
    }

    // =========================================================================
    // Stage 3.0 Audit Remediation Tests (Findings 1, 2, 3)
    // =========================================================================

    // 20. Finding 2: TLB invalidation occurs only upon successful mapping in the active address space
    #[test]
    fn test_map_user_page_triggers_tlb_invalidation_only_on_success_in_active_address_space() {
        let mut allocator = MockFrameAllocator::new();
        let accessor = MockPageTableAccessor::new();

        let active_pml4 = create_user_pml4_inner(&mut allocator, &accessor).unwrap();
        let inactive_pml4 = create_user_pml4_inner(&mut allocator, &accessor).unwrap();

        // Mark active_pml4 as loaded in CR3
        accessor.set_active_pml4(active_pml4);
        assert_eq!(accessor.tlb_flush_count(), 0);

        let flags = PageTableFlags::PRESENT
            | PageTableFlags::USER_ACCESSIBLE
            | PageTableFlags::WRITABLE
            | PageTableFlags::NO_EXECUTE;

        // 1. Successful mapping into active address space MUST trigger TLB flush
        map_user_page_inner(
            active_pml4,
            0x400000,
            0x8000,
            flags,
            &mut allocator,
            &accessor,
        )
        .unwrap();
        assert_eq!(accessor.tlb_flush_count(), 1);
        assert_eq!(accessor.last_flushed_vaddr(), Some(0x400000));

        // 2. Failed mapping (AlreadyMapped) MUST NOT trigger TLB flush
        let res_fail = map_user_page_inner(
            active_pml4,
            0x400000,
            0x9000,
            flags,
            &mut allocator,
            &accessor,
        );
        assert_eq!(res_fail, Err(PagingError::AlreadyMapped));
        assert_eq!(accessor.tlb_flush_count(), 1); // Count unchanged

        // 3. Successful mapping into INACTIVE address space MUST NOT trigger TLB flush
        map_user_page_inner(
            inactive_pml4,
            0x400000,
            0x9000,
            flags,
            &mut allocator,
            &accessor,
        )
        .unwrap();
        assert_eq!(accessor.tlb_flush_count(), 1); // Count still unchanged (CR3 reload contract)
    }

    // 21. Finding 3: create_user_pml4 frees allocated root frame if higher-half copy fails
    #[test]
    fn test_create_user_pml4_frees_root_frame_on_higher_half_copy_failure() {
        let mut allocator = MockFrameAllocator::new();
        let accessor = MockPageTableAccessor::new();

        let kernel_pml4_paddr = 0xF000;
        let kernel_pml4 = PageTable::new();
        accessor.set_kernel_pml4(kernel_pml4_paddr, kernel_pml4);

        let baseline_allocated = allocator.net_allocated();

        // Inject table access failure for the kernel source PML4 during higher-half copying
        accessor.set_fail_table_access_for(kernel_pml4_paddr);

        let res = create_user_pml4_inner(&mut allocator, &accessor);
        assert_eq!(res, Err(PagingError::PhysicalAddressOutOfBounds));

        // The newly allocated user PML4 frame must have been freed back to allocator; no leaks!
        assert_eq!(allocator.net_allocated(), baseline_allocated);
    }

    // 22. Finding 1: destroy_user_pml4 protects authoritative kernel root regardless of current CR3
    #[test]
    fn test_destroy_user_pml4_protects_authoritative_kernel_root_even_when_cr3_differs() {
        let mut allocator = MockFrameAllocator::new();
        let accessor = MockPageTableAccessor::new();

        let kernel_pml4_paddr = 0xF000;
        let kernel_pml4 = PageTable::new();
        accessor.set_kernel_pml4(kernel_pml4_paddr, kernel_pml4);

        // Simulate active CR3 pointing to a user task address space (0x2000)
        accessor.set_active_pml4(0x2000);

        // Attempting to destroy authoritative kernel root must be rejected
        destroy_user_pml4_inner(kernel_pml4_paddr, &mut allocator, &accessor);

        // Kernel root frame must NOT be freed!
        assert!(!allocator.was_freed(kernel_pml4_paddr));
    }

    // 23. Finding 1: destroy_user_pml4 protects currently active root in CR3
    #[test]
    fn test_destroy_user_pml4_protects_currently_active_root() {
        let mut allocator = MockFrameAllocator::new();
        let accessor = MockPageTableAccessor::new();

        let active_user_pml4 = 0x2000;
        accessor.set_active_pml4(active_user_pml4);

        // Attempting to destroy currently active CR3 root must be rejected
        destroy_user_pml4_inner(active_user_pml4, &mut allocator, &accessor);

        // Active root frame must NOT be freed!
        assert!(!allocator.was_freed(active_user_pml4));
    }
}
