#![allow(clippy::result_unit_err)]

use core::hint::spin_loop;
use core::sync::atomic::{fence, Ordering};

pub const VIRTIO_PCI_HOST_FEATURES: u16 = 0;
pub const VIRTIO_PCI_GUEST_FEATURES: u16 = 4;
pub const VIRTIO_PCI_QUEUE_PFN: u16 = 8;
pub const VIRTIO_PCI_QUEUE_NUM: u16 = 12;
pub const VIRTIO_PCI_QUEUE_SEL: u16 = 14;
pub const VIRTIO_PCI_QUEUE_NOTIFY: u16 = 16;
pub const VIRTIO_PCI_STATUS: u16 = 18;
pub const VIRTIO_PCI_ISR: u16 = 19;

pub const VIRTIO_STATUS_ACKNOWLEDGE: u8 = 1;
pub const VIRTIO_STATUS_DRIVER: u8 = 2;
pub const VIRTIO_STATUS_DRIVER_OK: u8 = 4;
pub const VIRTIO_STATUS_FAILED: u8 = 128;

pub const VRING_DESC_F_NEXT: u16 = 1;
pub const VRING_DESC_F_WRITE: u16 = 2;

pub const VIRTIO_BLK_T_IN: u32 = 0; // Read from disk
pub const VIRTIO_BLK_T_OUT: u32 = 1; // Write to disk

pub const VIRTIO_BLK_S_OK: u8 = 0;
pub const VIRTIO_BLK_S_IOERR: u8 = 1;
pub const VIRTIO_BLK_S_UNSUPP: u8 = 2;

pub const SECTOR_SIZE: usize = 512;

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct VRingDesc {
    pub addr: u64,
    pub len: u32,
    pub flags: u16,
    pub next: u16,
}

