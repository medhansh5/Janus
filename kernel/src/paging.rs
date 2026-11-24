use crate::memory::FrameAllocator as PhysFrameAllocator;
use x86_64::structures::paging::page_table::PageTableEntry;
use x86_64::structures::paging::{PageTable, PageTableFlags, PhysFrame};
use x86_64::{PhysAddr, VirtAddr};

pub const PHYS_MEMORY_OFFSET: u64 = 0xFFFF900000000000;

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

/// Unsafe helper to copy from userspace using an arbitrary root page table and direct map offset.
///
/// Retained as a public kernel memory interface for cross-address-space task inspection.
///
/// # Safety
/// - `pml4` must be an authentic, valid page table.
/// - `phys_offset` must be the higher-half direct physical mapping offset mapping physical memory
///   up to `DIRECT_MAP_LIMIT`.
/// - Memory referenced by `pml4` must not be concurrently unmapped or modified during the copy.
#[allow(dead_code)]
pub unsafe fn copy_from_user_with_pml4(
    pml4: &PageTable,
    phys_offset: u64,
    user_vaddr: u64,
    buf: &mut [u8],
) -> Result<(), UserCopyError> {
    struct OffsetReader(u64);
    unsafe impl PageReader for OffsetReader {
        fn read_table(&self, paddr: u64) -> Result<&PageTable, UserCopyError> {
            let end_paddr = paddr
                .checked_add(4096)
                .ok_or(UserCopyError::ArithmeticOverflow)?;
            if end_paddr > DIRECT_MAP_LIMIT || (paddr & 0xFFF) != 0 {
                return Err(UserCopyError::InvalidPhysicalAddress);
            }
            let vaddr = paddr
                .checked_add(self.0)
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
                .checked_add(self.0)
                .ok_or(UserCopyError::ArithmeticOverflow)?;
            unsafe {
                core::ptr::copy(vaddr as *const u8, buf.as_mut_ptr(), buf.len());
            }
            Ok(())
        }
    }
    let reader = OffsetReader(phys_offset);
    copy_from_user_with_reader(&reader, pml4, user_vaddr, buf)
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

#[cfg(test)]
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
}
