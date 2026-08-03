use boot_info::{BootInfo, MemoryRegionKind};

pub const PAGE_SIZE: u64 = 0x1000;

pub struct FrameAllocator {
    head: Option<u64>,
    total_frames: usize,
    phys_offset: u64,
}

impl FrameAllocator {
    pub const fn new() -> Self {
        Self { head: None, total_frames: 0, phys_offset: 0 }
    }

    pub fn set_phys_offset(&mut self, offset: u64) {
        self.phys_offset = offset;
    }

    /// Initializes the physical frame allocator from the boot memory map.
    /// # Safety
    /// This function requires the physical memory to be identity-mapped or
    /// accessible at `paddr`.
    pub unsafe fn init(&mut self, boot_info: *const BootInfo) {
        let boot_info_ref = &*boot_info;
        let regions = core::slice::from_raw_parts(boot_info_ref.memory_regions_ptr, boot_info_ref.memory_regions_count);
        
        for region in regions {
            if region.kind == MemoryRegionKind::Usable {
                let start = region.start;
                let end = region.start + region.size;
                let mut current = start;
                
                // Skip address 0 (null pointer) to prevent Option<u64> confusion, 
                // though Option<NonZeroU64> would be safer.
                if current == 0 {
                    current += PAGE_SIZE;
                }

                while current + PAGE_SIZE <= end {
                    self.free_frame(current);
                    current += PAGE_SIZE;
                }
            }
        }
    }

    pub fn allocate_frame(&mut self) -> Option<u64> {
        if let Some(addr) = self.head {
            unsafe {
                let ptr = (addr + self.phys_offset) as *const Option<u64>;
                self.head = core::ptr::read(ptr);
            }
            self.total_frames -= 1;
            Some(addr)
        } else {
            None
        }
    }

    pub fn free_frame(&mut self, paddr: u64) {
        unsafe {
            let ptr = (paddr + self.phys_offset) as *mut Option<u64>;
            core::ptr::write(ptr, self.head);
        }
        self.head = Some(paddr);
        self.total_frames += 1;
    }

    /// Iterates through the free list and coalesces contiguous frames into a single region.
    /// This is slow (O(N)), but only called once during boot to mint Untyped capabilities.
    pub fn extract_largest_contiguous(&mut self) -> Option<(u64, u64)> {
        // For v0, rather than a full coalescing which is complex for a simple free-list,
        // we'll just scan the free list, find the longest contiguous sequence,
        // remove it from the list, and return it.
        // Even simpler: just return the remaining frames as 4KB capabilities until we run out of slots,
        // OR rely on the fact that our initial allocator setup pushes frames in order, 
        // so the list is already mostly contiguous chunks.
        
        // Actually, the simplest approach for v0 init is to pop chunks that are already contiguous
        // in the list.
        if self.head.is_none() {
            return None;
        }

        let mut current = self.head.unwrap();
        let mut size = 4096;
        let mut prev_ptr = &mut self.head as *mut Option<u64>;

        unsafe {
            loop {
                let next_ptr = (current + self.phys_offset) as *mut Option<u64>;
                if let Some(next) = core::ptr::read(next_ptr) {
                    // Since frames are pushed to the head, the next frame in the list
                    // has a LOWER physical address.
                    if next == current - 4096 {
                        current = next;
                        size += 4096;
                        self.total_frames -= 1; // accounting
                    } else {
                        // Break in contiguity.
                        // Splice the list.
                        core::ptr::write(prev_ptr, Some(next));
                        self.total_frames -= 1; // for the head frame
                        // The region starts at the lowest address (current)
                        return Some((current, size));
                    }
                } else {
                    // End of list
                    core::ptr::write(prev_ptr, None);
                    self.total_frames -= 1;
                    return Some((current, size));
                }
            }
        }
    }

    pub fn total_usable_frames(&self) -> usize {
        self.total_frames
    }
}
