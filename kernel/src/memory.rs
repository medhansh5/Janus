use boot_info::{BootInfo, MemoryRegionKind};

pub const PAGE_SIZE: u64 = 0x1000;

pub struct FrameAllocator {
    head: Option<u64>,
    total_frames: usize,
    phys_offset: u64,
}

impl Default for FrameAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameAllocator {
    pub const fn new() -> Self {
        Self {
            head: None,
            total_frames: 0,
            phys_offset: 0,
        }
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
        let regions = core::slice::from_raw_parts(
            boot_info_ref.memory_regions_ptr,
            boot_info_ref.memory_regions_count,
        );

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

    /// Allocates a contiguous sequence of `count` frames.
    pub fn allocate_contiguous_frames(&mut self, count: usize) -> Option<u64> {
        if count == 0 {
            return None;
        }
        if count == 1 {
            return self.allocate_frame();
        }

        let mut current_opt = self.head;
        let mut prev_ptr = &mut self.head as *mut Option<u64>;

        while let Some(current_addr) = current_opt {
            let mut check_addr = current_addr;
            let mut found_count = 1;

            let mut peek_opt =
                unsafe { core::ptr::read((check_addr + self.phys_offset) as *const Option<u64>) };
            let mut lowest_addr = current_addr;

            // Detect direction on first step
            if let Some(next_addr) = peek_opt {
                let direction = if next_addr == check_addr - PAGE_SIZE {
                    -1
                } else if next_addr == check_addr + PAGE_SIZE {
                    1
                } else {
                    0
                };

                if direction != 0 {
                    while let Some(n_addr) = peek_opt {
                        let expected = if direction == 1 {
                            check_addr + PAGE_SIZE
                        } else {
                            check_addr - PAGE_SIZE
                        };
                        if n_addr == expected {
                            found_count += 1;
                            if n_addr < lowest_addr {
                                lowest_addr = n_addr;
                            }
                            if found_count == count {
                                unsafe {
                                    let next_after_chain = core::ptr::read(
                                        (n_addr + self.phys_offset) as *const Option<u64>,
                                    );
                                    core::ptr::write(prev_ptr, next_after_chain);
                                }
                                self.total_frames -= count;
                                return Some(lowest_addr);
                            }
                            check_addr = n_addr;
                            peek_opt = unsafe {
                                core::ptr::read(
                                    (check_addr + self.phys_offset) as *const Option<u64>,
                                )
                            };
                        } else {
                            break; // Break in contiguity
                        }
                    }
                }
            }

            prev_ptr = (current_addr + self.phys_offset) as *mut Option<u64>;
            current_opt = unsafe { core::ptr::read(prev_ptr) };
        }

        None
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
        let mut current = self.head?;
        let mut size = 4096;

        let prev_ptr = &mut self.head as *mut Option<u64>;

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
