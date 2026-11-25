use core::mem::size_of;

/// A 64-bit Generation-Bearing Capability Pointer (CPtr).
///
/// Bit allocation:
/// - Bits  0..6  (7 bits) : Slot index within CNode (0..127).
/// - Bits  7..15 (9 bits) : Reserved (must be zero).
/// - Bits 16..47 (32 bits): Expected slot generation counter (1..2^32-1).
/// - Bits 48..63 (16 bits): Reserved (must be zero).
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CPtr(pub u64);

impl CPtr {
    /// Mask covering all reserved bit positions in a 64-bit CPtr:
    /// - Bits 7..15 (9 bits, 0xFF80)
    /// - Bits 48..63 (16 bits, 0xFFFF_0000_0000_0000)
    pub const RESERVED_MASK: u64 = 0xFFFF_0000_0000_FF80;

    pub const fn new(generation: u32, slot: u16) -> Self {
        CPtr(((generation as u64) << 16) | (slot as u64))
    }

    /// Verifies that all reserved bits (7..15 and 48..63) are zero and generation is nonzero.
    #[inline(always)]
    pub const fn is_canonical(self) -> bool {
        (self.0 & Self::RESERVED_MASK) == 0 && self.generation() != 0
    }

    #[inline(always)]
    pub const fn slot(self) -> usize {
        (self.0 & 0x7F) as usize
    }

    #[inline(always)]
    pub const fn generation(self) -> u32 {
        ((self.0 >> 16) & 0xFFFF_FFFF) as u32
    }

    #[inline(always)]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct CapRights: u8 {
        const READ    = 1 << 0;
        const WRITE   = 1 << 1;
        const EXECUTE = 1 << 2;
        const GRANT   = 1 << 3;
        const MAP     = 1 << 4;
        const CALL    = 1 << 5;
    }
}

/// The core capability types in Janus Stage 3.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CapType {
    Empty = 0,
    Untyped = 1,
    Endpoint = 2,
    Notification = 3,
    CNode = 4,
    Frame = 5,
    MmioFrame = 6,
    IOPort = 7,
    Irq = 8,
}

pub const UNMAPPED_VADDR: u64 = u64::MAX;

/// An exact 32-byte CNode slot aligned to 8 bytes.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CNodeSlot {
    pub cap_type: CapType,   // offset  0, size 1, align 1
    pub rights: CapRights,   // offset  1, size 1, align 1
    pub _reserved1: u16,     // offset  2, size 2, align 2 (must be 0)
    pub generation: u32,     // offset  4, size 4, align 4 (32-bit generation)
    pub payload_a: u64,      // offset  8, size 8, align 8
    pub payload_b: u64,      // offset 16, size 8, align 8
    pub cdt_index: u32,      // offset 24, size 4, align 4
    pub _reserved2: [u8; 4], // offset 28, size 4, align 1 (must be 0)
}

impl CNodeSlot {
    pub const fn empty() -> Self {
        Self {
            cap_type: CapType::Empty,
            rights: CapRights::empty(),
            _reserved1: 0,
            generation: 0,
            payload_a: 0,
            payload_b: 0,
            cdt_index: 0,
            _reserved2: [0; 4],
        }
    }

    #[inline(always)]
    pub const fn is_empty(&self) -> bool {
        matches!(self.cap_type, CapType::Empty)
    }
}

// Compile-time static assertions for CNodeSlot layout and bounds
const _: () = {
    assert!(size_of::<CNodeSlot>() == 32);
    assert!(core::mem::align_of::<CNodeSlot>() == 8);
    assert!(core::mem::offset_of!(CNodeSlot, cap_type) == 0);
    assert!(core::mem::offset_of!(CNodeSlot, rights) == 1);
    assert!(core::mem::offset_of!(CNodeSlot, _reserved1) == 2);
    assert!(core::mem::offset_of!(CNodeSlot, generation) == 4);
    assert!(core::mem::offset_of!(CNodeSlot, payload_a) == 8);
    assert!(core::mem::offset_of!(CNodeSlot, payload_b) == 16);
    assert!(core::mem::offset_of!(CNodeSlot, cdt_index) == 24);
    assert!(core::mem::offset_of!(CNodeSlot, _reserved2) == 28);
};

/// A Capability Node (CNode) is a fixed table of 128 capability slots.
/// Fits inside a single 4096-byte frame (128 * 32 = 4096).
pub const CNODE_SLOTS: usize = 128;

#[repr(C, align(4096))]
#[derive(Clone, Copy)]
pub struct CNode {
    pub slots: [CNodeSlot; CNODE_SLOTS],
}

impl CNode {
    pub const fn empty() -> Self {
        Self {
            slots: [CNodeSlot::empty(); CNODE_SLOTS],
        }
    }
}

// Compile-time static assertions for CNode layout and bounds
const _: () = {
    assert!(size_of::<CNode>() == 4096);
    assert!(core::mem::align_of::<CNode>() == 4096);
};

/// A node in the Capability Derivation Tree (CDT).
/// Exactly 12 bytes.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CdtNode {
    pub parent: Option<u16>,       // offset 0, size 4
    pub first_child: Option<u16>,  // offset 4, size 4
    pub next_sibling: Option<u16>, // offset 8, size 4
}

impl CdtNode {
    pub const fn empty() -> Self {
        Self {
            parent: None,
            first_child: None,
            next_sibling: None,
        }
    }
}

const _: () = {
    assert!(size_of::<CdtNode>() == 12);
    assert!(core::mem::offset_of!(CdtNode, parent) == 0);
    assert!(core::mem::offset_of!(CdtNode, first_child) == 4);
    assert!(core::mem::offset_of!(CdtNode, next_sibling) == 8);
};

pub const MAX_TASKS: usize = 16;
pub const TOTAL_CDT_SLOTS: usize = MAX_TASKS * CNODE_SLOTS; // 2048

/// Global static Capability Derivation Tree table (2048 entries, 24 KiB).
pub static mut CDT: [CdtNode; TOTAL_CDT_SLOTS] = [CdtNode::empty(); TOTAL_CDT_SLOTS];

/// Test hook to allow host unit tests to mock per-task CNodes without x86_64 task switching.
pub static mut TEST_CNODES: Option<[CNode; MAX_TASKS]> = None;

#[inline(always)]
pub const fn cdt_index(task_id: u8, slot: u8) -> usize {
    ((task_id as usize) << 7) | ((slot as usize) & 0x7F)
}

#[inline(always)]
pub const fn cdt_task_id(idx: usize) -> u8 {
    (idx >> 7) as u8
}

#[inline(always)]
pub const fn cdt_slot_idx(idx: usize) -> u8 {
    (idx & 0x7F) as u8
}

/// Errors returned by capability operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapError {
    InvalidHandle,
    OutOfBounds,
    EmptySlot,
    OccupiedSlot,
    StaleHandle,
    InvalidRights,
    RightsEscalation,
    SelfMint,
    CycleDetected,
    SlotExhausted,
    InvalidBadge,
    InvalidTask,
    InvalidCNodePointer,
}

/// Resolves a task's CNode with direct-map boundary and alignment checks.
///
/// # Safety
/// Caller must ensure that concurrent modifications to the task table or CNode memory do not cause data races.
pub unsafe fn get_task_cnode_mut(task_id: u8) -> Result<&'static mut CNode, CapError> {
    if task_id as usize >= MAX_TASKS {
        return Err(CapError::InvalidTask);
    }
    if let Some(ref mut test_cnodes) = TEST_CNODES {
        return Ok(&mut test_cnodes[task_id as usize]);
    }
    if let Some(task) = crate::task::get_task_mut(task_id as u64) {
        let vaddr = task.root_cnode.0;
        if vaddr < crate::paging::PHYS_MEMORY_OFFSET
            || (vaddr & 0xFFF) != 0
            || vaddr >= crate::paging::PHYS_MEMORY_OFFSET + crate::paging::DIRECT_MAP_LIMIT
        {
            return Err(CapError::InvalidCNodePointer);
        }
        let ptr = vaddr as *mut CNode;
        Ok(&mut *ptr)
    } else {
        Err(CapError::InvalidTask)
    }
}

/// Safely looks up a capability slot from a task's CSpace.
///
/// # Safety
/// Caller must ensure exclusive or thread-safe access to task memory and CNode structures.
pub unsafe fn lookup_slot(task_id: u8, cptr: CPtr) -> Result<CNodeSlot, CapError> {
    if task_id as usize >= MAX_TASKS {
        return Err(CapError::InvalidTask);
    }
    if !cptr.is_canonical() {
        return Err(CapError::InvalidHandle);
    }
    let slot_idx = cptr.slot();
    let cnode = get_task_cnode_mut(task_id)?;
    let slot = cnode.slots[slot_idx];
    if slot.cap_type == CapType::Empty {
        return Err(CapError::EmptySlot);
    }
    if slot.generation != cptr.generation() {
        return Err(CapError::StaleHandle);
    }
    Ok(slot)
}