#[repr(C)]
pub struct VRingAvail {
    pub flags: u16,
    pub idx: u16,
    pub ring: [u16; 256],
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct VRingUsedElem {
    pub id: u32,
    pub len: u32,
}

#[repr(C)]
pub struct VRingUsed {
    pub flags: u16,
    pub idx: u16,
    pub ring: [VRingUsedElem; 256],
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct VirtioBlkOutHdr {
    pub req_type: u32,
    pub ioprio: u32,
    pub sector: u64,
}

pub struct VirtioBlockDevice {
    pub io_base: u16,
    pub virtqueue_paddr: u64,
    pub q_size: u16,
    pub last_used_idx: u16,
    pub initialized: bool,
}

impl Default for VirtioBlockDevice {
    fn default() -> Self {
        Self::new()
    }
}

impl VirtioBlockDevice {
    pub const fn new() -> Self {
        Self {
            io_base: 0,
            virtqueue_paddr: 0,
            q_size: 0,
            last_used_idx: 0,
            initialized: false,
        }
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Initializes the legacy VirtIO PCI block device.
    ///
    /// # Safety
    /// Must be called with valid `io_base` and 3 physically contiguous, 4KB-aligned pages at `virtqueue_paddr`.
    pub unsafe fn init(&mut self, io_base: u16, virtqueue_paddr: u64) -> Result<(), ()> {
        self.io_base = io_base;
        self.virtqueue_paddr = virtqueue_paddr;

        // 1. Reset device
        crate::pci::outb(io_base + VIRTIO_PCI_STATUS, 0);

        // 2. Set ACKNOWLEDGE status bit
        crate::pci::outb(io_base + VIRTIO_PCI_STATUS, VIRTIO_STATUS_ACKNOWLEDGE);

        // 3. Set DRIVER status bit
        crate::pci::outb(
            io_base + VIRTIO_PCI_STATUS,
            VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER,
        );

        // 4. Feature negotiation
        let _features = crate::pci::inl(io_base + VIRTIO_PCI_HOST_FEATURES);
        crate::pci::outl(io_base + VIRTIO_PCI_GUEST_FEATURES, 0);

        // 5. Select Queue 0 and query size
        crate::pci::outw(io_base + VIRTIO_PCI_QUEUE_SEL, 0);
        let q_size = crate::pci::inw(io_base + VIRTIO_PCI_QUEUE_NUM);
        if q_size == 0 {
            crate::pci::outb(io_base + VIRTIO_PCI_STATUS, VIRTIO_STATUS_FAILED);
            return Err(());
        }
        self.q_size = q_size;

        // 6. Zero out the allocated 3-page (12KB) virtqueue memory
        let vaddr = virtqueue_paddr + crate::paging::PHYS_MEMORY_OFFSET;
        core::ptr::write_bytes(vaddr as *mut u8, 0, 12288);

        // 7. Write page frame number (PFN) to Queue Address register
        crate::pci::outl(
            io_base + VIRTIO_PCI_QUEUE_PFN,
            (virtqueue_paddr >> 12) as u32,
        );

        // 8. Set DRIVER_OK status bit
        crate::pci::outb(
            io_base + VIRTIO_PCI_STATUS,
            VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER | VIRTIO_STATUS_DRIVER_OK,
        );

        self.last_used_idx = 0;
        self.initialized = true;
        Ok(())
    }

    /// Reads a 512-byte sector from disk into `buf`.
    ///
    /// # Safety
    /// Must be called with interrupts disabled or serialized access.
    pub unsafe fn read_sector(
        &mut self,
        sector: u64,
        buf: &mut [u8; SECTOR_SIZE],
    ) -> Result<(), ()> {
        if !self.initialized {
            return Err(());
        }
        let vaddr = self.virtqueue_paddr + crate::paging::PHYS_MEMORY_OFFSET;
        let desc = &mut *(vaddr as *mut [VRingDesc; 256]);
        let avail = &mut *((vaddr + 0x1000) as *mut VRingAvail);
        let used = &*((vaddr + 0x2000) as *const VRingUsed);

        // DMA Buffers placed in the upper region of Page 2 (well past used.ring at 0x2806)
        let hdr_ptr = (vaddr + 0x2900) as *mut VirtioBlkOutHdr;
        let data_ptr = (vaddr + 0x2A00) as *mut [u8; SECTOR_SIZE];
        let status_ptr = (vaddr + 0x2C00) as *mut u8;

        (*hdr_ptr).req_type = VIRTIO_BLK_T_IN;
        (*hdr_ptr).ioprio = 0;
        (*hdr_ptr).sector = sector;
        *status_ptr = 0xFF;

        // Descriptor 0: Request Header (Device reads)
        desc[0].addr = self.virtqueue_paddr + 0x2900;
        desc[0].len = 16;
        desc[0].flags = VRING_DESC_F_NEXT;
        desc[0].next = 1;

        // Descriptor 1: Sector Buffer (Device writes)
        desc[1].addr = self.virtqueue_paddr + 0x2A00;
        desc[1].len = SECTOR_SIZE as u32;
        desc[1].flags = VRING_DESC_F_NEXT | VRING_DESC_F_WRITE;
        desc[1].next = 2;

        // Descriptor 2: Status Byte (Device writes)
        desc[2].addr = self.virtqueue_paddr + 0x2C00;
        desc[2].len = 1;
        desc[2].flags = VRING_DESC_F_WRITE;
        desc[2].next = 0;

        // Publish to available ring
        let avail_slot = (avail.idx as usize) % 256;
        avail.ring[avail_slot] = 0;
        fence(Ordering::SeqCst);
        avail.idx = avail.idx.wrapping_add(1);
        fence(Ordering::SeqCst);

        // Notify device of new buffer on Queue 0
        crate::pci::outw(self.io_base + VIRTIO_PCI_QUEUE_NOTIFY, 0);

        let used_idx_ptr = core::ptr::addr_of!(used.idx);
        let mut spins = 0;
        while core::ptr::read_volatile(used_idx_ptr) == self.last_used_idx {
            spin_loop();
            spins += 1;
            if spins > 2_000_000 {
                return Err(());
            }
        }
        self.last_used_idx = core::ptr::read_volatile(used_idx_ptr);

        let status = core::ptr::read_volatile(status_ptr);
        if status != VIRTIO_BLK_S_OK {
            return Err(());
        }

        buf.copy_from_slice(&*data_ptr);
        Ok(())
    }

    /// Writes a 512-byte sector from `buf` to disk.
    ///
    /// # Safety
    /// Must be called with interrupts disabled or serialized access.
    pub unsafe fn write_sector(&mut self, sector: u64, buf: &[u8; SECTOR_SIZE]) -> Result<(), ()> {
        if !self.initialized {
            return Err(());
        }
        let vaddr = self.virtqueue_paddr + crate::paging::PHYS_MEMORY_OFFSET;
        let desc = &mut *(vaddr as *mut [VRingDesc; 256]);
        let avail = &mut *((vaddr + 0x1000) as *mut VRingAvail);
        let used = &*((vaddr + 0x2000) as *const VRingUsed);

        let hdr_ptr = (vaddr + 0x2900) as *mut VirtioBlkOutHdr;
        let data_ptr = (vaddr + 0x2A00) as *mut [u8; SECTOR_SIZE];
        let status_ptr = (vaddr + 0x2C00) as *mut u8;

        (*hdr_ptr).req_type = VIRTIO_BLK_T_OUT;
        (*hdr_ptr).ioprio = 0;
        (*hdr_ptr).sector = sector;
        (*data_ptr).copy_from_slice(buf);
        *status_ptr = 0xFF;

        // Descriptor 0: Request Header (Device reads)
        desc[0].addr = self.virtqueue_paddr + 0x2900;
        desc[0].len = 16;
        desc[0].flags = VRING_DESC_F_NEXT;
        desc[0].next = 1;

        // Descriptor 1: Sector Buffer (Device reads)
        desc[1].addr = self.virtqueue_paddr + 0x2A00;
        desc[1].len = SECTOR_SIZE as u32;
        desc[1].flags = VRING_DESC_F_NEXT; // Read-only for device
        desc[1].next = 2;

        // Descriptor 2: Status Byte (Device writes)
        desc[2].addr = self.virtqueue_paddr + 0x2C00;
        desc[2].len = 1;
        desc[2].flags = VRING_DESC_F_WRITE;
        desc[2].next = 0;

        // Publish to available ring
        let avail_slot = (avail.idx as usize) % 256;
        avail.ring[avail_slot] = 0;
        fence(Ordering::SeqCst);
        avail.idx = avail.idx.wrapping_add(1);
        fence(Ordering::SeqCst);

        // Notify device of new buffer on Queue 0
        crate::pci::outw(self.io_base + VIRTIO_PCI_QUEUE_NOTIFY, 0);

        let used_idx_ptr = core::ptr::addr_of!(used.idx);
        let mut spins = 0;
        while core::ptr::read_volatile(used_idx_ptr) == self.last_used_idx {
            spin_loop();
            spins += 1;
            if spins > 2_000_000 {
                return Err(());
            }
        }
        self.last_used_idx = core::ptr::read_volatile(used_idx_ptr);

        let status = core::ptr::read_volatile(status_ptr);
        if status != VIRTIO_BLK_S_OK {
            return Err(());
        }

        Ok(())
    }
}
