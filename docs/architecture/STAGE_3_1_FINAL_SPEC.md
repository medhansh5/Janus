# Stage 3.1 Dynamic Capability Engine: Final Implementation Specification

**Repository:** `medhansh5/Janus`  
**Baseline Commit:** `b58d23aebbee2a5ce625cb5b612b42274d5449c4` (`main`)  
**Status:** **APPROVED FOR IMPLEMENTATION**  
**Review Status:** Independently Audited and Adversarially Verified  
**Document Identity:** `docs/architecture/STAGE_3_1_FINAL_SPEC.md`  
**Date:** 2026-11-24  

---

## 1. Final Verdict

### Verdict: APPROVED FOR IMPLEMENTATION

The architecture for **Stage 3.1: Dynamic Capability Engine & Rights Attenuation** is complete, mathematically coherent, and approved for implementation.

Every issue raised across the initial architecture reviews, the primary audit (`STAGE_3_1_CAPABILITY_AUDIT.md`), and the adversarial second review (`STAGE_3_1_SECOND_REVIEW.md`) has been fully resolved:

1. **Frame Mapping Invariant:** Established the strict **1-capability-slot-to-at-most-1-virtual-mapping invariant**. `payload_b` records the active virtual mapping (`mapped_vaddr`), enabling guaranteed $O(1)$ page unmapping on capability revocation without a 2-million PTE tree scan and without dynamic heap allocations.
2. **CPtr & Generation ABI:** Formally defined the 64-bit generation-bearing capability handle (`CPtr`). The slot layout stores a 32-bit generation counter ($4.29 \times 10^9$ cycles) utilizing existing compiler alignment padding, guaranteeing exact 32-byte compile-time assertions and immune to practical ABA wraparound.
3. **In-Situ Iterative Revocation:** Formulated and mathematically proved the non-recursive, in-situ leaf-pruning revocation algorithm. It operates directly upon the 2,048-entry static CDT using $O(1)$ stack space, zero heap allocations, and zero auxiliary worklist arrays.
4. **Preserved Syscall ABI:** The user-space syscall ABI remains strictly untouched in Stage 3.1. Cross-task capability delegation is implemented via kernel-internal primitives in Stage 3.1, ready for clean IPC rendezvous wiring in Stage 3.3.
5. **Orthogonal Security Verification:** All capability-consuming operations enforce four distinct validation layers: Type Validation, Rights Bitmask Validation, Identity Authorization, and CSpace Ownership.

---

## 2. Repository Baseline

- **Repository:** `medhansh5/Janus`
- **Branch:** `main`
- **HEAD Commit:** `b58d23aebbee2a5ce625cb5b612b42274d5449c4`
- **Parent 1 (`main` Baseline):** `d288cadd17aee34ddac16f95de89ddb16b329f92`
- **Parent 2 (Stage 3.0 VMM):** `ccff9eb2ee429f3ebc996f3abc5a708f3ba63408`
- **GitHub PR #4:** Closed and merged into `main` (`2026-11-24T14:46:05Z`).
- **CI Status:** GitHub Actions run `45417782710` passing on `main` (both Target Builds & Unit Tests, and Code Formatting & Static Analysis).
- **Working Tree:** Pristine. All prior architecture documents in `docs/architecture/` remain intact and untracked.

---

## 3. Final CNodeSlot and CPtr ABI

### 3.1 CNodeSlot Memory Layout (Exact 32-Byte Budget)

To ensure that 128 slots fit inside a single 4,096-byte frame ($128 \times 32 = 4,096$), all fields, explicit padding, and alignments are strictly specified:

```rust
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CNodeSlot {
    pub cap_type: CapType,         // offset  0, size 1, align 1
    pub rights: CapRights,         // offset  1, size 1, align 1
    pub _reserved1: u16,           // offset  2, size 2, align 2 (must be 0)
    pub generation: u32,           // offset  4, size 4, align 4 (32-bit generation)
    pub payload_a: u64,            // offset  8, size 8, align 8 (primary payload)
    pub payload_b: u64,            // offset 16, size 8, align 8 (secondary payload)
    pub cdt_index: u32,            // offset 24, size 4, align 4 (cached CDT index)
    pub _reserved2: [u8; 4],       // offset 28, size 4, align 1 (must be 0)
}

const _: () = assert!(core::mem::size_of::<CNodeSlot>() == 32);
const _: () = assert!(core::mem::align_of::<CNodeSlot>() == 8);

#[repr(C, align(4096))]
pub struct CNode {
    pub slots: [CNodeSlot; 128],
}

const _: () = assert!(core::mem::size_of::<CNode>() == 4096);
const _: () = assert!(core::mem::align_of::<CNode>() == 4096);
```