/// Internal primitive: mints a derived capability from `(src_task, src_cptr)` into `(dest_task, dest_slot)`.
///
/// # Safety
/// Caller must ensure exclusive or thread-safe access to CNode and CDT structures across tasks.
pub unsafe fn mint_cap_internal(
    src_task: u8,
    src_cptr: CPtr,
    dest_task: u8,
    dest_slot: usize,
    new_rights: CapRights,
    new_badge: u64,
) -> Result<CPtr, CapError> {
    if src_task as usize >= MAX_TASKS || dest_task as usize >= MAX_TASKS {
        return Err(CapError::InvalidTask);
    }
    if dest_slot >= CNODE_SLOTS {
        return Err(CapError::OutOfBounds);
    }
    if !src_cptr.is_canonical() {
        return Err(CapError::InvalidHandle);
    }
    let src_slot = src_cptr.slot();
    if src_task == dest_task && src_slot == dest_slot {
        return Err(CapError::SelfMint);
    }

    let src_cnode = get_task_cnode_mut(src_task)?;
    let src_slot_ref = src_cnode.slots[src_slot];

    if src_slot_ref.cap_type == CapType::Empty {
        return Err(CapError::EmptySlot);
    }
    if src_slot_ref.generation != src_cptr.generation() {
        return Err(CapError::StaleHandle);
    }
    if !src_slot_ref.rights.contains(CapRights::GRANT) {
        return Err(CapError::InvalidRights);
    }
    match src_slot_ref.cap_type {
        CapType::MmioFrame => {
            if new_rights.contains(CapRights::EXECUTE) {
                return Err(CapError::InvalidRights);
            }
        }
        CapType::Frame => {
            if new_rights.contains(CapRights::WRITE) && new_rights.contains(CapRights::EXECUTE) {
                return Err(CapError::InvalidRights);
            }
        }
        CapType::Endpoint => {
            if src_slot_ref.payload_b != 0 && new_badge != src_slot_ref.payload_b {
                return Err(CapError::InvalidBadge);
            }
        }
        _ => {
            if new_badge != 0 {
                return Err(CapError::InvalidBadge);
            }
        }
    }

    if !src_slot_ref.rights.contains(new_rights) {
        return Err(CapError::RightsEscalation);
    }

    let dest_cnode = get_task_cnode_mut(dest_task)?;
    let dest_slot_ref = &mut dest_cnode.slots[dest_slot];
    if dest_slot_ref.cap_type != CapType::Empty {
        return Err(CapError::OccupiedSlot);
    }
    if dest_slot_ref.generation == u32::MAX {
        return Err(CapError::SlotExhausted);
    }

    let src_idx = cdt_index(src_task, src_slot as u8);
    let dest_idx = cdt_index(dest_task, dest_slot as u8);

    if CDT[dest_idx].parent.is_some()
        || CDT[dest_idx].first_child.is_some()
        || CDT[dest_idx].next_sibling.is_some()
    {
        return Err(CapError::CycleDetected);
    }

    let next_gen = dest_slot_ref.generation + 1;

    dest_slot_ref.cap_type = src_slot_ref.cap_type;
    dest_slot_ref.rights = new_rights;
    dest_slot_ref._reserved1 = 0;
    dest_slot_ref.generation = next_gen;
    dest_slot_ref.payload_a = src_slot_ref.payload_a;
    dest_slot_ref.payload_b = match src_slot_ref.cap_type {
        CapType::Frame | CapType::MmioFrame => UNMAPPED_VADDR,
        CapType::Endpoint => {
            if src_slot_ref.payload_b != 0 {
                src_slot_ref.payload_b
            } else {
                new_badge
            }
        }
        _ => 0,
    };
    dest_slot_ref.cdt_index = dest_idx as u32;
    dest_slot_ref._reserved2 = [0; 4];

    CDT[dest_idx].parent = Some(src_idx as u16);
    CDT[dest_idx].next_sibling = CDT[src_idx].first_child;
    CDT[dest_idx].first_child = None;
    CDT[src_idx].first_child = Some(dest_idx as u16);

    Ok(CPtr::new(next_gen, dest_slot as u16))
}

/// Mints a derived capability within the caller's CSpace.
///
/// # Safety
/// Caller must ensure thread safety when modifying CNode slots and the CDT.
pub unsafe fn mint_cap(
    caller_task: u8,
    src_cptr: CPtr,
    dest_slot: usize,
    new_rights: CapRights,
    new_badge: u64,
) -> Result<CPtr, CapError> {
    mint_cap_internal(
        caller_task,
        src_cptr,
        caller_task,
        dest_slot,
        new_rights,
        new_badge,
    )
}

/// Duplicates a capability within the caller's CSpace preserving rights and badge.
///
/// # Safety
/// Caller must ensure thread safety when modifying CNode slots and the CDT.
pub unsafe fn copy_cap(
    caller_task: u8,
    src_cptr: CPtr,
    dest_slot: usize,
) -> Result<CPtr, CapError> {
    let slot = lookup_slot(caller_task, src_cptr)?;
    mint_cap_internal(
        caller_task,
        src_cptr,
        caller_task,
        dest_slot,
        slot.rights,
        slot.payload_b,
    )
}

/// Mints a derived capability across task boundaries (for kernel delegation and Stage 3.3 IPC).
///
/// # Safety
/// Caller must ensure thread safety when modifying CNode slots and the CDT across tasks.
pub unsafe fn mint_cap_cross_task(
    src_task: u8,
    src_cptr: CPtr,
    dest_task: u8,
    dest_slot: usize,
    new_rights: CapRights,
    new_badge: u64,
) -> Result<CPtr, CapError> {
    mint_cap_internal(
        src_task, src_cptr, dest_task, dest_slot, new_rights, new_badge,
    )
}

/// Unlinks a node from its parent's sibling chain.
unsafe fn unlink_node(idx: usize) {
    if let Some(parent_idx) = CDT[idx].parent {
        let parent = parent_idx as usize;
        if CDT[parent].first_child == Some(idx as u16) {
            CDT[parent].first_child = CDT[idx].next_sibling;
        } else {
            let mut curr = CDT[parent].first_child;
            while let Some(c) = curr {
                let c_idx = c as usize;
                if CDT[c_idx].next_sibling == Some(idx as u16) {
                    CDT[c_idx].next_sibling = CDT[idx].next_sibling;
                    break;
                }
                curr = CDT[c_idx].next_sibling;
            }
        }
        CDT[idx].parent = None;
        CDT[idx].next_sibling = None;
    }
}

/// Cleans a revoked or deleted slot in a task's CNode.
unsafe fn cleanup_slot(task_id: u8, slot: u8) {
    if let Ok(cnode) = get_task_cnode_mut(task_id) {
        let slot_ref = &mut cnode.slots[slot as usize];
        if (slot_ref.cap_type == CapType::Frame || slot_ref.cap_type == CapType::MmioFrame)
            && slot_ref.payload_b != UNMAPPED_VADDR
        {
            slot_ref.payload_b = UNMAPPED_VADDR;
        }
        slot_ref.cap_type = CapType::Empty;
        slot_ref.rights = CapRights::empty();
        slot_ref.payload_a = 0;
        slot_ref.payload_b = 0;
        slot_ref.generation = slot_ref.generation.saturating_add(1);
    }
}

/// Read-only preflight validation for iterative revocation.
///
/// Validates the complete reachable subtree in O(1) auxiliary stack space (256-byte bitset)
/// with zero heap allocations and zero recursion before any mutation occurs.
///
/// Ensures:
/// 1. All reachable CDT indices are strictly < TOTAL_CDT_SLOTS.
/// 2. Parent, first-child, and next-sibling pointers are mutually consistent.
/// 3. Sibling chains have matching parent pointers.
/// 4. No cycles or self-referential links exist (enforced by visited bitset and step bounds).
/// 5. Root capability is not pointed to by any descendant.
/// 6. Every descendant CDT index corresponds to an occupied CNode slot owned by that index.
///
/// # Safety
/// Caller must ensure thread-safe access to CDT and CNode structures.
pub unsafe fn validate_revocation_subtree(root_task: u8, root_slot: u8) -> Result<(), CapError> {
    if root_task as usize >= MAX_TASKS || root_slot as usize >= CNODE_SLOTS {
        return Err(CapError::OutOfBounds);
    }
    let root_idx = cdt_index(root_task, root_slot);
    if root_idx >= TOTAL_CDT_SLOTS {
        return Err(CapError::OutOfBounds);
    }

    let first_child_idx = match CDT[root_idx].first_child {
        Some(c) => c as usize,
        None => return Ok(()), // Subtree has no descendants; preflight succeeds vacuously
    };

    if first_child_idx >= TOTAL_CDT_SLOTS {
        return Err(CapError::OutOfBounds);
    }
    if first_child_idx == root_idx {
        return Err(CapError::CycleDetected);
    }

    // Fixed 256-byte bitset tracking visited nodes (2048 bits / 64 = 32 u64 words)
    let mut visited = [0u64; TOTAL_CDT_SLOTS / 64];
    let mut curr = first_child_idx;
    let mut expected_parent = root_idx as u16;
    let mut steps = 0;

    loop {
        steps += 1;
        if steps > 2 * TOTAL_CDT_SLOTS {
            return Err(CapError::CycleDetected);
        }

        if curr >= TOTAL_CDT_SLOTS {
            return Err(CapError::OutOfBounds);
        }
        if curr == root_idx {
            return Err(CapError::CycleDetected);
        }

        // Cycle check: detect if curr was already visited
        let word = curr / 64;
        let bit = 1u64 << (curr % 64);
        if (visited[word] & bit) != 0 {
            return Err(CapError::CycleDetected);
        }
        visited[word] |= bit;

        // Structural check: curr's parent pointer must match expected_parent
        let node = &CDT[curr];
        if node.parent != Some(expected_parent) {
            return Err(CapError::CycleDetected);
        }

        // Ownership and CNode slot consistency check
        let task_id = cdt_task_id(curr);
        let slot_idx = cdt_slot_idx(curr);
        if task_id as usize >= MAX_TASKS || slot_idx as usize >= CNODE_SLOTS {
            return Err(CapError::OutOfBounds);
        }
        let cnode = get_task_cnode_mut(task_id)?;
        let slot = &cnode.slots[slot_idx as usize];
        if slot.cap_type == CapType::Empty {
            return Err(CapError::EmptySlot);
        }
        if slot.cdt_index as usize != curr {
            return Err(CapError::CycleDetected);
        }

        // DFS Advance: prioritize first_child
        if let Some(child) = node.first_child {
            let child_idx = child as usize;
            if child_idx >= TOTAL_CDT_SLOTS {
                return Err(CapError::OutOfBounds);
            }
            if child_idx == root_idx {
                return Err(CapError::CycleDetected);
            }
            expected_parent = curr as u16;
            curr = child_idx;
        } else {
            // Leaf reached: find next sibling or ascend via parent
            let mut up = curr;
            loop {
                if let Some(sibling) = CDT[up].next_sibling {
                    let sib_idx = sibling as usize;
                    if sib_idx >= TOTAL_CDT_SLOTS {
                        return Err(CapError::OutOfBounds);
                    }
                    if sib_idx == root_idx {
                        return Err(CapError::CycleDetected);
                    }
                    let p = match CDT[up].parent {
                        Some(parent_idx) => {
                            if parent_idx as usize >= TOTAL_CDT_SLOTS {
                                return Err(CapError::OutOfBounds);
                            }
                            parent_idx
                        }
                        None => return Err(CapError::CycleDetected),
                    };
                    expected_parent = p;
                    curr = sib_idx;
                    break;
                }

                // No sibling: ascend to parent
                let p = match CDT[up].parent {
                    Some(parent_idx) => {
                        let p_usize = parent_idx as usize;
                        if p_usize >= TOTAL_CDT_SLOTS && p_usize != root_idx {
                            return Err(CapError::OutOfBounds);
                        }
                        p_usize
                    }
                    None => return Err(CapError::CycleDetected),
                };

                if p == root_idx {
                    // Ascended all the way to root with no remaining siblings; preflight complete
                    return Ok(());
                }

                up = p;
            }
        }
    }
}

