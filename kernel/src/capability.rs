use core::mem::size_of;

/// A Capability Pointer (CPtr) is an opaque index into a CNode.
/// Userspace processes use these to reference their capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CPtr(pub usize);

/// Memory permission flags.
bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct MemoryFlags: u64 {
        const READ = 1 << 0;
        const WRITE = 1 << 1;
        const EXECUTE = 1 << 2;
    }
}

/// The core capability types in Janus.
#[derive(Debug, Clone, Copy)]
pub enum Capability {
    /// An empty slot in the CNode.
    Empty,
    /// An Untyped capability representing raw, unallocated physical memory.
    /// The init process must explicitly retype this into usable objects.
    Untyped {
        paddr: u64,
        size: u64,
    },
    /// (Placeholder) An IPC Endpoint capability for synchronous messaging.
    Endpoint {
        target_task_id: u64,
    },
    /// (Placeholder) A capability granting access to another CNode.
    CNode {
        paddr: u64,
        size: u64,
    }
}

/// A Capability Node (CNode) is a table of capabilities.
/// For v0, a CNode is a fixed-size array of 128 slots, designed to fit cleanly inside a 4KB physical frame.
pub const CNODE_SLOTS: usize = 128;

#[repr(C, align(4096))]
pub struct CNode {
    pub slots: [Capability; CNODE_SLOTS],
}

impl CNode {
    /// Creates a new CNode with all empty slots.
    pub const fn empty() -> Self {
        // Since Capability is not Copy due to being a complex enum (wait, we derived Copy!), 
        // we can initialize it like this.
        Self {
            slots: [Capability::Empty; CNODE_SLOTS],
        }
    }
}

/// Statically assert that a CNode fits inside a single 4KB page.
const _: () = assert!(size_of::<CNode>() <= 4096);
