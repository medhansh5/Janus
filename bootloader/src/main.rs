#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use log::{info, error};
use uefi::prelude::*;
use uefi::boot::{AllocateType, MemoryType, ScopedProtocol};
use uefi::proto::media::file::{File, FileAttribute, FileMode};
use uefi::proto::media::fs::SimpleFileSystem;
use uefi::mem::memory_map::MemoryMap;
use xmas_elf::ElfFile;
use xmas_elf::program::{Type, ProgramHeader};
use x86_64::structures::paging::{PageTable, PageTableFlags, PhysFrame, Size4KiB, Page, Mapper, FrameAllocator, OffsetPageTable};
use x86_64::{PhysAddr, VirtAddr};
use core::slice;
use boot_info::{BootInfo, MemoryRegion, MemoryRegionKind};

struct UefiFrameAllocator;

unsafe impl FrameAllocator<Size4KiB> for UefiFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        let addr = uefi::boot::allocate_pages(AllocateType::AnyPages, MemoryType::LOADER_DATA, 1).ok()?;
        Some(PhysFrame::containing_address(PhysAddr::new(addr.as_ptr() as u64)))
    }
}

const DEV_PUBLIC_KEY: &[u8] = b"MOCK_PUBLIC_KEY_12345";

fn verify_kernel_signature(_kernel_data: &[u8]) -> bool {
    info!("Verifying kernel signature against pubkey (len: {})... [MOCK PASS]", DEV_PUBLIC_KEY.len());
    true
}