/// In-situ iterative revocation: revokes all descendants of `(root_task, root_slot)`.
/// Runs in strictly O(1) stack space and zero dynamic allocations.
///
/// # Safety
/// Caller must ensure thread safety and valid CNode references for all affected descendants in the CDT.
pub unsafe fn revoke_descendants_iterative(root_task: u8, root_slot: u8) -> Result<(), CapError> {
    if root_task as usize >= MAX_TASKS || root_slot as usize >= CNODE_SLOTS {
        return Err(CapError::OutOfBounds);
    }

    // 1. Mandatory read-only preflight validation before any mutation
    validate_revocation_subtree(root_task, root_slot)?;

    let root_idx = cdt_index(root_task, root_slot);

    let mut iterations = 0;
    while let Some(child_idx) = CDT[root_idx].first_child {
        iterations += 1;
        if iterations > TOTAL_CDT_SLOTS {
            return Err(CapError::CycleDetected);
        }

        let mut leaf = child_idx as usize;
        let mut descent_steps = 0;
        while let Some(grandchild) = CDT[leaf].first_child {
            leaf = grandchild as usize;
            descent_steps += 1;
            if descent_steps > TOTAL_CDT_SLOTS {
                return Err(CapError::CycleDetected);
            }
        }

        let parent_idx = match CDT[leaf].parent {
            Some(p) => p as usize,
            None => return Err(CapError::CycleDetected),
        };
        CDT[parent_idx].first_child = CDT[leaf].next_sibling;
        CDT[leaf].parent = None;
        CDT[leaf].next_sibling = None;
        CDT[leaf].first_child = None;

        let leaf_task = cdt_task_id(leaf);
        let leaf_slot = cdt_slot_idx(leaf);
        cleanup_slot(leaf_task, leaf_slot);
    }

    Ok(())
}

/// Revokes all descendants of `cptr` within the caller's CSpace.
///
/// # Safety
/// Caller must ensure thread safety when modifying CNode slots and the CDT.
pub unsafe fn revoke_cap(caller_task: u8, cptr: CPtr) -> Result<(), CapError> {
    if caller_task as usize >= MAX_TASKS {
        return Err(CapError::InvalidTask);
    }
    let slot = lookup_slot(caller_task, cptr)?;
    if !slot.rights.contains(CapRights::GRANT) {
        return Err(CapError::InvalidRights);
    }
    revoke_descendants_iterative(caller_task, cptr.slot() as u8)
}

/// Deletes a capability: cascades revocation to descendants, unlinks, and clears the slot.
///
/// # Safety
/// Caller must ensure thread safety when mutating the CDT and CNode slots.
pub unsafe fn delete_cap(task_id: u8, cptr: CPtr) -> Result<(), CapError> {
    if task_id as usize >= MAX_TASKS {
        return Err(CapError::InvalidTask);
    }
    if !cptr.is_canonical() {
        return Err(CapError::InvalidHandle);
    }
    let slot = cptr.slot();
    let cnode = get_task_cnode_mut(task_id)?;
    let slot_ref = &cnode.slots[slot];
    if slot_ref.cap_type == CapType::Empty {
        return Err(CapError::EmptySlot);
    }
    if slot_ref.generation != cptr.generation() {
        return Err(CapError::StaleHandle);
    }

    let idx = cdt_index(task_id, slot as u8);
    if CDT[idx].first_child.is_some() {
        revoke_descendants_iterative(task_id, slot as u8)?;
    }

    unlink_node(idx);
    cleanup_slot(task_id, slot as u8);

    Ok(())
}

/// Validates an I/O port access against a capability slot and returns the checked port address.
pub fn validate_io_port_access(
    slot: &CNodeSlot,
    required_right: CapRights,
    offset: u16,
    width: u8,
) -> Result<u16, CapError> {
    if slot.cap_type != CapType::IOPort {
        return Err(CapError::InvalidRights);
    }
    if !slot.rights.contains(required_right) {
        return Err(CapError::InvalidRights);
    }
    if width != 1 && width != 2 && width != 4 {
        return Err(CapError::OutOfBounds);
    }

    let base = (slot.payload_a & 0xFFFF) as u16;
    let size = ((slot.payload_a >> 16) & 0xFFFF) as u16;

    // Range condition: offset.checked_add(width).is_some_and(|end| end <= size)
    let end_offset = match (offset as u32).checked_add(width as u32) {
        Some(end) if end <= (size as u32) => end,
        _ => return Err(CapError::OutOfBounds),
    };
    let _ = end_offset;

    // Checked addition for base + offset
    let port = match base.checked_add(offset) {
        Some(p) => p,
        None => return Err(CapError::OutOfBounds),
    };

    // Ensure the final accessed byte (port + width - 1) does not exceed 0xFFFF (16-bit I/O port address space)
    match (port as u32).checked_add((width as u32) - 1) {
        Some(lp) if lp <= 0xFFFF => (),
        _ => return Err(CapError::OutOfBounds),
    };

    Ok(port)
}

/// Synchronously cleans up all CSpace slots and CDT links for a terminating task.
///
/// # Safety
/// Caller must ensure that the target task is halted and no concurrent access occurs to its CNode or CDT.
pub unsafe fn destroy_task_cspace(task_id: u8) {
    if task_id as usize >= MAX_TASKS {
        return;
    }
    for slot in 0..CNODE_SLOTS as u8 {
        let idx = cdt_index(task_id, slot);
        if CDT[idx].first_child.is_some() {
            let _ = revoke_descendants_iterative(task_id, slot);
        }
        unlink_node(idx);
        cleanup_slot(task_id, slot);
        CDT[idx] = CdtNode::empty();
    }
}

/// Populates Task A's bootstrap CNode with initial root capabilities.
pub fn init_task_a_bootstrap_cnode(cnode_a: &mut CNode, shared_mem_paddr: u64) {
    cnode_a.slots[1] = CNodeSlot {
        cap_type: CapType::Endpoint,
        rights: CapRights::WRITE | CapRights::GRANT,
        _reserved1: 0,
        generation: 1,
        payload_a: 1,
        payload_b: 0,
        cdt_index: cdt_index(0, 1) as u32,
        _reserved2: [0; 4],
    };
    cnode_a.slots[2] = CNodeSlot {
        cap_type: CapType::Endpoint,
        rights: CapRights::READ | CapRights::GRANT,
        _reserved1: 0,
        generation: 1,
        payload_a: 1,
        payload_b: 0,
        cdt_index: cdt_index(0, 2) as u32,
        _reserved2: [0; 4],
    };
    cnode_a.slots[3] = CNodeSlot {
        cap_type: CapType::Frame,
        rights: CapRights::READ | CapRights::WRITE | CapRights::MAP | CapRights::GRANT,
        _reserved1: 0,
        generation: 1,
        payload_a: shared_mem_paddr,
        payload_b: UNMAPPED_VADDR,
        cdt_index: cdt_index(0, 3) as u32,
        _reserved2: [0; 4],
    };
}