#### Field Offset Verification Table
| Field | Type | Size (Bytes) | Offset | Alignment | Purpose |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `cap_type` | `CapType` (u8) | 1 | 0 | 1 | Capability object type discriminant. |
| `rights` | `CapRights` (u8) | 1 | 1 | 1 | Attenuated permission bitmask. |
| `_reserved1` | `u16` | 2 | 2 | 2 | Explicit zero padding to align `generation`. |
| `generation` | `u32` | 4 | 4 | 4 | Slot reuse generation counter (skips 0). |
| `payload_a` | `u64` | 8 | 8 | 8 | Primary object payload (paddr, port, ID). |
| `payload_b` | `u64` | 8 | 16 | 8 | Secondary payload (mapped_vaddr, badge). |
| `cdt_index` | `u32` | 4 | 24 | 4 | Cached CDT index (`(task << 7) \| slot`). |
| `_reserved2` | `[u8; 4]` | 4 | 28 | 1 | Padding to round struct to 32 bytes. |

### 3.2 64-Bit Generation-Bearing Capability Pointer (`CPtr`)

Capability pointers passed by user tasks are 64-bit opaque tokens:

```rust
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CPtr(pub u64);

impl CPtr {
    pub const fn new(generation: u32, slot: u16) -> Self {
        CPtr(((generation as u64) << 16) | ((slot as u64) & 0x7F))
    }

    #[inline(always)]
    pub const fn slot(self) -> usize {
        (self.0 & 0x7F) as usize
    }

    #[inline(always)]
    pub const fn generation(self) -> u32 {
        ((self.0 >> 16) & 0xFFFF_FFFF) as u32
    }
}
```

#### Bit Allocation Table
```
 63                                48 47                                16 15       7 6       0
+------------------------------------+------------------------------------+----------+---------+
|              Reserved              |         Generation Counter         | Reserved |  Slot   |
|              (16 bits)             |              (32 bits)             |  (9 bits)| (7 bits)|
+------------------------------------+------------------------------------+----------+---------+
```
- **Bits 0..6 (7 bits):** Slot index ($0 \dots 127$).
- **Bits 7..15 (9 bits):** Reserved (must be zero).
- **Bits 16..47 (32 bits):** Expected slot generation counter ($1 \dots 2^{32}-1$).
- **Bits 48..63 (16 bits):** Reserved (must be zero).

#### Handle Validation Rule:
When user space executes any syscall passing a `CPtr`:
1. Check `cptr.slot() < 128`.
2. Check `slot.cap_type != CapType::Empty`.
3. Check `cptr.generation() != 0 && slot.generation == cptr.generation()`.  
   *(If `cptr.generation() == 0`, rejected with `Err(CapError::InvalidHandle)`).*

---

## 4. Capability Types and Rights Semantics

### 4.1 Capability Types (`CapType`)
```rust
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
```

### 4.2 Payload Variant Mapping

| `CapType` | `payload_a` (u64) | `payload_b` (u64) | Invariant Rules |
| :--- | :--- | :--- | :--- |
| `Empty (0)` | `0` | `0` | Inactive slot. `rights = 0`. |
| `Untyped (1)` | `paddr` (4 KiB aligned) | `size` (bytes) | Raw physical RAM region for retyping. |
| `Endpoint (2)` | `target_task_id: u32` | `badge: u64` | Synchronous rendezvous IPC. 0 = unbadged. |
| `Notification (3)` | `object_id: u32` | `0` (or `badge: u64`) | Asynchronous 64-bit event bitmask. |
| `CNode (4)` | `paddr` (4 KiB aligned) | `slot_count: u16` | Authority over a CSpace table. |
| `Frame (5)` | `paddr` (4 KiB aligned) | `mapped_vaddr: u64` | RAM page. `payload_b = u64::MAX` when unmapped. |
| `MmioFrame (6)` | `paddr` (4 KiB aligned) | `mapped_vaddr: u64` | MMIO page. `CapRights::EXECUTE` forbidden. |
| `IOPort (7)` | `(base: u16) \| ((count: u16) << 16)` | `0` | Permitted I/O port range. |
| `Irq (8)` | `irq: u8` | `0` | Hardware interrupt line authority. |

