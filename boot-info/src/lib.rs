#![no_std]

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryRegionKind {
    Usable,
    Bootloader,
    Kernel,
    Reserved,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MemoryRegion {
    pub start: u64,
    pub size: u64,
    pub kind: MemoryRegionKind,
}

#[repr(C)]
pub struct BootInfo {
    pub memory_regions_ptr: *const MemoryRegion,
    pub memory_regions_count: usize,
}