/// Populates Task B's bootstrap CNode with initial root capabilities conforming to POLA-01.
pub fn init_task_b_bootstrap_cnode(
    cnode_b: &mut CNode,
    shared_mem_paddr: u64,
    virtqueue_paddr: u64,
    virtio_io_base: u16,
    virtio_irq: u8,
) {
    cnode_b.slots[1] = CNodeSlot {
        cap_type: CapType::Endpoint,
        rights: CapRights::READ | CapRights::GRANT,
        _reserved1: 0,
        generation: 1,
        payload_a: 0,
        payload_b: 0,
        cdt_index: cdt_index(1, 1) as u32,
        _reserved2: [0; 4],
    };
    cnode_b.slots[2] = CNodeSlot {
        cap_type: CapType::Endpoint,
        rights: CapRights::WRITE | CapRights::GRANT,
        _reserved1: 0,
        generation: 1,
        payload_a: 0,
        payload_b: 0,
        cdt_index: cdt_index(1, 2) as u32,
        _reserved2: [0; 4],
    };
    cnode_b.slots[3] = CNodeSlot {
        cap_type: CapType::Frame,
        rights: CapRights::READ | CapRights::WRITE | CapRights::MAP | CapRights::GRANT,
        _reserved1: 0,
        generation: 1,
        payload_a: shared_mem_paddr,
        payload_b: UNMAPPED_VADDR,
        cdt_index: cdt_index(1, 3) as u32,
        _reserved2: [0; 4],
    };
    cnode_b.slots[4] = CNodeSlot {
        cap_type: CapType::Frame,
        rights: CapRights::READ | CapRights::WRITE | CapRights::MAP | CapRights::GRANT,
        _reserved1: 0,
        generation: 1,
        payload_a: virtqueue_paddr,
        payload_b: UNMAPPED_VADDR,
        cdt_index: cdt_index(1, 4) as u32,
        _reserved2: [0; 4],
    };
    cnode_b.slots[5] = CNodeSlot {
        cap_type: CapType::IOPort,
        rights: CapRights::READ | CapRights::WRITE,
        _reserved1: 0,
        generation: 1,
        payload_a: (virtio_io_base as u64) | (64u64 << 16),
        payload_b: 0,
        cdt_index: cdt_index(1, 5) as u32,
        _reserved2: [0; 4],
    };
    cnode_b.slots[6] = CNodeSlot {
        cap_type: CapType::Irq,
        rights: CapRights::READ | CapRights::WRITE,
        _reserved1: 0,
        generation: 1,
        payload_a: virtio_irq as u64,
        payload_b: 0,
        cdt_index: cdt_index(1, 6) as u32,
        _reserved2: [0; 4],
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    static TEST_LOCK: spin::Mutex<()> = spin::Mutex::new(());

    unsafe fn setup_test_env() {
        TEST_CNODES = Some([CNode::empty(); MAX_TASKS]);
        CDT = [CdtNode::empty(); TOTAL_CDT_SLOTS];
    }

    #[test]
    fn test_cnodeslot_size_and_alignment() {
        assert_eq!(size_of::<CNodeSlot>(), 32);
        assert_eq!(core::mem::align_of::<CNodeSlot>(), 8);
        assert_eq!(core::mem::offset_of!(CNodeSlot, cap_type), 0);
        assert_eq!(core::mem::offset_of!(CNodeSlot, rights), 1);
        assert_eq!(core::mem::offset_of!(CNodeSlot, _reserved1), 2);
        assert_eq!(core::mem::offset_of!(CNodeSlot, generation), 4);
        assert_eq!(core::mem::offset_of!(CNodeSlot, payload_a), 8);
        assert_eq!(core::mem::offset_of!(CNodeSlot, payload_b), 16);
        assert_eq!(core::mem::offset_of!(CNodeSlot, cdt_index), 24);
        assert_eq!(core::mem::offset_of!(CNodeSlot, _reserved2), 28);
    }

    #[test]
    fn test_cnode_size_and_alignment() {
        assert_eq!(size_of::<CNode>(), 4096);
        assert_eq!(core::mem::align_of::<CNode>(), 4096);
        assert_eq!(CNODE_SLOTS, 128);
    }

    #[test]
    fn test_cptr_encode_decode() {
        let cptr1 = CPtr::new(1, 0);
        assert_eq!(cptr1.slot(), 0);
        assert_eq!(cptr1.generation(), 1);

        let cptr2 = CPtr::new(42, 127);
        assert_eq!(cptr2.slot(), 127);
        assert_eq!(cptr2.generation(), 42);

        let cptr3 = CPtr::new(u32::MAX, 64);
        assert_eq!(cptr3.slot(), 64);
        assert_eq!(cptr3.generation(), u32::MAX);
    }

    #[test]
    fn test_valid_capability_lookup() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_env();
            let cnode = get_task_cnode_mut(0).unwrap();
            cnode.slots[1] = CNodeSlot {
                cap_type: CapType::Endpoint,
                rights: CapRights::WRITE | CapRights::GRANT,
                _reserved1: 0,
                generation: 5,
                payload_a: 10,
                payload_b: 0,
                cdt_index: cdt_index(0, 1) as u32,
                _reserved2: [0; 4],
            };

            let lookup = lookup_slot(0, CPtr::new(5, 1)).unwrap();
            assert_eq!(lookup.cap_type, CapType::Endpoint);
            assert_eq!(lookup.rights, CapRights::WRITE | CapRights::GRANT);
            assert_eq!(lookup.generation, 5);
            assert_eq!(lookup.payload_a, 10);
        }
    }

    #[test]
    fn test_invalid_slot_and_task_indices() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_env();
            assert_eq!(lookup_slot(16, CPtr::new(1, 0)), Err(CapError::InvalidTask));
            assert_eq!(lookup_slot(0, CPtr(128)), Err(CapError::InvalidHandle));
            assert_eq!(
                lookup_slot(0, CPtr::new(1, 128)),
                Err(CapError::InvalidHandle)
            );
            assert_eq!(lookup_slot(0, CPtr::new(1, 5)), Err(CapError::EmptySlot));
        }
    }

    #[test]
    fn test_stale_handle_rejection() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_env();
            let cnode = get_task_cnode_mut(0).unwrap();
            cnode.slots[1] = CNodeSlot {
                cap_type: CapType::Endpoint,
                rights: CapRights::WRITE,
                _reserved1: 0,
                generation: 5,
                payload_a: 1,
                payload_b: 0,
                cdt_index: cdt_index(0, 1) as u32,
                _reserved2: [0; 4],
            };

            assert_eq!(lookup_slot(0, CPtr::new(4, 1)), Err(CapError::StaleHandle));
            assert_eq!(lookup_slot(0, CPtr::new(6, 1)), Err(CapError::StaleHandle));
            assert!(lookup_slot(0, CPtr::new(5, 1)).is_ok());
            // Generation 0 must be strictly rejected
            assert_eq!(lookup_slot(0, CPtr(1)), Err(CapError::InvalidHandle));
            assert_eq!(
                lookup_slot(0, CPtr::new(0, 1)),
                Err(CapError::InvalidHandle)
            );
        }
    }

    #[test]
    fn test_generation_advancement_and_exhaustion() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_env();
            let cnode = get_task_cnode_mut(0).unwrap();
            cnode.slots[1] = CNodeSlot {
                cap_type: CapType::Endpoint,
                rights: CapRights::WRITE | CapRights::GRANT,
                _reserved1: 0,
                generation: 1,
                payload_a: 1,
                payload_b: 0,
                cdt_index: cdt_index(0, 1) as u32,
                _reserved2: [0; 4],
            };

            // Mint into empty slot 2 (initial generation 0) advances generation to 1
            let minted_cptr = mint_cap(0, CPtr::new(1, 1), 2, CapRights::WRITE, 0).unwrap();
            assert_eq!(minted_cptr.slot(), 2);
            assert_eq!(minted_cptr.generation(), 1);

            // Delete slot 2 advances generation to 2
            delete_cap(0, minted_cptr).unwrap();
            let cnode = get_task_cnode_mut(0).unwrap();
            assert_eq!(cnode.slots[2].generation, 2);
            assert_eq!(cnode.slots[2].cap_type, CapType::Empty);

            // Force slot to u32::MAX (exhaustion)
            cnode.slots[2].generation = u32::MAX;
            assert_eq!(
                mint_cap(0, CPtr::new(1, 1), 2, CapRights::WRITE, 0),
                Err(CapError::SlotExhausted)
            );
        }
    }

    #[test]
    fn test_valid_mint() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_env();
            let cnode = get_task_cnode_mut(0).unwrap();
            cnode.slots[1] = CNodeSlot {
                cap_type: CapType::Endpoint,
                rights: CapRights::READ | CapRights::WRITE | CapRights::GRANT,
                _reserved1: 0,
                generation: 1,
                payload_a: 1,
                payload_b: 0,
                cdt_index: cdt_index(0, 1) as u32,
                _reserved2: [0; 4],
            };

            let child_cptr = mint_cap(
                0,
                CPtr::new(1, 1),
                2,
                CapRights::READ | CapRights::GRANT,
                0x1234,
            )
            .unwrap();
            let child_slot = lookup_slot(0, child_cptr).unwrap();
            assert_eq!(child_slot.cap_type, CapType::Endpoint);
            assert_eq!(child_slot.rights, CapRights::READ | CapRights::GRANT);
            assert_eq!(child_slot.payload_b, 0x1234);

            let p_idx = cdt_index(0, 1);
            let c_idx = cdt_index(0, 2);
            assert_eq!(CDT[p_idx].first_child, Some(c_idx as u16));
            assert_eq!(CDT[c_idx].parent, Some(p_idx as u16));
        }
    }

    #[test]
    fn test_self_mint_rejection() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_env();
            let cnode = get_task_cnode_mut(0).unwrap();
            cnode.slots[1] = CNodeSlot {
                cap_type: CapType::Endpoint,
                rights: CapRights::READ | CapRights::GRANT,
                _reserved1: 0,
                generation: 1,
                payload_a: 1,
                payload_b: 0,
                cdt_index: cdt_index(0, 1) as u32,
                _reserved2: [0; 4],
            };

            assert_eq!(
                mint_cap(0, CPtr::new(1, 1), 1, CapRights::READ, 0),
                Err(CapError::SelfMint)
            );
        }
    }

    #[test]
    fn test_rights_attenuation() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_env();
            let cnode = get_task_cnode_mut(0).unwrap();
            cnode.slots[1] = CNodeSlot {
                cap_type: CapType::Frame,
                rights: CapRights::READ | CapRights::WRITE | CapRights::GRANT | CapRights::MAP,
                _reserved1: 0,
                generation: 1,
                payload_a: 0x1000_0000,
                payload_b: UNMAPPED_VADDR,
                cdt_index: cdt_index(0, 1) as u32,
                _reserved2: [0; 4],
            };

            let cptr =
                mint_cap(0, CPtr::new(1, 1), 2, CapRights::READ | CapRights::MAP, 0).unwrap();
            let child = lookup_slot(0, cptr).unwrap();
            assert_eq!(child.rights, CapRights::READ | CapRights::MAP);
            assert_eq!(child.payload_b, UNMAPPED_VADDR);
        }
    }

    #[test]
    fn test_rights_escalation_rejection() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_env();
            let cnode = get_task_cnode_mut(0).unwrap();
            cnode.slots[1] = CNodeSlot {
                cap_type: CapType::Endpoint,
                rights: CapRights::READ | CapRights::GRANT,
                _reserved1: 0,
                generation: 1,
                payload_a: 1,
                payload_b: 0,
                cdt_index: cdt_index(0, 1) as u32,
                _reserved2: [0; 4],
            };

            // Attempt to gain WRITE rights
            assert_eq!(
                mint_cap(0, CPtr::new(1, 1), 2, CapRights::READ | CapRights::WRITE, 0),
                Err(CapError::RightsEscalation)
            );
            // Verify destination slot was not modified
            let cnode = get_task_cnode_mut(0).unwrap();
            assert_eq!(cnode.slots[2].cap_type, CapType::Empty);
        }
    }

    #[test]
    fn test_valid_copy() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_env();
            let cnode = get_task_cnode_mut(0).unwrap();
            cnode.slots[1] = CNodeSlot {
                cap_type: CapType::Endpoint,
                rights: CapRights::READ | CapRights::WRITE | CapRights::GRANT,
                _reserved1: 0,
                generation: 1,
                payload_a: 42,
                payload_b: 0xbeef,
                cdt_index: cdt_index(0, 1) as u32,
                _reserved2: [0; 4],
            };

            let copy_cptr = copy_cap(0, CPtr::new(1, 1), 3).unwrap();
            let copy_slot = lookup_slot(0, copy_cptr).unwrap();
            assert_eq!(copy_slot.cap_type, CapType::Endpoint);
            assert_eq!(
                copy_slot.rights,
                CapRights::READ | CapRights::WRITE | CapRights::GRANT
            );
            assert_eq!(copy_slot.payload_a, 42);
            assert_eq!(copy_slot.payload_b, 0xbeef);
        }
    }

    #[test]
    fn test_occupied_destination_rejection() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_env();
            let cnode = get_task_cnode_mut(0).unwrap();
            cnode.slots[1] = CNodeSlot {
                cap_type: CapType::Endpoint,
                rights: CapRights::READ | CapRights::GRANT,
                _reserved1: 0,
                generation: 1,
                payload_a: 1,
                payload_b: 0,
                cdt_index: cdt_index(0, 1) as u32,
                _reserved2: [0; 4],
            };
            cnode.slots[2] = CNodeSlot {
                cap_type: CapType::Endpoint,
                rights: CapRights::READ,
                _reserved1: 0,
                generation: 1,
                payload_a: 2,
                payload_b: 0,
                cdt_index: cdt_index(0, 2) as u32,
                _reserved2: [0; 4],
            };

            assert_eq!(
                mint_cap(0, CPtr::new(1, 1), 2, CapRights::READ, 0),
                Err(CapError::OccupiedSlot)
            );
        }
    }

    #[test]
    fn test_parent_child_cdt_linkage() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_env();
            let cnode = get_task_cnode_mut(0).unwrap();
            cnode.slots[1] = CNodeSlot {
                cap_type: CapType::Endpoint,
                rights: CapRights::READ | CapRights::GRANT,
                _reserved1: 0,
                generation: 1,
                payload_a: 1,
                payload_b: 0,
                cdt_index: cdt_index(0, 1) as u32,
                _reserved2: [0; 4],
            };

            let _c2 = mint_cap(0, CPtr::new(1, 1), 2, CapRights::READ, 0).unwrap();
            assert_eq!(
                CDT[cdt_index(0, 1)].first_child,
                Some(cdt_index(0, 2) as u16)
            );
            assert_eq!(CDT[cdt_index(0, 2)].parent, Some(cdt_index(0, 1) as u16));
            assert_eq!(CDT[cdt_index(0, 2)].next_sibling, None);
        }
    }

    #[test]
    fn test_multiple_siblings() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_env();
            let cnode = get_task_cnode_mut(0).unwrap();
            cnode.slots[1] = CNodeSlot {
                cap_type: CapType::Endpoint,
                rights: CapRights::READ | CapRights::GRANT,
                _reserved1: 0,
                generation: 1,
                payload_a: 1,
                payload_b: 0,
                cdt_index: cdt_index(0, 1) as u32,
                _reserved2: [0; 4],
            };

            mint_cap(0, CPtr::new(1, 1), 2, CapRights::READ, 0).unwrap();
            mint_cap(0, CPtr::new(1, 1), 3, CapRights::READ, 0).unwrap();
            mint_cap(0, CPtr::new(1, 1), 4, CapRights::READ, 0).unwrap();

            // Sibling chain prepends at head of first_child: 4 -> 3 -> 2
            let p_idx = cdt_index(0, 1);
            let c4 = cdt_index(0, 4);
            let c3 = cdt_index(0, 3);
            let c2 = cdt_index(0, 2);

            assert_eq!(CDT[p_idx].first_child, Some(c4 as u16));
            assert_eq!(CDT[c4].next_sibling, Some(c3 as u16));
            assert_eq!(CDT[c3].next_sibling, Some(c2 as u16));
            assert_eq!(CDT[c2].next_sibling, None);
        }
    }

    #[test]
    fn test_deep_iterative_revocation() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_env();
            let cnode = get_task_cnode_mut(0).unwrap();
            cnode.slots[1] = CNodeSlot {
                cap_type: CapType::Endpoint,
                rights: CapRights::READ | CapRights::GRANT,
                _reserved1: 0,
                generation: 1,
                payload_a: 1,
                payload_b: 0,
                cdt_index: cdt_index(0, 1) as u32,
                _reserved2: [0; 4],
            };

            // Mint chain: 1 -> 2 -> 3 -> ... -> 15
            for i in 1..15 {
                mint_cap(
                    0,
                    CPtr::new(1, i),
                    (i + 1) as usize,
                    CapRights::READ | CapRights::GRANT,
                    0,
                )
                .unwrap();
            }

            // Verify root has child
            assert_eq!(
                CDT[cdt_index(0, 1)].first_child,
                Some(cdt_index(0, 2) as u16)
            );

            // Revoke descendants of slot 1
            revoke_descendants_iterative(0, 1).unwrap();

            // Verify root is intact
            let root = lookup_slot(0, CPtr::new(1, 1)).unwrap();
            assert_eq!(root.cap_type, CapType::Endpoint);
            assert_eq!(CDT[cdt_index(0, 1)].first_child, None);

            // Verify all descendants 2..15 were deleted and unlinked
            for i in 2..=15 {
                assert_eq!(lookup_slot(0, CPtr::new(1, i)), Err(CapError::EmptySlot));
                let idx = cdt_index(0, i as u8);
                assert_eq!(CDT[idx].parent, None);
                assert_eq!(CDT[idx].first_child, None);
                assert_eq!(CDT[idx].next_sibling, None);
            }
        }
    }

    #[test]
    fn test_cycle_detection_and_prevention() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_env();
            let cnode = get_task_cnode_mut(0).unwrap();
            cnode.slots[1] = CNodeSlot {
                cap_type: CapType::Endpoint,
                rights: CapRights::READ | CapRights::GRANT,
                _reserved1: 0,
                generation: 1,
                payload_a: 1,
                payload_b: 0,
                cdt_index: cdt_index(0, 1) as u32,
                _reserved2: [0; 4],
            };

            // Corrupt destination CDT node with dangling link
            CDT[cdt_index(0, 2)].parent = Some(99);
            assert_eq!(
                mint_cap(0, CPtr::new(1, 1), 2, CapRights::READ, 0),
                Err(CapError::CycleDetected)
            );
        }
    }

    #[test]
    fn test_parent_deletion_cascades() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_env();
            let cnode = get_task_cnode_mut(0).unwrap();
            cnode.slots[1] = CNodeSlot {
                cap_type: CapType::Endpoint,
                rights: CapRights::READ | CapRights::GRANT,
                _reserved1: 0,
                generation: 1,
                payload_a: 1,
                payload_b: 0,
                cdt_index: cdt_index(0, 1) as u32,
                _reserved2: [0; 4],
            };

            let c2 =
                mint_cap(0, CPtr::new(1, 1), 2, CapRights::READ | CapRights::GRANT, 0).unwrap();
            let _c3 = mint_cap(0, c2, 3, CapRights::READ, 0).unwrap();

            // Deleting slot 1 cascades revocation to slot 2 and slot 3, then deletes slot 1
            delete_cap(0, CPtr::new(1, 1)).unwrap();

            assert_eq!(lookup_slot(0, CPtr::new(1, 1)), Err(CapError::EmptySlot));
            assert_eq!(lookup_slot(0, CPtr::new(1, 2)), Err(CapError::EmptySlot));
            assert_eq!(lookup_slot(0, CPtr::new(1, 3)), Err(CapError::EmptySlot));

            assert_eq!(CDT[cdt_index(0, 1)], CdtNode::empty());
            assert_eq!(CDT[cdt_index(0, 2)], CdtNode::empty());
            assert_eq!(CDT[cdt_index(0, 3)], CdtNode::empty());
        }
    }

    #[test]
    fn test_task_cspace_cleanup() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_env();
            let cnode = get_task_cnode_mut(0).unwrap();
            cnode.slots[1] = CNodeSlot {
                cap_type: CapType::Endpoint,
                rights: CapRights::READ | CapRights::GRANT,
                _reserved1: 0,
                generation: 1,
                payload_a: 1,
                payload_b: 0,
                cdt_index: cdt_index(0, 1) as u32,
                _reserved2: [0; 4],
            };

            // Mint cross-task into Task 1 slot 5
            mint_cap_cross_task(0, CPtr::new(1, 1), 1, 5, CapRights::READ, 0).unwrap();
            assert_eq!(CDT[cdt_index(1, 5)].parent, Some(cdt_index(0, 1) as u16));

            // Destroy Task 0 CSpace
            destroy_task_cspace(0);

            // Assert Task 0 slots are empty and CDT reset
            assert_eq!(lookup_slot(0, CPtr::new(1, 1)), Err(CapError::EmptySlot));
            assert_eq!(CDT[cdt_index(0, 1)], CdtNode::empty());

            // Assert Task 1 slot 5 (derived from Task 0) was also revoked
            assert_eq!(lookup_slot(1, CPtr::new(1, 5)), Err(CapError::EmptySlot));
            assert_eq!(CDT[cdt_index(1, 5)], CdtNode::empty());
        }
    }

    #[test]
    fn test_cross_task_derivation() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_env();
            let cnode = get_task_cnode_mut(0).unwrap();
            cnode.slots[2] = CNodeSlot {
                cap_type: CapType::Frame,
                rights: CapRights::READ | CapRights::WRITE | CapRights::MAP | CapRights::GRANT,
                _reserved1: 0,
                generation: 1,
                payload_a: 0x2000_0000,
                payload_b: UNMAPPED_VADDR,
                cdt_index: cdt_index(0, 2) as u32,
                _reserved2: [0; 4],
            };

            let cptr = mint_cap_cross_task(
                0,
                CPtr::new(1, 2),
                1,
                7,
                CapRights::READ | CapRights::MAP,
                0,
            )
            .unwrap();
            assert_eq!(cptr.slot(), 7);
            let child = lookup_slot(1, cptr).unwrap();
            assert_eq!(child.cap_type, CapType::Frame);
            assert_eq!(child.rights, CapRights::READ | CapRights::MAP);
            assert_eq!(child.payload_a, 0x2000_0000);
            assert_eq!(child.payload_b, UNMAPPED_VADDR);
            assert_eq!(CDT[cdt_index(1, 7)].parent, Some(cdt_index(0, 2) as u16));
        }
    }

    #[test]
    fn test_frame_wx_conflict_rejected() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_env();
            let cnode = get_task_cnode_mut(0).unwrap();
            cnode.slots[1] = CNodeSlot {
                cap_type: CapType::Frame,
                rights: CapRights::READ | CapRights::WRITE | CapRights::EXECUTE | CapRights::GRANT,
                _reserved1: 0,
                generation: 1,
                payload_a: 0x3000_0000,
                payload_b: UNMAPPED_VADDR,
                cdt_index: cdt_index(0, 1) as u32,
                _reserved2: [0; 4],
            };

            assert_eq!(
                mint_cap(
                    0,
                    CPtr::new(1, 1),
                    2,
                    CapRights::WRITE | CapRights::EXECUTE,
                    0
                ),
                Err(CapError::InvalidRights)
            );
        }
    }

    #[test]
    fn test_mmio_execute_rejected() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_env();
            let cnode = get_task_cnode_mut(0).unwrap();
            cnode.slots[1] = CNodeSlot {
                cap_type: CapType::MmioFrame,
                rights: CapRights::READ | CapRights::WRITE | CapRights::GRANT,
                _reserved1: 0,
                generation: 1,
                payload_a: 0xF000_0000,
                payload_b: UNMAPPED_VADDR,
                cdt_index: cdt_index(0, 1) as u32,
                _reserved2: [0; 4],
            };

            assert_eq!(
                mint_cap(0, CPtr::new(1, 1), 2, CapRights::EXECUTE, 0),
                Err(CapError::InvalidRights)
            );
        }
    }

    #[test]
    fn test_badged_endpoint_immutability() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_env();
            let cnode = get_task_cnode_mut(0).unwrap();
            cnode.slots[1] = CNodeSlot {
                cap_type: CapType::Endpoint,
                rights: CapRights::READ | CapRights::WRITE | CapRights::GRANT,
                _reserved1: 0,
                generation: 1,
                payload_a: 1,
                payload_b: 0, // unbadged
                cdt_index: cdt_index(0, 1) as u32,
                _reserved2: [0; 4],
            };

            // Mint badged child
            let c2 = mint_cap(
                0,
                CPtr::new(1, 1),
                2,
                CapRights::READ | CapRights::GRANT,
                0xbeef,
            )
            .unwrap();
            let child = lookup_slot(0, c2).unwrap();
            assert_eq!(child.payload_b, 0xbeef);

            // Attempt to change badge in grandchild must fail
            assert_eq!(
                mint_cap(0, c2, 3, CapRights::READ, 0xcafe),
                Err(CapError::InvalidBadge)
            );

            // Preserving existing badge succeeds
            let c3 = mint_cap(0, c2, 3, CapRights::READ, 0xbeef).unwrap();
            let grandchild = lookup_slot(0, c3).unwrap();
            assert_eq!(grandchild.payload_b, 0xbeef);
        }
    }

    #[test]
    fn test_delete_leaf_unlinks_from_sibling_chain() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_env();
            let cnode = get_task_cnode_mut(0).unwrap();
            cnode.slots[1] = CNodeSlot {
                cap_type: CapType::Endpoint,
                rights: CapRights::READ | CapRights::GRANT,
                _reserved1: 0,
                generation: 1,
                payload_a: 1,
                payload_b: 0,
                cdt_index: cdt_index(0, 1) as u32,
                _reserved2: [0; 4],
            };

            // Mint C2, C3, C4
            mint_cap(0, CPtr::new(1, 1), 2, CapRights::READ, 0).unwrap();
            let c3 = mint_cap(0, CPtr::new(1, 1), 3, CapRights::READ, 0).unwrap();
            mint_cap(0, CPtr::new(1, 1), 4, CapRights::READ, 0).unwrap();

            // Sibling chain: 4 -> 3 -> 2
            // Delete middle sibling C3
            delete_cap(0, c3).unwrap();

            let p_idx = cdt_index(0, 1);
            let c4 = cdt_index(0, 4);
            let c2 = cdt_index(0, 2);

            assert_eq!(CDT[p_idx].first_child, Some(c4 as u16));
            assert_eq!(CDT[c4].next_sibling, Some(c2 as u16));
            assert_eq!(CDT[c2].next_sibling, None);
            assert_eq!(CDT[cdt_index(0, 3)], CdtNode::empty());
        }
    }

    #[test]
    fn test_sec01_generation_zero_rejected_across_all_ops() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_env();
            let cnode = get_task_cnode_mut(0).unwrap();
            cnode.slots[1] = CNodeSlot {
                cap_type: CapType::Endpoint,
                rights: CapRights::READ | CapRights::WRITE | CapRights::GRANT,
                _reserved1: 0,
                generation: 5,
                payload_a: 1,
                payload_b: 0,
                cdt_index: cdt_index(0, 1) as u32,
                _reserved2: [0; 4],
            };

            let gen0_cptr = CPtr(1);
            let gen0_explicit = CPtr::new(0, 1);

            // 1. lookup_slot rejects generation zero
            assert_eq!(lookup_slot(0, gen0_cptr), Err(CapError::InvalidHandle));
            assert_eq!(lookup_slot(0, gen0_explicit), Err(CapError::InvalidHandle));

            // 2. mint_cap rejects generation zero
            assert_eq!(
                mint_cap(0, gen0_cptr, 2, CapRights::READ, 0),
                Err(CapError::InvalidHandle)
            );
            assert_eq!(
                mint_cap(0, gen0_explicit, 2, CapRights::READ, 0),
                Err(CapError::InvalidHandle)
            );

            // 3. copy_cap rejects generation zero
            assert_eq!(copy_cap(0, gen0_cptr, 2), Err(CapError::InvalidHandle));
            assert_eq!(copy_cap(0, gen0_explicit, 2), Err(CapError::InvalidHandle));

            // 4. delete_cap rejects generation zero
            assert_eq!(delete_cap(0, gen0_cptr), Err(CapError::InvalidHandle));
            assert_eq!(delete_cap(0, gen0_explicit), Err(CapError::InvalidHandle));

            // 5. revoke_cap rejects generation zero
            assert_eq!(revoke_cap(0, gen0_cptr), Err(CapError::InvalidHandle));
            assert_eq!(revoke_cap(0, gen0_explicit), Err(CapError::InvalidHandle));

            // Assert destination slot and source slot remain completely untouched
            let cnode = get_task_cnode_mut(0).unwrap();
            assert_eq!(cnode.slots[1].generation, 5);
            assert_eq!(cnode.slots[1].cap_type, CapType::Endpoint);
            assert_eq!(cnode.slots[2].cap_type, CapType::Empty);
            assert_eq!(CDT[cdt_index(0, 1)], CdtNode::empty());
            assert_eq!(CDT[cdt_index(0, 2)], CdtNode::empty());
        }
    }

    #[test]
    fn test_sec02_revoke_handles_and_authority() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_env();
            let cnode = get_task_cnode_mut(0).unwrap();
            cnode.slots[1] = CNodeSlot {
                cap_type: CapType::Endpoint,
                rights: CapRights::READ | CapRights::WRITE | CapRights::GRANT,
                _reserved1: 0,
                generation: 1,
                payload_a: 1,
                payload_b: 0,
                cdt_index: cdt_index(0, 1) as u32,
                _reserved2: [0; 4],
            };

            let c2 =
                mint_cap(0, CPtr::new(1, 1), 2, CapRights::READ | CapRights::GRANT, 0).unwrap();
            let _c3 = mint_cap(0, c2, 3, CapRights::READ, 0).unwrap();

            // 1. Generation-zero handle cannot revoke
            assert_eq!(revoke_cap(0, CPtr::new(0, 1)), Err(CapError::InvalidHandle));
            assert_eq!(revoke_cap(0, CPtr(1)), Err(CapError::InvalidHandle));

            // 2. Non-canonical handle cannot revoke
            let malformed = CPtr((1 << 48) | (1 << 16) | 1);
            assert_eq!(revoke_cap(0, malformed), Err(CapError::InvalidHandle));

            // 3. Stale handle cannot revoke
            assert_eq!(revoke_cap(0, CPtr::new(99, 1)), Err(CapError::StaleHandle));

            // 4. Empty slot cannot revoke
            assert_eq!(revoke_cap(0, CPtr::new(1, 10)), Err(CapError::EmptySlot));

            // 5. Capability lacking GRANT authority cannot revoke
            let child_no_grant = lookup_slot(0, _c3).unwrap();
            assert_eq!(child_no_grant.rights, CapRights::READ);
            assert_eq!(revoke_cap(0, _c3), Err(CapError::InvalidRights));

            // 6. Invalid task cannot revoke
            assert_eq!(revoke_cap(16, CPtr::new(1, 1)), Err(CapError::InvalidTask));

            // Verify children are still intact after all rejected attempts
            assert!(lookup_slot(0, c2).is_ok());
            assert!(lookup_slot(0, _c3).is_ok());

            // 7. Legitimate revocation succeeds with valid handle + GRANT
            revoke_cap(0, CPtr::new(1, 1)).unwrap();

            // Root remains intact
            let root = lookup_slot(0, CPtr::new(1, 1)).unwrap();
            assert_eq!(root.cap_type, CapType::Endpoint);
            assert_eq!(CDT[cdt_index(0, 1)].first_child, None);

            // Descendants are deleted and CDT unlinked
            assert_eq!(lookup_slot(0, c2), Err(CapError::EmptySlot));
            assert_eq!(lookup_slot(0, _c3), Err(CapError::EmptySlot));
            assert_eq!(CDT[cdt_index(0, 2)], CdtNode::empty());
            assert_eq!(CDT[cdt_index(0, 3)], CdtNode::empty());
        }
    }

    #[test]
    fn test_sec03_canonical_cptr_reserved_bits() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_env();
            let cnode = get_task_cnode_mut(0).unwrap();
            cnode.slots[1] = CNodeSlot {
                cap_type: CapType::Endpoint,
                rights: CapRights::READ | CapRights::WRITE | CapRights::GRANT,
                _reserved1: 0,
                generation: 1,
                payload_a: 1,
                payload_b: 0,
                cdt_index: cdt_index(0, 1) as u32,
                _reserved2: [0; 4],
            };

            // Valid canonical handles
            let valid_1 = CPtr::new(1, 1);
            let valid_127 = CPtr::new(1, 127);
            assert!(valid_1.is_canonical());
            assert!(valid_127.is_canonical());
            assert!(lookup_slot(0, valid_1).is_ok());

            // Generation 0 is non-canonical
            let gen0 = CPtr::new(0, 1);
            assert!(!gen0.is_canonical());
            assert_eq!(lookup_slot(0, gen0), Err(CapError::InvalidHandle));

            // Reserved bits 7..15 independently set
            for bit in 7..=15 {
                let bad = CPtr(((1u64) << 16) | (1u64 << bit) | 1);
                assert!(!bad.is_canonical());
                assert_eq!(lookup_slot(0, bad), Err(CapError::InvalidHandle));
            }

            // Reserved bits 48..63 independently set
            for bit in 48..=63 {
                let bad = CPtr(((1u64) << bit) | ((1u64) << 16) | 1);
                assert!(!bad.is_canonical());
                assert_eq!(lookup_slot(0, bad), Err(CapError::InvalidHandle));
            }

            // Multiple reserved bits set
            let multi_reserved = CPtr(CPtr::RESERVED_MASK | (1 << 16) | 1);
            assert!(!multi_reserved.is_canonical());
            assert_eq!(lookup_slot(0, multi_reserved), Err(CapError::InvalidHandle));

            // Slot 128 (bit 7 set) is rejected
            let slot_128 = CPtr::new(1, 128);
            assert!(!slot_128.is_canonical());
            assert_eq!(lookup_slot(0, slot_128), Err(CapError::InvalidHandle));
        }
    }

    #[test]
    fn test_sec04_io_port_validation() {
        let port_cap = CNodeSlot {
            cap_type: CapType::IOPort,
            rights: CapRights::READ | CapRights::WRITE,
            _reserved1: 0,
            generation: 1,
            payload_a: 0x0004_1000, // base = 0x1000, size = 4 ports (0x1000..0x1003)
            payload_b: 0,
            cdt_index: 0,
            _reserved2: [0; 4],
        };

        // 1. Valid accesses
        assert_eq!(
            validate_io_port_access(&port_cap, CapRights::READ, 0, 1),
            Ok(0x1000)
        );
        assert_eq!(
            validate_io_port_access(&port_cap, CapRights::READ, 0, 2),
            Ok(0x1000)
        );
        assert_eq!(
            validate_io_port_access(&port_cap, CapRights::READ, 0, 4),
            Ok(0x1000)
        );
        assert_eq!(
            validate_io_port_access(&port_cap, CapRights::WRITE, 2, 2),
            Ok(0x1002)
        );
        assert_eq!(
            validate_io_port_access(&port_cap, CapRights::WRITE, 3, 1),
            Ok(0x1003)
        );

        // 2. Multi-byte boundary overruns
        // offset 3 with width 2 accesses 0x1003..0x1004 (exceeds size 4)
        assert_eq!(
            validate_io_port_access(&port_cap, CapRights::READ, 3, 2),
            Err(CapError::OutOfBounds)
        );
        // offset 1 with width 4 accesses 0x1001..0x1004 (exceeds size 4)
        assert_eq!(
            validate_io_port_access(&port_cap, CapRights::READ, 1, 4),
            Err(CapError::OutOfBounds)
        );
        // offset 4 with width 1
        assert_eq!(
            validate_io_port_access(&port_cap, CapRights::READ, 4, 1),
            Err(CapError::OutOfBounds)
        );

        // 3. Offset arithmetic overflow
        assert_eq!(
            validate_io_port_access(&port_cap, CapRights::READ, 0xFFFF, 1),
            Err(CapError::OutOfBounds)
        );
        assert_eq!(
            validate_io_port_access(&port_cap, CapRights::READ, 0xFFFF, 2),
            Err(CapError::OutOfBounds)
        );

        // 4. Base + offset arithmetic overflow
        let high_base_cap = CNodeSlot {
            cap_type: CapType::IOPort,
            rights: CapRights::READ,
            _reserved1: 0,
            generation: 1,
            payload_a: 0x0020_FFF0, // base = 0xFFF0, size = 0x20
            payload_b: 0,
            cdt_index: 0,
            _reserved2: [0; 4],
        };
        // 0xFFF0 + 0x15 would overflow u16
        assert_eq!(
            validate_io_port_access(&high_base_cap, CapRights::READ, 0x15, 1),
            Err(CapError::OutOfBounds)
        );

        // 5. Access exceeding 16-bit port space (0xFFFF)
        let end_of_space_cap = CNodeSlot {
            cap_type: CapType::IOPort,
            rights: CapRights::READ,
            _reserved1: 0,
            generation: 1,
            payload_a: 0x0004_FFFE, // base = 0xFFFE, size = 4
            payload_b: 0,
            cdt_index: 0,
            _reserved2: [0; 4],
        };
        // 0xFFFE with width 4 would access up to 0x10001 (beyond 0xFFFF)
        assert_eq!(
            validate_io_port_access(&end_of_space_cap, CapRights::READ, 0, 4),
            Err(CapError::OutOfBounds)
        );
        // 0xFFFE with width 2 accesses 0xFFFE..0xFFFF (fits inside 16-bit space)
        assert_eq!(
            validate_io_port_access(&end_of_space_cap, CapRights::READ, 0, 2),
            Ok(0xFFFE)
        );

        // 6. Missing rights
        let read_only_cap = CNodeSlot {
            cap_type: CapType::IOPort,
            rights: CapRights::READ,
            _reserved1: 0,
            generation: 1,
            payload_a: 0x0004_1000,
            payload_b: 0,
            cdt_index: 0,
            _reserved2: [0; 4],
        };
        assert_eq!(
            validate_io_port_access(&read_only_cap, CapRights::WRITE, 0, 1),
            Err(CapError::InvalidRights)
        );

        // 7. Wrong capability type
        let frame_cap = CNodeSlot {
            cap_type: CapType::Frame,
            rights: CapRights::READ | CapRights::WRITE,
            _reserved1: 0,
            generation: 1,
            payload_a: 0x1000,
            payload_b: UNMAPPED_VADDR,
            cdt_index: 0,
            _reserved2: [0; 4],
        };
        assert_eq!(
            validate_io_port_access(&frame_cap, CapRights::READ, 0, 1),
            Err(CapError::InvalidRights)
        );
    }

    #[test]
    fn test_pola01_task_b_bootstrap_rights() {
        let mut cnode_b = CNode::empty();
        init_task_b_bootstrap_cnode(&mut cnode_b, 0x1000, 0x2000, 0xC000, 11);

        // Verify slot 5 (IOPort)
        let io_port = cnode_b.slots[5];
        assert_eq!(io_port.cap_type, CapType::IOPort);
        assert_eq!(io_port.rights, CapRights::READ | CapRights::WRITE);
        assert!(!io_port.rights.contains(CapRights::GRANT));
        assert_eq!(io_port.generation, 1);
        assert_eq!(io_port.payload_a, 0xC000 | (64u64 << 16));
        assert_eq!(io_port.payload_b, 0);
        assert_eq!(io_port.cdt_index, cdt_index(1, 5) as u32);

        // Verify slot 6 (Irq)
        let irq_cap = cnode_b.slots[6];
        assert_eq!(irq_cap.cap_type, CapType::Irq);
        assert_eq!(irq_cap.rights, CapRights::READ | CapRights::WRITE);
        assert!(!irq_cap.rights.contains(CapRights::GRANT));
        assert_eq!(irq_cap.generation, 1);
        assert_eq!(irq_cap.payload_a, 11);
        assert_eq!(irq_cap.payload_b, 0);
        assert_eq!(irq_cap.cdt_index, cdt_index(1, 6) as u32);

        // Verify operational slots populated by production helper
        assert_eq!(cnode_b.slots[1].cap_type, CapType::Endpoint);
        assert_eq!(cnode_b.slots[2].cap_type, CapType::Endpoint);
        assert_eq!(cnode_b.slots[3].cap_type, CapType::Frame);
        assert_eq!(cnode_b.slots[4].cap_type, CapType::Frame);
    }

    #[test]
    fn test_revocation_preflight_valid_deep_tree() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_env();
            let cnode = get_task_cnode_mut(0).unwrap();
            cnode.slots[1] = CNodeSlot {
                cap_type: CapType::Endpoint,
                rights: CapRights::READ | CapRights::WRITE | CapRights::GRANT,
                _reserved1: 0,
                generation: 1,
                payload_a: 10,
                payload_b: 0,
                cdt_index: cdt_index(0, 1) as u32,
                _reserved2: [0; 4],
            };

            let c2 =
                mint_cap(0, CPtr::new(1, 1), 2, CapRights::READ | CapRights::GRANT, 0).unwrap();
            let c3 = mint_cap(0, c2, 3, CapRights::READ, 0).unwrap();
            let c4 = mint_cap(0, CPtr::new(1, 1), 4, CapRights::READ, 0).unwrap();

            // Validate preflight passes
            assert_eq!(validate_revocation_subtree(0, 1), Ok(()));

            // Execute full revocation
            revoke_cap(0, CPtr::new(1, 1)).unwrap();

            // Root remains intact
            let root = lookup_slot(0, CPtr::new(1, 1)).unwrap();
            assert_eq!(root.cap_type, CapType::Endpoint);
            assert_eq!(root.generation, 1);

            // Descendants are deleted and slots empty
            assert_eq!(lookup_slot(0, c2), Err(CapError::EmptySlot));
            assert_eq!(lookup_slot(0, c3), Err(CapError::EmptySlot));
            assert_eq!(lookup_slot(0, c4), Err(CapError::EmptySlot));

            // Descendant CDT nodes are empty
            assert_eq!(CDT[cdt_index(0, 2)], CdtNode::empty());
            assert_eq!(CDT[cdt_index(0, 3)], CdtNode::empty());
            assert_eq!(CDT[cdt_index(0, 4)], CdtNode::empty());
        }
    }

    #[test]
    fn test_revocation_preflight_out_of_bounds_child_rejected_without_mutation() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_env();
            let cnode = get_task_cnode_mut(0).unwrap();
            cnode.slots[1] = CNodeSlot {
                cap_type: CapType::Endpoint,
                rights: CapRights::READ | CapRights::GRANT,
                _reserved1: 0,
                generation: 1,
                payload_a: 1,
                payload_b: 0,
                cdt_index: cdt_index(0, 1) as u32,
                _reserved2: [0; 4],
            };

            let _c2 = mint_cap(0, CPtr::new(1, 1), 2, CapRights::READ, 0).unwrap();

            // 1. Inject malformed pointer state (child index 5000 >= TOTAL_CDT_SLOTS)
            CDT[cdt_index(0, 2)].first_child = Some(5000);

            // 2. Immediately snapshot complete CNode and CDT state in the malformed pre-call state
            let cnode_snapshot = *cnode;
            let cdt_snapshot = CDT;

            // 3. Attempt revoke: preflight must detect out-of-bounds child and reject
            assert_eq!(revoke_cap(0, CPtr::new(1, 1)), Err(CapError::OutOfBounds));

            // 4. Compare post-error state directly against the malformed pre-call snapshot (no restoration before comparison)
            let cnode_after = *get_task_cnode_mut(0).unwrap();
            for i in 0..CNODE_SLOTS {
                assert_eq!(cnode_after.slots[i], cnode_snapshot.slots[i]);
            }
            for i in 0..TOTAL_CDT_SLOTS {
                assert_eq!(CDT[i], cdt_snapshot[i]);
            }

            // 5. Clean up for test isolation after comparison
            CDT[cdt_index(0, 2)].first_child = None;
        }
    }

    #[test]
    fn test_revocation_preflight_malformed_parent_rejected_without_mutation() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_env();
            let cnode = get_task_cnode_mut(0).unwrap();
            cnode.slots[1] = CNodeSlot {
                cap_type: CapType::Endpoint,
                rights: CapRights::READ | CapRights::GRANT,
                _reserved1: 0,
                generation: 1,
                payload_a: 1,
                payload_b: 0,
                cdt_index: cdt_index(0, 1) as u32,
                _reserved2: [0; 4],
            };

            let _c2 = mint_cap(0, CPtr::new(1, 1), 2, CapRights::READ, 0).unwrap();

            // 1. Inject malformed parent link
            CDT[cdt_index(0, 2)].parent = Some(999);

            // 2. Immediately snapshot complete CNode and CDT state in the malformed pre-call state
            let cnode_snapshot = *cnode;
            let cdt_snapshot = CDT;

            // 3. Attempt revoke: preflight must detect inconsistent parent and reject
            assert_eq!(revoke_cap(0, CPtr::new(1, 1)), Err(CapError::CycleDetected));

            // 4. Compare post-error state directly against the malformed pre-call snapshot (no restoration before comparison)
            let cnode_after = *get_task_cnode_mut(0).unwrap();
            for i in 0..CNODE_SLOTS {
                assert_eq!(cnode_after.slots[i], cnode_snapshot.slots[i]);
            }
            for i in 0..TOTAL_CDT_SLOTS {
                assert_eq!(CDT[i], cdt_snapshot[i]);
            }

            // 5. Clean up for test isolation after comparison
            CDT[cdt_index(0, 2)].parent = Some(cdt_index(0, 1) as u16);
        }
    }

    #[test]
    fn test_revocation_preflight_cycle_rejected_without_mutation() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_env();
            let cnode = get_task_cnode_mut(0).unwrap();
            cnode.slots[1] = CNodeSlot {
                cap_type: CapType::Endpoint,
                rights: CapRights::READ | CapRights::GRANT,
                _reserved1: 0,
                generation: 1,
                payload_a: 1,
                payload_b: 0,
                cdt_index: cdt_index(0, 1) as u32,
                _reserved2: [0; 4],
            };

            let c2 =
                mint_cap(0, CPtr::new(1, 1), 2, CapRights::READ | CapRights::GRANT, 0).unwrap();
            let _c3 = mint_cap(0, c2, 3, CapRights::READ, 0).unwrap();

            // 1. Inject cyclic link: node 3's first_child points back to node 2
            CDT[cdt_index(0, 3)].first_child = Some(cdt_index(0, 2) as u16);

            // 2. Immediately snapshot complete CNode and CDT state in the malformed pre-call state
            let cnode_snapshot = *cnode;
            let cdt_snapshot = CDT;

            // 3. Attempt revoke: preflight must detect cycle and reject before mutation
            assert_eq!(revoke_cap(0, CPtr::new(1, 1)), Err(CapError::CycleDetected));

            // 4. Compare post-error state directly against the malformed pre-call snapshot (no restoration before comparison)
            let cnode_after = *get_task_cnode_mut(0).unwrap();
            for i in 0..CNODE_SLOTS {
                assert_eq!(cnode_after.slots[i], cnode_snapshot.slots[i]);
            }
            for i in 0..TOTAL_CDT_SLOTS {
                assert_eq!(CDT[i], cdt_snapshot[i]);
            }

            // 5. Clean up for test isolation after comparison
            CDT[cdt_index(0, 3)].first_child = None;
        }
    }

    #[test]
    fn test_revocation_preflight_empty_slot_rejected_without_mutation() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_env();
            let cnode = get_task_cnode_mut(0).unwrap();
            cnode.slots[1] = CNodeSlot {
                cap_type: CapType::Endpoint,
                rights: CapRights::READ | CapRights::GRANT,
                _reserved1: 0,
                generation: 1,
                payload_a: 1,
                payload_b: 0,
                cdt_index: cdt_index(0, 1) as u32,
                _reserved2: [0; 4],
            };

            let _c2 = mint_cap(0, CPtr::new(1, 1), 2, CapRights::READ, 0).unwrap();

            // 1. Inject malformed slot state: CDT references slot 2, but slot 2 is cleared to Empty
            cnode.slots[2] = CNodeSlot::empty();

            // 2. Immediately snapshot complete CNode and CDT state in the malformed pre-call state
            let cnode_snapshot = *cnode;
            let cdt_snapshot = CDT;

            // 3. Attempt revoke: preflight must detect inconsistent slot and reject
            assert_eq!(revoke_cap(0, CPtr::new(1, 1)), Err(CapError::EmptySlot));

            // 4. Compare post-error state directly against the malformed pre-call snapshot (no restoration before comparison)
            let cnode_after = *get_task_cnode_mut(0).unwrap();
            for i in 0..CNODE_SLOTS {
                assert_eq!(cnode_after.slots[i], cnode_snapshot.slots[i]);
            }
            for i in 0..TOTAL_CDT_SLOTS {
                assert_eq!(CDT[i], cdt_snapshot[i]);
            }

            // 5. Clean up for test isolation after comparison
            cnode.slots[2] = CNodeSlot {
                cap_type: CapType::Endpoint,
                rights: CapRights::READ,
                _reserved1: 0,
                generation: 1,
                payload_a: 1,
                payload_b: 0,
                cdt_index: cdt_index(0, 2) as u32,
                _reserved2: [0; 4],
            };
        }
    }

    #[test]
    fn test_revocation_preflight_mismatched_cdt_index_rejected_without_mutation() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_env();
            let cnode = get_task_cnode_mut(0).unwrap();
            cnode.slots[1] = CNodeSlot {
                cap_type: CapType::Endpoint,
                rights: CapRights::READ | CapRights::GRANT,
                _reserved1: 0,
                generation: 1,
                payload_a: 1,
                payload_b: 0,
                cdt_index: cdt_index(0, 1) as u32,
                _reserved2: [0; 4],
            };

            let _c2 = mint_cap(0, CPtr::new(1, 1), 2, CapRights::READ, 0).unwrap();

            // 1. Inject malformed slot state: slot 2's cdt_index does not match its actual index
            cnode.slots[2].cdt_index = 999;

            // 2. Immediately snapshot complete CNode and CDT state in the malformed pre-call state
            let cnode_snapshot = *cnode;
            let cdt_snapshot = CDT;

            // 3. Attempt revoke: preflight must detect inconsistent cdt_index and reject
            assert_eq!(revoke_cap(0, CPtr::new(1, 1)), Err(CapError::CycleDetected));

            // 4. Compare post-error state directly against the malformed pre-call snapshot (no restoration before comparison)
            let cnode_after = *get_task_cnode_mut(0).unwrap();
            for i in 0..CNODE_SLOTS {
                assert_eq!(cnode_after.slots[i], cnode_snapshot.slots[i]);
            }
            for i in 0..TOTAL_CDT_SLOTS {
                assert_eq!(CDT[i], cdt_snapshot[i]);
            }

            // 5. Clean up for test isolation after comparison
            cnode.slots[2].cdt_index = cdt_index(0, 2) as u32;
        }
    }
}