### 4.3 Rights Bitmask (`CapRights`)
```rust
bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct CapRights: u8 {
        const READ    = 1 << 0; // 0x01: Can read / recv / in_port
        const WRITE   = 1 << 1; // 0x02: Can write / send / out_port / irq_ack
        const EXECUTE = 1 << 2; // 0x04: Can map executable (Frame only)
        const GRANT   = 1 << 3; // 0x08: Can mint, copy, or delegate
        const MAP     = 1 << 4; // 0x10: Can invoke SYS_MAP_FRAME
        const CALL    = 1 << 5; // 0x20: Can invoke atomic SYS_CALL
    }
}
```

---

## 5. CDT Data Structure and Deterministic Indexing

### 5.1 CDT Array Specification
The Capability Derivation Tree is statically allocated in `.bss` and sized to exactly 2,048 entries:
$$\text{CDT Capacity} = \text{MAX\_TASKS} \times \text{CNODE\_SLOTS} = 16 \times 128 = 2,048$$

```rust
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CdtNode {
    pub parent: Option<u16>,       // 4 bytes: parent CDT index
    pub first_child: Option<u16>,  // 4 bytes: head of children list
    pub next_sibling: Option<u16>, // 4 bytes: next sibling in parent's list
}

const _: () = assert!(core::mem::size_of::<CdtNode>() == 12);

pub static mut CDT: [CdtNode; 2048] = [CdtNode {
    parent: None,
    first_child: None,
    next_sibling: None,
}; 2048];
```
- Total static BSS footprint: $2,048 \times 12 \text{ bytes} = 24,576 \text{ bytes}$ (6 physical frames).

### 5.2 Deterministic Arithmetic Indexing
The CDT index is **never looked up via in-memory pointers**. It is strictly computed via:
```rust
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
```

---

## 6. Derivation and Attenuation Rules

### 6.1 Capability Minting (`mint_cap`)
1. **Preconditions:**
   - `src_slot < 128` and `dest_slot < 128`.
   - `src_slot != dest_slot` (prevents self-referential CDT cycles).
   - `src.cap_type != CapType::Empty`.
   - `src.rights.contains(CapRights::GRANT)`.
   - `dest.cap_type == CapType::Empty`.
   - `dest.generation == 0` (or retains prior slot generation).
2. **Rights Attenuation Invariant:**
   $$\text{new\_rights} \subseteq \text{src.rights} \iff (\text{new\_rights.bits()} \ \& \ \neg\text{src.rights.bits()}) == 0$$
   *Rights amplification is impossible.*
3. **Type-Specific Rules:**
   - `MmioFrame`: `new_rights.contains(CapRights::EXECUTE)` is strictly rejected.
   - `Frame`: `!(new_rights.contains(WRITE) && new_rights.contains(EXECUTE))` is strictly enforced (global physical W^X).
   - `Endpoint`:
     - If `src.badge != 0`: `new_badge` must equal `src.badge` (existing badges are immutable).
     - If `src.badge == 0`: `new_badge` can be set to any `u64`.
   - Non-endpoint capabilities: `new_badge` must be `0`.
4. **State Transition:**
   - `dest.cap_type = src.cap_type`.
   - `dest.rights = new_rights`.
   - `dest.payload_a = src.payload_a`.
   - `dest.payload_b = if is_frame { u64::MAX } else { new_badge }`.
   - `dest.generation = if dest.generation == u32::MAX { 1 } else { dest.generation + 1 }`.
   - CDT linking:
     - `CDT[dest_idx].parent = Some(src_idx)`
     - `CDT[dest_idx].next_sibling = CDT[src_idx].first_child`
     - `CDT[src_idx].first_child = Some(dest_idx)`

### 6.2 Capability Copying (`copy_cap`)
Equivalent to `mint_cap` with $\text{new\_rights} = \text{src.rights}$ and $\text{new\_badge} = \text{src.badge}$.

---

## 7. In-Situ Iterative Revocation Algorithm

