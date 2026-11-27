#![allow(
    clippy::result_unit_err,
    clippy::needless_range_loop,
    clippy::manual_range_contains,
    clippy::collapsible_if
)]

pub const MAGIC: u32 = 0x4A41_4E53; // "JANS"
pub const BLOCK_SIZE: usize = 512;
pub const TOTAL_BLOCKS: u32 = 2048; // 1 MB
pub const MAX_INODES: usize = 64;
pub const MAX_DIRECT_BLOCKS: usize = 5;
pub const MAX_FILE_SIZE: usize = MAX_DIRECT_BLOCKS * BLOCK_SIZE; // 2560 bytes

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Superblock {
    pub magic: u32,
    pub version: u32,
    pub block_size: u32,
    pub total_blocks: u32,
    pub inode_count: u32,
    pub free_blocks: u32,
    pub reserved: [u32; 122],
}

impl Default for Superblock {
    fn default() -> Self {
        Self::new()
    }
}

impl Superblock {
    pub const fn new() -> Self {
        Self {
            magic: MAGIC,
            version: 1,
            block_size: BLOCK_SIZE as u32,
            total_blocks: TOTAL_BLOCKS,
            inode_count: MAX_INODES as u32,
            free_blocks: TOTAL_BLOCKS - 5,
            reserved: [0; 122],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Inode {
    pub name: [u8; 16],
    pub size: u32,
    pub flags: u16, // 0 = Free, 1 = Regular file
    pub direct_blocks: [u16; MAX_DIRECT_BLOCKS],
    pub reserved: u16,
}

impl Inode {
    pub const fn empty() -> Self {
        Self {
            name: [0; 16],
            size: 0,
            flags: 0,
            direct_blocks: [0; MAX_DIRECT_BLOCKS],
            reserved: 0,
        }
    }

    pub fn is_used(&self) -> bool {
        self.flags == 1
    }

    pub fn matches_name(&self, name: &str) -> bool {
        if !self.is_used() {
            return false;
        }
        let name_bytes = name.as_bytes();
        let len = core::cmp::min(name_bytes.len(), 16);
        if self.name[..len] != name_bytes[..len] {
            return false;
        }
        if len < 16 && self.name[len] != 0 {
            return false;
        }
        true
    }

    pub fn set_name(&mut self, name: &str) {
        self.name = [0; 16];
        let bytes = name.as_bytes();
        let len = core::cmp::min(bytes.len(), 16);
        self.name[..len].copy_from_slice(&bytes[..len]);
    }
}

use crate::capability::{CPtr, CapRights, CapType};
#[cfg(not(test))]
use crate::virtio_blk::VirtioBlockDevice;

// In-memory block cache (holding 64 active blocks)
pub const CACHE_BLOCKS: usize = 64;
static mut BLOCK_CACHE: [[u8; BLOCK_SIZE]; CACHE_BLOCKS] = [[0; BLOCK_SIZE]; CACHE_BLOCKS];
static mut ROLLBACK_BLOCKS: [[u8; BLOCK_SIZE]; MAX_DIRECT_BLOCKS] =
    [[0; BLOCK_SIZE]; MAX_DIRECT_BLOCKS];
static mut SUPERBLOCK: Superblock = Superblock::new();
static mut INODES: [Inode; MAX_INODES] = [Inode::empty(); MAX_INODES];
#[cfg(not(test))]
static mut VIRTIO_DEV: VirtioBlockDevice = VirtioBlockDevice::new();
#[cfg(not(test))]
static mut HAS_VIRTIO: bool = false;
static mut FS_INITIALIZED: bool = false;

#[cfg(test)]
pub static mut TEST_FAIL_ON_WRITE_NUM: Option<usize> = None;
#[cfg(test)]
pub static mut TEST_WRITE_COUNT: usize = 0;

/// Flushes a block (0..CACHE_BLOCKS) from in-memory cache / structures to VirtIO disk.
pub fn flush_block_to_disk(_block_idx: u16) -> Result<(), ()> {
    #[cfg(test)]
    unsafe {
        let count = TEST_WRITE_COUNT;
        TEST_WRITE_COUNT += 1;
        if TEST_FAIL_ON_WRITE_NUM == Some(count) {
            return Err(());
        }
    }
    #[cfg(not(test))]
    unsafe {
        if !HAS_VIRTIO {
            return Ok(());
        }
        if _block_idx == 0 {
            let sb_ptr = core::ptr::addr_of!(SUPERBLOCK) as *const [u8; BLOCK_SIZE];
            return VIRTIO_DEV.write_sector(0, &*sb_ptr);
        } else if (1..=4).contains(&_block_idx) {
            let inode_offset = (_block_idx as usize - 1) * 16;
            let slice_ptr = core::ptr::addr_of!(INODES[inode_offset]) as *const [u8; BLOCK_SIZE];
            return VIRTIO_DEV.write_sector(_block_idx as u64, &*slice_ptr);
        } else if (_block_idx as usize) < CACHE_BLOCKS {
            return VIRTIO_DEV.write_sector(_block_idx as u64, &BLOCK_CACHE[_block_idx as usize]);
        }
    }
    Ok(())
}

/// Flushes all filesystem metadata and cached blocks to disk.
pub fn sync() -> Result<(), ()> {
    flush_block_to_disk(0)?;
    for b in 1..=4 {
        flush_block_to_disk(b)?;
    }
    for b in 5..CACHE_BLOCKS as u16 {
        flush_block_to_disk(b)?;
    }
    Ok(())
}

/// Formats the filesystem, clearing all inodes and writing default seeded files.
pub fn format() {
    unsafe {
        SUPERBLOCK = Superblock::new();
        INODES = [Inode::empty(); MAX_INODES];
        BLOCK_CACHE = [[0; BLOCK_SIZE]; CACHE_BLOCKS];

        // Seed default files:
        // 1. /welcome.txt
        let _ = write_file(
            "/welcome.txt",
            b"Welcome to Janus OS v1.0!\nA capability-based, security-first microkernel in Rust.\n",
        );

        // 2. /help.txt
        let _ = write_file(
            "/help.txt",
            b"Janus OS Shell Commands:\n  help    - show this help\n  ps      - list running tasks\n  ls      - list files\n  cat     - display file content\n  write   - create or overwrite file\n  rm      - delete file\n  time    - system uptime\n  echo    - print text\n  test    - run test suite\n  clear   - clear terminal\n  exit    - halt/terminate shell\n",
        );

        // 3. /version.txt
        let _ = write_file(
            "/version.txt",
            b"Janus OS v1.0.0 (x86_64-unknown-none)\nTarget: Full Usable V1\n",
        );

        let _ = sync();
        FS_INITIALIZED = true;
    }
}

/// Initializes JanusFS. If a valid superblock exists on disk, restores from disk.
pub fn init(_io_base: Option<u16>, _virtqueue_paddr: Option<u64>) {
    #[cfg(not(test))]
    unsafe {
        if let (Some(base), Some(vq_paddr)) = (_io_base, _virtqueue_paddr) {
            let _ = VIRTIO_DEV.init(base, vq_paddr);
            HAS_VIRTIO = true;

            // Probe sector 0 for Superblock
            let mut sec_buf = [0u8; BLOCK_SIZE];
            if VIRTIO_DEV.read_sector(0, &mut sec_buf).is_ok() {
                let probed_magic =
                    u32::from_le_bytes([sec_buf[0], sec_buf[1], sec_buf[2], sec_buf[3]]);
                if probed_magic == MAGIC {
                    // Valid JanusFS filesystem detected on disk! Restore metadata
                    core::ptr::copy_nonoverlapping(
                        sec_buf.as_ptr(),
                        core::ptr::addr_of_mut!(SUPERBLOCK) as *mut u8,
                        BLOCK_SIZE,
                    );
                    for b in 1..=4 {
                        let mut inode_buf = [0u8; BLOCK_SIZE];
                        if VIRTIO_DEV.read_sector(b, &mut inode_buf).is_ok() {
                            let inode_offset = (b as usize - 1) * 16;
                            core::ptr::copy_nonoverlapping(
                                inode_buf.as_ptr(),
                                core::ptr::addr_of_mut!(INODES[inode_offset]) as *mut u8,
                                BLOCK_SIZE,
                            );
                        }
                    }
                    // Restore active data blocks into BLOCK_CACHE
                    for inode in INODES.iter() {
                        if inode.is_used() {
                            for &blk in inode.direct_blocks.iter() {
                                let blk_idx = blk as usize;
                                if blk_idx >= 5 && blk_idx < CACHE_BLOCKS {
                                    let _ = VIRTIO_DEV
                                        .read_sector(blk as u64, &mut BLOCK_CACHE[blk_idx]);
                                }
                            }
                        }
                    }
                    FS_INITIALIZED = true;
                    let mut serial = crate::SerialPort(0x3F8);
                    serial.write_str("[JanusFS] Existing filesystem restored from VirtIO disk.\n");
                    return;
                }
            }
        }
    }

    unsafe {
        if !FS_INITIALIZED {
            format();
        }
    }
}

/// Finds a free data block index (starting at block 5), excluding blocks in `pending`.
fn allocate_data_block_excluding(pending: &[u16]) -> Option<u16> {
    unsafe {
        let mut used = [false; CACHE_BLOCKS];
        for inode in INODES.iter() {
            if inode.is_used() {
                for &b in inode.direct_blocks.iter() {
                    let idx = b as usize;
                    if idx >= 5 && idx < CACHE_BLOCKS {
                        used[idx] = true;
                    }
                }
            }
        }
        for &b in pending.iter() {
            let idx = b as usize;
            if idx >= 5 && idx < CACHE_BLOCKS {
                used[idx] = true;
            }
        }
        for (i, &is_used) in used.iter().enumerate().skip(5) {
            if !is_used {
                return Some(i as u16);
            }
        }
        None
    }
}

/// Creates a new empty file in JanusFS.
pub fn create_file(name: &str) -> Result<usize, ()> {
    if name.is_empty() || name.len() > 16 {
        return Err(());
    }
    unsafe {
        // Check if already exists
        for (i, inode) in INODES.iter().enumerate() {
            if inode.matches_name(name) {
                return Ok(i);
            }
        }

        // Find free inode
        for (i, inode) in INODES.iter_mut().enumerate() {
            if !inode.is_used() {
                inode.flags = 1;
                inode.size = 0;
                inode.set_name(name);
                inode.direct_blocks = [0; MAX_DIRECT_BLOCKS];
                let inode_blk = 1 + (i / 16) as u16;
                if flush_block_to_disk(inode_blk).is_err() {
                    *inode = Inode::empty();
                    return Err(());
                }
                return Ok(i);
            }
        }
        Err(())
    }
}

/// Reads data from a file in JanusFS into `buf`.
///
/// Returns the number of bytes read.
pub fn read_file(name: &str, buf: &mut [u8]) -> Result<usize, ()> {
    unsafe {
        for inode in INODES.iter() {
            if inode.matches_name(name) {
                let capped_size = core::cmp::min(inode.size as usize, MAX_FILE_SIZE);
                let to_read = core::cmp::min(buf.len(), capped_size);
                let mut read_bytes = 0;

                for &block_num in inode.direct_blocks.iter() {
                    if block_num == 0 || read_bytes >= to_read {
                        break;
                    }
                    let chunk = core::cmp::min(BLOCK_SIZE, to_read - read_bytes);
                    let block_idx = block_num as usize;
                    if block_idx < CACHE_BLOCKS {
                        buf[read_bytes..read_bytes + chunk]
                            .copy_from_slice(&BLOCK_CACHE[block_idx][..chunk]);
                    }
                    read_bytes += chunk;
                }
                return Ok(read_bytes);
            }
        }
        Err(())
    }
}

/// Writes `data` into a file in JanusFS transactionally.
///
/// Pre-validates bounds and pre-allocates all required blocks before mutating
/// persistent metadata. On allocation failure or disk write error, rolls back
/// any staged block-cache or inode changes so old contents and metadata remain intact.
pub fn write_file(name: &str, data: &[u8]) -> Result<usize, ()> {
    if name.is_empty() || name.len() > 16 || data.len() > MAX_FILE_SIZE {
        return Err(());
    }

    unsafe {
        // 1. Locate existing inode or free inode slot without mutating INODES
        let mut existing_idx: Option<usize> = None;
        let mut free_idx: Option<usize> = None;
        for (i, inode) in INODES.iter().enumerate() {
            if inode.matches_name(name) {
                existing_idx = Some(i);
                break;
            }
            if !inode.is_used() && free_idx.is_none() {
                free_idx = Some(i);
            }
        }
        let inode_idx = existing_idx.or(free_idx).ok_or(())?;
        let old_inode = INODES[inode_idx];

        let bytes_to_write = data.len();
        let needed_blocks = bytes_to_write.div_ceil(BLOCK_SIZE);

        // 2. Plan & pre-allocate all required blocks before touching INODES or BLOCK_CACHE
        let mut target_blocks = [0u16; MAX_DIRECT_BLOCKS];
        for b_idx in 0..needed_blocks {
            if old_inode.is_used() && old_inode.direct_blocks[b_idx] != 0 {
                target_blocks[b_idx] = old_inode.direct_blocks[b_idx];
            } else {
                let new_blk = allocate_data_block_excluding(&target_blocks[..b_idx]).ok_or(())?;
                target_blocks[b_idx] = new_blk;
            }
        }

        // 3. Snapshot prior BLOCK_CACHE contents for rollback on disk I/O failure
        //    (stored in static ROLLBACK_BLOCKS to avoid overflowing the 4 KiB kernel stack)
        for b_idx in 0..needed_blocks {
            let blk_idx = target_blocks[b_idx] as usize;
            if blk_idx >= CACHE_BLOCKS {
                return Err(());
            }
            ROLLBACK_BLOCKS[b_idx].copy_from_slice(&BLOCK_CACHE[blk_idx]);
        }

        // 4. Write data blocks into cache and flush to disk
        let mut written = 0;
        for b_idx in 0..needed_blocks {
            let block_num = target_blocks[b_idx];
            let blk_idx = block_num as usize;
            let chunk = core::cmp::min(BLOCK_SIZE, bytes_to_write - written);
            BLOCK_CACHE[blk_idx] = [0; BLOCK_SIZE];
            BLOCK_CACHE[blk_idx][..chunk].copy_from_slice(&data[written..written + chunk]);
            if flush_block_to_disk(block_num).is_err() {
                for r_idx in 0..=b_idx {
                    BLOCK_CACHE[target_blocks[r_idx] as usize]
                        .copy_from_slice(&ROLLBACK_BLOCKS[r_idx]);
                }
                return Err(());
            }
            written += chunk;
        }

        // 5. Commit new inode metadata and flush inode sector + superblock
        let mut new_inode = Inode::empty();
        new_inode.flags = 1;
        new_inode.size = bytes_to_write as u32;
        new_inode.set_name(name);
        new_inode.direct_blocks = target_blocks;
        INODES[inode_idx] = new_inode;

        let inode_blk = 1 + (inode_idx / 16) as u16;
        if flush_block_to_disk(inode_blk).is_err() || flush_block_to_disk(0).is_err() {
            INODES[inode_idx] = old_inode;
            for r_idx in 0..needed_blocks {
                BLOCK_CACHE[target_blocks[r_idx] as usize].copy_from_slice(&ROLLBACK_BLOCKS[r_idx]);
            }
            return Err(());
        }

        // 6. Release any trailing blocks from an overwrite that shrank the file
        if old_inode.is_used() {
            for b_idx in needed_blocks..MAX_DIRECT_BLOCKS {
                let old_blk = old_inode.direct_blocks[b_idx];
                if old_blk != 0 && (old_blk as usize) < CACHE_BLOCKS {
                    BLOCK_CACHE[old_blk as usize] = [0; BLOCK_SIZE];
                    let _ = flush_block_to_disk(old_blk);
                }
            }
        }

        Ok(written)
    }
}

/// Deletes a file from JanusFS.
pub fn delete_file(name: &str) -> Result<(), ()> {
    unsafe {
        for (idx, inode) in INODES.iter_mut().enumerate() {
            if inode.matches_name(name) {
                let old_inode = *inode;
                for (b_i, blk) in old_inode.direct_blocks.iter().enumerate() {
                    let b_idx = *blk as usize;
                    if b_idx >= 5 && b_idx < CACHE_BLOCKS {
                        ROLLBACK_BLOCKS[b_i].copy_from_slice(&BLOCK_CACHE[b_idx]);
                        BLOCK_CACHE[b_idx] = [0; BLOCK_SIZE];
                        if flush_block_to_disk(*blk).is_err() {
                            for r_i in 0..=b_i {
                                let rb = old_inode.direct_blocks[r_i] as usize;
                                if rb >= 5 && rb < CACHE_BLOCKS {
                                    BLOCK_CACHE[rb].copy_from_slice(&ROLLBACK_BLOCKS[r_i]);
                                }
                            }
                            return Err(());
                        }
                    }
                }
                *inode = Inode::empty();
                let inode_blk = 1 + (idx / 16) as u16;
                if flush_block_to_disk(inode_blk).is_err() || flush_block_to_disk(0).is_err() {
                    *inode = old_inode;
                    for (r_i, blk) in old_inode.direct_blocks.iter().enumerate() {
                        let rb = *blk as usize;
                        if rb >= 5 && rb < CACHE_BLOCKS {
                            BLOCK_CACHE[rb].copy_from_slice(&ROLLBACK_BLOCKS[r_i]);
                        }
                    }
                    return Err(());
                }
                return Ok(());
            }
        }
        Err(())
    }
}

/// Formats the directory listing into `buf`.
///
/// Returns number of bytes formatted.
pub fn list_files(buf: &mut [u8]) -> usize {
    let mut offset = 0;

    let header = b"JanusFS Directory:\nNAME             SIZE\n-----------------------\n";
    if buf.len() < header.len() {
        return 0;
    }
    buf[..header.len()].copy_from_slice(header);
    offset += header.len();

    unsafe {
        for inode in INODES.iter() {
            if inode.is_used() {
                // Find name length
                let mut name_len = 0;
                while name_len < 16 && inode.name[name_len] != 0 {
                    name_len += 1;
                }

                if offset + name_len + 32 > buf.len() {
                    break;
                }

                // Write name
                buf[offset..offset + name_len].copy_from_slice(&inode.name[..name_len]);
                offset += name_len;

                // Pad spaces
                let pad = if name_len < 16 { 16 - name_len } else { 1 };
                for _ in 0..pad {
                    if offset < buf.len() {
                        buf[offset] = b' ';
                        offset += 1;
                    }
                }

                // Format size
                let size = inode.size;
                offset += format_u32(size, &mut buf[offset..]);

                if offset + 3 < buf.len() {
                    buf[offset] = b' ';
                    buf[offset + 1] = b'B';
                    buf[offset + 2] = b'\n';
                    offset += 3;
                }
            }
        }
    }
    offset
}

fn format_u32(mut val: u32, buf: &mut [u8]) -> usize {
    if val == 0 {
        if !buf.is_empty() {
            buf[0] = b'0';
            return 1;
        }
        return 0;
    }
    let mut digits = [0u8; 10];
    let mut count = 0;
    while val > 0 && count < 10 {
        digits[count] = b'0' + (val % 10) as u8;
        val /= 10;
        count += 1;
    }
    let write_len = core::cmp::min(count, buf.len());
    for i in 0..write_len {
        buf[i] = digits[count - 1 - i];
    }
    write_len
}

// Syscall handlers with capability enforcement:
fn check_storage_cap(cptr: CPtr, required_rights: CapRights) -> Result<(), u64> {
    let task_id = match crate::task::current_task_id() {
        Some(id) => id as u8,
        None => return Err(!0),
    };
    let slot = match unsafe { crate::capability::lookup_slot(task_id, cptr) } {
        Ok(s) => s,
        Err(_) => return Err(!0),
    };
    if slot.cap_type != CapType::Storage || !slot.rights.contains(required_rights) {
        return Err(!0);
    }
    Ok(())
}

pub fn sys_fs_create(cptr: CPtr, path_ptr: u64, path_len: u64) -> u64 {
    if check_storage_cap(cptr, CapRights::WRITE).is_err() {
        return !0;
    }
    let mut name_buf = [0u8; 32];
    let len = core::cmp::min(path_len as usize, name_buf.len());
    if crate::ipc::copy_user_slice_from(path_ptr, &mut name_buf[..len]).is_err() {
        return !0;
    }
    let name_str = match core::str::from_utf8(&name_buf[..len]) {
        Ok(s) => s.trim_matches(char::from(0)),
        Err(_) => return !0,
    };
    match create_file(name_str) {
        Ok(_) => 0,
        Err(_) => !0,
    }
}

pub fn sys_fs_read(cptr: CPtr, path_ptr: u64, path_len: u64, buf_ptr: u64, max_len: u64) -> u64 {
    if check_storage_cap(cptr, CapRights::READ).is_err() {
        return !0;
    }
    let mut name_buf = [0u8; 32];
    let len = core::cmp::min(path_len as usize, name_buf.len());
    if crate::ipc::copy_user_slice_from(path_ptr, &mut name_buf[..len]).is_err() {
        return !0;
    }
    let name_str = match core::str::from_utf8(&name_buf[..len]) {
        Ok(s) => s.trim_matches(char::from(0)),
        Err(_) => return !0,
    };
    let mut data_buf = [0u8; MAX_FILE_SIZE];
    let read_limit = core::cmp::min(max_len as usize, data_buf.len());
    match read_file(name_str, &mut data_buf[..read_limit]) {
        Ok(bytes_read) => {
            if bytes_read > 0 {
                if crate::ipc::copy_user_slice_to(buf_ptr, &data_buf[..bytes_read]).is_err() {
                    return !0;
                }
            }
            bytes_read as u64
        }
        Err(_) => !0,
    }
}

pub fn sys_fs_write(cptr: CPtr, path_ptr: u64, path_len: u64, buf_ptr: u64, len: u64) -> u64 {
    if check_storage_cap(cptr, CapRights::WRITE).is_err() {
        return !0;
    }
    let mut name_buf = [0u8; 32];
    let p_len = core::cmp::min(path_len as usize, name_buf.len());
    if crate::ipc::copy_user_slice_from(path_ptr, &mut name_buf[..p_len]).is_err() {
        return !0;
    }
    let name_str = match core::str::from_utf8(&name_buf[..p_len]) {
        Ok(s) => s.trim_matches(char::from(0)),
        Err(_) => return !0,
    };
    let mut data_buf = [0u8; MAX_FILE_SIZE];
    let to_write = core::cmp::min(len as usize, data_buf.len());
    if crate::ipc::copy_user_slice_from(buf_ptr, &mut data_buf[..to_write]).is_err() {
        return !0;
    }
    match write_file(name_str, &data_buf[..to_write]) {
        Ok(written) => written as u64,
        Err(_) => !0,
    }
}

pub fn sys_fs_delete(cptr: CPtr, path_ptr: u64, path_len: u64) -> u64 {
    if check_storage_cap(cptr, CapRights::WRITE).is_err() {
        return !0;
    }
    let mut name_buf = [0u8; 32];
    let len = core::cmp::min(path_len as usize, name_buf.len());
    if crate::ipc::copy_user_slice_from(path_ptr, &mut name_buf[..len]).is_err() {
        return !0;
    }
    let name_str = match core::str::from_utf8(&name_buf[..len]) {
        Ok(s) => s.trim_matches(char::from(0)),
        Err(_) => return !0,
    };
    match delete_file(name_str) {
        Ok(_) => 0,
        Err(_) => !0,
    }
}

pub fn sys_fs_list(cptr: CPtr, buf_ptr: u64, max_len: u64) -> u64 {
    if check_storage_cap(cptr, CapRights::READ).is_err() {
        return !0;
    }
    let mut list_buf = [0u8; 1024];
    let limit = core::cmp::min(max_len as usize, list_buf.len());
    let written = list_files(&mut list_buf[..limit]);
    if written > 0 {
        if crate::ipc::copy_user_slice_to(buf_ptr, &list_buf[..written]).is_err() {
            return !0;
        }
    }
    written as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::TEST_LOCK;

    unsafe fn count_used_blocks() -> usize {
        let mut used = [false; CACHE_BLOCKS];
        for inode in INODES.iter() {
            if inode.is_used() {
                for &b in inode.direct_blocks.iter() {
                    let idx = b as usize;
                    if idx >= 5 && idx < CACHE_BLOCKS {
                        assert!(!used[idx], "Duplicate block allocation detected!");
                        used[idx] = true;
                    }
                }
            }
        }
        used.iter().filter(|&&u| u).count()
    }

    unsafe fn reset_fault_injection() {
        TEST_FAIL_ON_WRITE_NUM = None;
        TEST_WRITE_COUNT = 0;
    }

    #[test]
    fn test_janusfs_format_and_seeded_files() {
        let _guard = TEST_LOCK.lock();
        unsafe { reset_fault_injection() };
        format();
        let mut buf = [0u8; 256];
        let read = read_file("/welcome.txt", &mut buf).unwrap();
        assert!(read > 0);
        let content = core::str::from_utf8(&buf[..read]).unwrap();
        assert!(content.contains("Welcome to Janus OS"));
    }

    #[test]
    fn test_janusfs_create_write_read_delete() {
        let _guard = TEST_LOCK.lock();
        unsafe { reset_fault_injection() };
        format();
        assert_eq!(write_file("test.txt", b"Hello JanusFS!"), Ok(14));
        let mut buf = [0u8; 32];
        assert_eq!(read_file("test.txt", &mut buf), Ok(14));
        assert_eq!(&buf[..14], b"Hello JanusFS!");

        assert_eq!(delete_file("test.txt"), Ok(()));
        assert_eq!(read_file("test.txt", &mut buf), Err(()));
    }

    #[test]
    fn test_janusfs_transactional_overwrite_and_shrink() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            reset_fault_injection();
            format();
            let big = [0x41u8; 1024]; // 2 blocks
            assert_eq!(write_file("/ow.txt", &big), Ok(1024));
            let blocks_before = count_used_blocks();

            let small = b"shrunk";
            assert_eq!(write_file("/ow.txt", small), Ok(6));
            let mut buf = [0u8; 1024];
            assert_eq!(read_file("/ow.txt", &mut buf), Ok(6));
            assert_eq!(&buf[..6], b"shrunk");
            // Shrinking must release unused trailing block
            assert_eq!(count_used_blocks(), blocks_before - 1);
        }
    }

    #[test]
    fn test_janusfs_disk_full_create_and_overwrite_rollback() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            reset_fault_injection();
            format();

            // Create a baseline file we will later attempt to overwrite when disk is full
            assert_eq!(write_file("/keep.txt", b"original_data"), Ok(13));

            // Fill remaining data blocks (5..64) using 5-block (2560B) files
            let full_buf = [0x55u8; MAX_FILE_SIZE];
            let mut idx = 0usize;
            loop {
                let name_bytes = [b'f', b'0' + (idx / 10) as u8, b'0' + (idx % 10) as u8];
                let name = core::str::from_utf8(&name_bytes).unwrap();
                if write_file(name, &full_buf).is_err() {
                    break;
                }
                idx += 1;
            }
            let used_at_full = count_used_blocks();

            // 1. Disk-full creation failure must not leave partial inode or leaked blocks
            assert_eq!(write_file("/new_fail.txt", &[0x77u8; 1024]), Err(()));
            let mut rbuf = [0u8; 64];
            assert_eq!(read_file("/new_fail.txt", &mut rbuf), Err(()));
            assert_eq!(count_used_blocks(), used_at_full);

            // 2. Disk-full overwrite failure on /keep.txt (needs +1/+2 blocks) must preserve old contents
            assert_eq!(write_file("/keep.txt", &full_buf), Err(()));
            assert_eq!(read_file("/keep.txt", &mut rbuf), Ok(13));
            assert_eq!(&rbuf[..13], b"original_data");
            assert_eq!(count_used_blocks(), used_at_full);
        }
    }

    #[test]
    fn test_janusfs_mid_operation_io_failure_rollback() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            reset_fault_injection();
            format();

            // Seed a 1-block file
            assert_eq!(write_file("/tx.txt", b"before_io_err"), Ok(13));
            let used_before = count_used_blocks();

            // Inject I/O failure on the 2nd disk write (block 1 of a 2-block overwrite)
            TEST_WRITE_COUNT = 0;
            TEST_FAIL_ON_WRITE_NUM = Some(1);
            let two_block_data = [0x99u8; 1000];
            assert_eq!(write_file("/tx.txt", &two_block_data), Err(()));
            reset_fault_injection();

            // Verify old contents, size, and block count are completely intact
            let mut rbuf = [0u8; 1024];
            assert_eq!(read_file("/tx.txt", &mut rbuf), Ok(13));
            assert_eq!(&rbuf[..13], b"before_io_err");
            assert_eq!(count_used_blocks(), used_before);

            // Inject I/O failure on inode sector flush (3rd write of a 2-block new file)
            TEST_WRITE_COUNT = 0;
            TEST_FAIL_ON_WRITE_NUM = Some(2);
            assert_eq!(write_file("/tx_new.txt", &two_block_data), Err(()));
            reset_fault_injection();

            assert_eq!(read_file("/tx_new.txt", &mut rbuf), Err(()));
            assert_eq!(count_used_blocks(), used_before);

            // Subsequent valid write and delete succeed cleanly
            assert_eq!(write_file("/tx_ok.txt", b"after_recovery"), Ok(14));
            assert_eq!(delete_file("/tx_ok.txt"), Ok(()));
            assert_eq!(count_used_blocks(), used_before);
        }
    }
}