#[entry]
fn main() -> Status {
    uefi::helpers::init().unwrap();

    info!("================================");
    info!("   Janus Microkernel Loader     ");
    info!("================================");

    // 1. Load kernel.elf
    let sfs_handle = uefi::boot::get_handle_for_protocol::<SimpleFileSystem>().unwrap();
    let mut sfs = uefi::boot::open_protocol_exclusive::<SimpleFileSystem>(sfs_handle).unwrap();
    let mut root = sfs.open_volume().unwrap();
    
    let mut kernel_file = match root.open(cstr16!("kernel.elf"), FileMode::Read, FileAttribute::empty()) {
        Ok(file) => file.into_regular_file().unwrap(),
        Err(_) => {
            error!("Failed to open kernel.elf");
            return Status::LOAD_ERROR;
        }
    };

    let mut kernel_buffer = Vec::with_capacity(1024 * 1024);
    kernel_buffer.resize(1024 * 1024, 0);
    let size = kernel_file.read(&mut kernel_buffer).unwrap();
    kernel_buffer.truncate(size);
    info!("Kernel loaded: {} bytes", size);

    // 2. Signature verification
    if !verify_kernel_signature(&kernel_buffer) {
        error!("Kernel signature verification failed! System halted.");
        return Status::SECURITY_VIOLATION;
    }

    // 3. Parse ELF and map with W^X
    let elf = ElfFile::new(&kernel_buffer).expect("Failed to parse kernel ELF");
    
    // UEFI page tables are often read-only. We allocate a new PML4 and copy the UEFI one.
    let cr3 = x86_64::registers::control::Cr3::read().0;
    let old_pml4_ptr = cr3.start_address().as_u64() as *const PageTable;
    
    let new_pml4_addr = uefi::boot::allocate_pages(AllocateType::AnyPages, MemoryType::LOADER_DATA, 1).unwrap();
    let new_pml4_ptr = new_pml4_addr.as_ptr() as *mut PageTable;
    
    unsafe {
        core::ptr::copy_nonoverlapping(old_pml4_ptr, new_pml4_ptr, 1);
    }
    
    let pml4 = unsafe { &mut *new_pml4_ptr };
    let mut mapper = unsafe { OffsetPageTable::new(pml4, VirtAddr::new(0)) };
    let mut frame_allocator = UefiFrameAllocator;

    let mut min_vaddr = u64::MAX;
    let mut max_vaddr = 0;
    for ph in elf.program_iter() {
        if ph.get_type() == Ok(Type::Load) {
            let start = ph.virtual_addr();
            let end = start + ph.mem_size();
            if start < min_vaddr { min_vaddr = start; }
            if end > max_vaddr { max_vaddr = end; }
        }
    }

    let min_page = min_vaddr & !0xFFF;
    let max_page = (max_vaddr + 0xFFF) & !0xFFF;
    let total_pages = ((max_page - min_page) / 0x1000) as usize;
    
    info!("Kernel virtual range: {:#x} - {:#x} ({} pages)", min_page, max_page, total_pages);

    let kernel_paddr_ptr = uefi::boot::allocate_pages(AllocateType::AnyPages, MemoryType::LOADER_DATA, total_pages).unwrap();
    let kernel_paddr = kernel_paddr_ptr.as_ptr() as u64;

    // Map all pages as RW first
    for i in 0..total_pages {
        let v = min_page + (i as u64 * 0x1000);
        let p = kernel_paddr + (i as u64 * 0x1000);
        let page = Page::<Size4KiB>::containing_address(VirtAddr::new(v));
        let frame = PhysFrame::containing_address(PhysAddr::new(p));
        let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE;
        unsafe {
            mapper.map_to(page, frame, flags, &mut frame_allocator).expect("Failed to map kernel").flush();
        }
    }

    // Switch CR3 so we can write to VirtAddr
    let new_cr3_frame = PhysFrame::containing_address(PhysAddr::new(new_pml4_ptr as u64));
    unsafe {
        x86_64::registers::control::Cr3::write(new_cr3_frame, x86_64::registers::control::Cr3Flags::empty());
    }
    info!("CR3 updated. Copying segments...");

    // Zero the memory
    unsafe {
        core::ptr::write_bytes(min_page as *mut u8, 0, (total_pages * 0x1000) as usize);
    }

    // Copy segments
    for ph in elf.program_iter() {
        if ph.get_type() == Ok(Type::Load) {
            unsafe {
                core::ptr::copy_nonoverlapping(
                    kernel_buffer[ph.offset() as usize..].as_ptr(),
                    ph.virtual_addr() as *mut u8,
                    ph.file_size() as usize,
                );
            }
        }
    }

    // Enforce W^X
    for ph in elf.program_iter() {
        if ph.get_type() == Ok(Type::Load) {
            let start_page = ph.virtual_addr() & !0xFFF;
            let end_page = ((ph.virtual_addr() + ph.mem_size()) + 0xFFF) & !0xFFF;
            let pages = ((end_page - start_page) / 0x1000) as usize;
            
            let mut flags = PageTableFlags::PRESENT;
            if !ph.flags().is_execute() { flags |= PageTableFlags::NO_EXECUTE; }
            if ph.flags().is_write() { flags |= PageTableFlags::WRITABLE; }
            
            for i in 0..pages {
                let v = start_page + (i as u64 * 0x1000);
                let page = Page::<Size4KiB>::containing_address(VirtAddr::new(v));
                unsafe {
                    mapper.update_flags(page, flags).unwrap().flush();
                }
            }
        }
    }

    let entry_point = elf.header.pt2.entry_point();
    info!("Kernel entry point: {:#x}", entry_point);
    info!("Calling ExitBootServices...");

    // 4. Memory map and Handoff
    let max_regions = 256;
    let regions_ptr = uefi::boot::allocate_pages(AllocateType::AnyPages, MemoryType::LOADER_DATA, (max_regions * core::mem::size_of::<MemoryRegion>() + 0xFFF) / 0x1000).unwrap();
    let regions_addr = regions_ptr.as_ptr() as u64;
    let regions_slice = unsafe { slice::from_raw_parts_mut(regions_addr as *mut MemoryRegion, max_regions) };

    let mmap = unsafe { uefi::boot::exit_boot_services(Some(MemoryType::LOADER_DATA)) };

    let mut region_count = 0;
    for desc in mmap.entries() {
        if region_count >= max_regions { break; }
        let kind = match desc.ty {
            MemoryType::CONVENTIONAL => MemoryRegionKind::Usable,
            MemoryType::LOADER_DATA | MemoryType::LOADER_CODE => MemoryRegionKind::Bootloader,
            _ => MemoryRegionKind::Reserved,
        };
        regions_slice[region_count] = MemoryRegion {
            start: desc.phys_start,
            size: desc.page_count * 0x1000,
            kind,
        };
        region_count += 1;
    }

    let boot_info = BootInfo {
        memory_regions_ptr: regions_addr as *const MemoryRegion,
        memory_regions_count: region_count,
    };

    let kernel_main: extern "sysv64" fn(*const BootInfo) -> ! = unsafe { core::mem::transmute(entry_point) };
    kernel_main(&boot_info as *const _);
}