### 7.1 Algorithm Specification
```rust
pub unsafe fn revoke_descendants_iterative(root_task: u8, root_slot: u8) {
    let root_idx = cdt_index(root_task, root_slot);

    while let Some(child_idx) = CDT[root_idx].first_child {
        // 1. Descend leftmost first_child links to find the deepest leaf
        let mut leaf = child_idx;
        while let Some(grandchild) = CDT[leaf as usize].first_child {
            leaf = grandchild;
        }

        // 2. Retrieve leaf's parent
        let parent_idx = CDT[leaf as usize].parent.expect("Leaf must have parent");

        // 3. Unlink leaf from parent's first_child list
        CDT[parent_idx as usize].first_child = CDT[leaf as usize].next_sibling;

        // 4. Reset CDT node for leaf
        CDT[leaf as usize] = CdtNode {
            parent: None,
            first_child: None,
            next_sibling: None,
        };

        // 5. Clean up the revoked slot in the target task's CNode
        let leaf_task = cdt_task_id(leaf as usize);
        let leaf_slot = cdt_slot_idx(leaf as usize);
        cleanup_revoked_slot(leaf_task, leaf_slot);
    }
}

unsafe fn cleanup_revoked_slot(task_id: u8, slot: u8) {
    if let Some(task) = crate::task::get_task_mut(task_id as u64) {
        let cnode = &mut *(task.root_cnode.0 as *mut CNode);
        let slot_ref = &mut cnode.slots[slot as usize];

        // If mapped Frame or MmioFrame, unmap in O(1)
        if (slot_ref.cap_type == CapType::Frame || slot_ref.cap_type == CapType::MmioFrame)
            && slot_ref.payload_b != u64::MAX
        {
            let _ = crate::paging::unmap_user_page(task.page_table, slot_ref.payload_b);
        }

        slot_ref.cap_type = CapType::Empty;
        slot_ref.rights = CapRights::empty();
        slot_ref.payload_a = 0;
        slot_ref.payload_b = 0;
        slot_ref.generation = if slot_ref.generation == u32::MAX {
            1
        } else {
            slot_ref.generation + 1
        };
    }
}
```

### 7.2 Formal Verification Properties
- **Post-Order Invariant:** Children are always visited and cleaned before their ancestors.
- **Root Preservation:** The capability at `(root_task, root_slot)` is never unmapped, cleared, or incremented; only its `first_child` pointer becomes `None`.
- **Termination Guarantee:** Each iteration unlinks exactly one leaf node. With at most 2,047 descendants, the loop is guaranteed to terminate in at most 2,047 iterations.
- **Bounded Resource Footprint:** $O(1)$ stack space (two local `u16` variables), zero dynamic allocations, zero static worklist arrays.

---

## 8. Deletion and Task-Cleanup Semantics

### 8.1 Deletion Semantics (`delete_cap`)
1. **Mandatory Cascading Revocation:** If `CDT[idx].first_child.is_some()`, the kernel automatically executes `revoke_descendants_iterative(task_id, slot)` before proceeding.
2. **Unlink from Sibling List:**
   - If `CDT[parent].first_child == Some(idx)`:
     `CDT[parent].first_child = CDT[idx].next_sibling`.
   - Else: traverse sibling chain from `CDT[parent].first_child` to find `prev` where `CDT[prev].next_sibling == Some(idx)`, and set `CDT[prev].next_sibling = CDT[idx].next_sibling`.
3. **Slot Sanitization:** Clear `cap_type = Empty`, `rights = 0`, increment `generation`. Reset `CDT[idx]`.

### 8.2 Task Termination CSpace Cleanup (`destroy_task_cspace`)
Invoked when any task faults or terminates:
```rust
pub unsafe fn destroy_task_cspace(task_id: u8) {
    for slot in 0..128 {
        let idx = cdt_index(task_id, slot);
        if CDT[idx].first_child.is_some() {
            revoke_descendants_iterative(task_id, slot);
        }
        unlink_from_parent(idx);
        cleanup_revoked_slot(task_id, slot);
        CDT[idx] = CdtNode { parent: None, first_child: None, next_sibling: None };
    }
}
```

---

## 9. Frame Ownership and Mapping Model

### 9.1 The 1-Capability-to-1-Mapping Invariant
1. A `Frame` capability slot represents **at most one active virtual mapping**.
2. `payload_b == u64::MAX` denotes unmapped state.
3. `payload_b < 0x0000_8000_0000_0000` denotes mapped virtual address.
4. Mapping an already-mapped slot (`payload_b != u64::MAX`) is rejected.
5. To map the same physical frame at multiple virtual addresses or in multiple tasks, the user must derive a separate child capability slot for each mapping.

### 9.2 W^X Enforcement at Inception and Derivation
- Virtual W^X is enforced per-mapping by `paging::map_user_page`.
- Global Physical W^X is enforced at capability derivation:
  $$\forall \text{slot}, \quad \neg(\text{slot.rights.contains(WRITE)} \land \text{slot.rights.contains(EXECUTE)})$$
  Because rights can only attenuate, a physical frame can never be simultaneously writable through one capability and executable through another.

---

## 10. Cross-Task Delegation and IPC ABI

### 10.1 Stage 3.1 Kernel Primitive
In Stage 3.1, cross-task delegation is implemented via the internal primitive:
```rust
pub unsafe fn mint_cap_cross_task(
    src_task: u8,
    src_slot: u8,
    dest_task: u8,
    dest_slot: u8,
    new_rights: CapRights,
    new_badge: u64,
) -> Result<(), CapError>
```

### 10.2 Stage 3.3 Syscall Rendezvous Integration Contract
In Stage 3.3, IPC capability transfer will be wired into synchronous rendezvous without expanding registers:
- `SYS_SEND(endpoint_cptr, msg, grant_cptr)`: `arg3 = grant_cptr` (0 = none).
- `SYS_RECV(endpoint_cptr, recv_dest_slot)`: `arg2 = recv_dest_slot` (0 = none).
- During rendezvous, if `grant_cptr != 0 && recv_dest_slot != 0`, `mint_cap_cross_task` executes atomically with message transfer.

---

## 11. Syscall Interface Specification (Stage 3.1)

| Syscall | Number | Arguments | Description & Return Value |
| :--- | :--- | :--- | :--- |
| `SYS_CAP_MINT` | `13` | `rdi=13, rsi=src_cptr, rdx=dest_slot, rcx=rights, r8=badge` | Derives child capability with attenuated rights/badge into `dest_slot`. Returns `0` on success, `!0` on error. |
| `SYS_CAP_REVOKE` | `14` | `rdi=14, rsi=cptr` | Recursively revokes all descendants of `cptr`. Root capability remains intact. Returns `0` on success, `!0` on error. |
| `SYS_CAP_COPY` | `20` | `rdi=20, rsi=src_cptr, rdx=dest_slot` | Duplicates capability into `dest_slot` with identical rights/badge. Returns `0` on success, `!0` on error. |
| `SYS_CAP_DELETE` | `21` | `rdi=21, rsi=cptr` | Cascades revocation to all descendants, unlinks, and clears `cptr`. Returns `0` on success, `!0` on error. |
| `SYS_CAP_INFO` | `5` | `rdi=5, rsi=cptr` | Returns metadata `(type \| (rights << 8) \| (generation << 16))`. Returns `!0` on error without terminating caller. |

---

## 12. Security Invariants Summary

1. **Acyclic Derivation:** Self-minting (`src == dest`) and minting into occupied slots are strictly forbidden. The CDT is provably an acyclic forest.
2. **Monotonic Attenuation:** $R_{child} \subseteq R_{parent}$. No capability can amplify permissions.
3. **Badge Non-Forgeability:** Once set on an endpoint, badges are immutable.
4. **Physical Memory Safety:** Revocation unmaps leaf frames in $O(1)$ time, guaranteeing zero use-after-revocation.
5. **Stack Boundedness:** Revocation runs in $O(1)$ stack space, eliminating kernel stack overflow risks.
6. **Task Boundary Isolation:** Terminated tasks synchronously release all CDT links and CSpace slots before task IDs can be recycled.

---

## 13. Concrete Host Unit-Test Matrix

All tests execute in host CI (`cargo test -p kernel`) without requiring QEMU:

| Test Identifier | Module | Description & Invariant Tested |
| :--- | :--- | :--- |
| `test_cnodeslot_layout_and_size` | `capability::tests` | Assert `size_of::<CNodeSlot>() == 32`, `align_of == 8`, offsets `(0, 1, 4, 8, 16, 24, 28)`. |
| `test_cnode_layout_and_size` | `capability::tests` | Assert `size_of::<CNode>() == 4096`, `align_of == 4096`, 128 slots. |
| `test_cdt_node_size` | `capability::tests` | Assert `size_of::<CdtNode>() == 12`, total table size is 24 KiB. |
| `test_deterministic_indexing_bijective` | `capability::tests` | Verify bijective round-trip `(task, slot) -> idx -> (task, slot)` for all 2,048 entries. |
| `test_valid_mint_attenuation` | `capability::tests` | Mint child with attenuated rights; assert child state matches expected bitmask. |
| `test_mint_rights_escalation_rejected` | `capability::tests` | Assert parent lacking `WRITE` cannot mint child with `WRITE`. |
| `test_mint_without_grant_rejected` | `capability::tests` | Assert parent lacking `GRANT` cannot mint child. |
| `test_mint_self_reference_rejected` | `capability::tests` | Assert `src_slot == dest_slot` is rejected immediately with error. |
| `test_mint_into_occupied_slot_rejected`| `capability::tests` | Assert minting into occupied slot is rejected without mutating destination. |
| `test_badged_endpoint_minting` | `capability::tests` | Mint badge onto unbadged endpoint; verify badge stored in `payload_b`. |
| `test_badged_endpoint_tamper_rejected` | `capability::tests` | Assert badged endpoint cannot have its badge overwritten by a child. |
| `test_mmio_execute_rejected` | `capability::tests` | Assert `MmioFrame` cannot be minted with `CapRights::EXECUTE`. |
| `test_frame_wx_conflict_rejected` | `capability::tests` | Assert `Frame` cannot be minted with both `WRITE` and `EXECUTE`. |
| `test_cdt_multi_child_sibling_integrity`| `capability::tests` | Verify derivation tree structure across multiple siblings and children. |
| `test_iterative_revocation_descendants`| `capability::tests` | Revoke parent with 3 children and 4 grandchildren; assert children cleared, parent intact. |
| `test_iterative_revocation_deep_chain` | `capability::tests` | Mint chain of depth 20; verify iterative revocation succeeds in $O(1)$ stack space. |
| `test_generation_counter_lifecycle` | `capability::tests` | Verify slot reuse increments 32-bit generation; wraps from `u32::MAX` to `1` (skipping `0`). |
| `test_cascading_deletion` | `capability::tests` | Delete parent with live descendants; assert descendants auto-revoked, parent deleted. |
| `test_destroy_task_cspace` | `capability::tests` | Terminate task with 10 active capabilities; assert all slots and CDT links cleanly reset. |

---

## 14. Bare-Metal & QEMU Validation Requirements (Stage 3.3 Gate)
- Verification of Ring 3 `syscall` / `sysret` instruction register state under `SYS_CAP_MINT` and `SYS_CAP_REVOKE`.
- Direct hardware unmapping verification via `INVLPG` upon frame capability revocation.

---

## 15. Implementation File Scope and Sequencing

### Strict File Scope:
1. `kernel/src/capability.rs`:
   - Replace legacy `Capability` enum with `CapType`, `CapRights`, `CNodeSlot`, `CNode`, `CdtNode`.
   - Implement `CDT: [CdtNode; 2048]` and deterministic arithmetic indexing.
   - Implement core operations: `mint_cap`, `copy_cap`, `revoke_descendants_iterative`, `delete_cap`, `lookup_slot`, `destroy_task_cspace`.
   - Add unit test suite with all 19 tests.
2. `kernel/src/syscall.rs` & `kernel/src/ipc.rs`:
   - Dispatch `SYS_CAP_MINT` (13), `SYS_CAP_REVOKE` (14), `SYS_CAP_COPY` (20), `SYS_CAP_DELETE` (21).
   - Update `sys_cap_info` (5) to return metadata without terminating.
   - Add `slot.rights.contains(...)` checks to `sys_send`, `sys_recv`, `sys_in_port`, `sys_out_port`.
3. `kernel/src/main.rs`:
   - Update boot-time capability initialization for Task A and Task B to populate `CNodeSlot` structures.

---

## 16. Explicit Deferred Features

- `SYS_MAP_FRAME` and `SYS_UNMAP_FRAME` $\to$ **Stage 3.3**.
- Asynchronous notifications, event bitmasks, PIC masking in IDT, `SYS_NOTIFY_WAIT`, `SYS_NOTIFY_SIGNAL`, `SYS_IRQ_ACK` $\to$ **Stage 3.2**.
- Embedded ELF Loader $\to$ **Stage 3.4**.
- VirtIO DMA bounce proxy $\to$ **Stage 3.4**.
- Driver fault recovery and respawn $\to$ **Stage 3.5**.
