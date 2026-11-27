# Stage 3.1 Capability Engine: Independent Adversarial Review

**Repository:** `medhansh5/Janus`  
**Baseline Commit:** `b58d23aebbee2a5ce625cb5b612b42274d5449c4` (`main`)  
**Primary Audit Reviewed:** `docs/architecture/STAGE_3_1_CAPABILITY_AUDIT.md`  
**Mode:** STRICTLY READ-ONLY — INDEPENDENT ADVERSARIAL REVIEW  
**Review Date:** 2026-11-24  

---

## 1. Repository Baseline Verification

Direct inspection of the Git repository state and GitHub remote confirms the exact baseline:

- **Repository:** `medhansh5/Janus`
- **Branch:** `main`
- **HEAD Commit:** `b58d23aebbee2a5ce625cb5b612b42274d5449c4` (merge commit of PR #4)
- **Lineage:** Parent 1 = `d288cad` (M1–M2B hardening), Parent 2 = `ccff9eb` (Stage 3.0 Dynamic User VMM)
- **Upstream Tracking:** `origin/main` (clean, fully synchronized)
- **PR #4 Status:** `CLOSED` / `MERGED` at `2026-11-24T14:46:05Z`
- **CI Status on `main` (`b58d23a`):**
  - Run ID: `45417782710`
  - *Target Builds & Host Unit Tests:* `success`
  - *Code Formatting & Static Analysis:* `success`
- **Tracked Working Tree:** Clean. All architecture documents under `docs/architecture/` remain untracked and completely unmodified.

---

## 2. Independent Verdict

### Verdict: APPROVED WITH CONDITIONS

The Stage 3.1 capability architecture—built upon a 32-byte `CNodeSlot` budget, 128 slots per 4 KiB page, a static 2,048-entry deterministic Capability Derivation Tree (CDT) indexed by `(task_id << 7) | slot_index`, fine-grained `CapRights` bitmasks, non-zero generation counters, and badged endpoints—is fundamentally sound and well suited to Janus's uniprocessor microkernel design.

This adversarial review independently scrutinized every claim in the primary audit (`STAGE_3_1_CAPABILITY_AUDIT.md`) and challenged its foundational assumptions:

1. **Findings Confirmed (9 of 11):** Findings F-01, F-02, F-03, F-05, F-06, F-08, F-09, F-10, and F-11 are confirmed as genuine design or code defects.
2. **Finding Reclassified as Overstated (1 of 11):** Finding F-04 (Stale CDT links on task termination) is **Confirmed but Overstated** (reclassified from High to Medium). In the existing code base (`kernel/src/task.rs`), tasks are statically initialized at boot and never dynamically destroyed or recycled. Task recycling does not exist today; F-04 is an essential design precondition for Stage 3.5 driver fault recovery rather than an immediately exploitable vulnerability.
3. **Finding Reclassified as Partially Confirmed with Deeper Defect Identified (1 of 11):** Finding F-07 (16-bit generation wraparound) is **Partially Confirmed**, but the primary audit failed to uncover the deeper defect: `CPtr` in `kernel/src/capability.rs` and the syscall interface passes only a raw slot index (`0..127`). **Generation is currently never verified against any user-supplied handle**. Without handle verification, generation counters are dead data regardless of bit width.
4. **Breakthrough Architectural Solution for Frame Revocation (F-03):** The primary audit suggested adding `mapped_vaddr` to `CNodeSlot.payload_b` to make unmapping $O(1)$. This review challenged whether that assumes an invalid 1:1 mapping model. We prove that by enforcing a strict **1-capability-slot-to-at-most-1-virtual-mapping invariant**, storing `mapped_vaddr` in `payload_b` is completely sound, matches seL4 Mapping Database (MDB) semantics, and requires zero dynamic heap allocations.
5. **Breakthrough In-Situ Iterative Revocation Algorithm (F-01):** The primary audit suggested allocating a worklist or 256-byte bitmask in `.bss` to prevent recursive stack overflow. This review designs an **in-situ iterative tree pruning algorithm** that operates directly on the existing `parent`, `first_child`, and `next_sibling` links, requiring **zero bytes of heap, zero bytes of static worklist, and strictly $O(1)$ stack space**.

---

## 3. Finding-by-Finding Validation (F-01 through F-11)

### F-01: Recursive Revocation Stack Overflow
- **Cited Location:** `MILESTONE_3_DESIGN.md:252`
- **Source Inspection:** `kernel/src/main.rs:466` and `kernel/src/task.rs:60` allocate exactly 1 physical frame (4,096 bytes) for each task's kernel stack (`stack_a_paddr`, `stack_b_paddr`).
- **Failure Scenario:** A user task mints a derivation chain $C_0 \to C_1 \to \dots \to C_{40}$. A call to `SYS_CAP_REVOKE(C_0)` recurses down the CDT. Under SysV x86_64 calling conventions in unoptimized/debug Rust, each frame consumes 48–64 bytes. At depth $\ge 35$, the stack overflows its 4,096-byte boundary, corrupting adjacent direct-map kernel memory or triggering a CPU Double/Triple Fault.
- **Classification:** **CONFIRMED**
- **Severity:** **CRITICAL**
- **Adversarial Refinement:** The primary audit's recommendation of an external worklist or bitmask is unnecessary. The in-situ leaf-pruning algorithm designed in Section 6 accomplishes iterative revocation with zero auxiliary memory.

---

### F-02: Self-Minting and Cyclic Derivation Relationships
- **Cited Location:** `MILESTONE_3_DESIGN.md:281`, `kernel/src/capability.rs:43`
- **Source Inspection:** `SYS_CAP_MINT` in `MILESTONE_3_DESIGN.md` takes `src_slot` and `dest_slot`. It lacks any precondition asserting `src_slot != dest_slot` or `dest_slot.is_empty()`.
- **Failure Scenario:** An adversarial task invokes `SYS_CAP_MINT(src=5, dest=5, rights, badge)`. The kernel links slot 5 as its own parent and child: `CDT[5].first_child = Some(5)`. Because syscall entry clears the Interrupt Flag (`SFMask::write(RFlags::INTERRUPT_FLAG)` in `syscall.rs:35`), subsequent traversal or revocation enters an infinite loop in Ring 0 with interrupts disabled, permanently deadlocking the CPU core.
- **Classification:** **CONFIRMED**
- **Severity:** **CRITICAL**
- **Correction:** Preflight check:
  ```rust
  if src_slot == dest_slot || dest.cap_type != CapType::Empty {
      return Err(CapError::InvalidSlot);
  }
  ```

---

### F-03: Missing Virtual Address in `CNodeSlot` for Frame Revocation
- **Cited Location:** `MILESTONE_3_DESIGN.md:111-120`, `252`
- **Source Inspection:** `CNodeSlot` stores `payload_a = paddr` and `payload_b = 0`. An x86_64 4-level PML4 maps virtual addresses to physical pages. Finding which virtual address maps `paddr` requires walking all 512 entries across 4 levels (up to 2,000,000 PTEs).
- **Failure Scenario:** If the 2-million PTE scan is omitted during revocation, the physical frame is cleared from the capability slot while remaining actively mapped in the task's page table. The task retains unrestricted read/write hardware access to physical memory it no longer owns (Use-After-Revocation / Memory Aliasing).
- **Classification:** **CONFIRMED**
- **Severity:** **CRITICAL**
- **Adversarial Refinement:** Confirmed, but requires establishing the **1-capability-slot-to-at-most-1-virtual-mapping invariant** (detailed in Section 5).

---

### F-04: Stale CDT Links on Task Termination and Task-ID Reuse
- **Cited Location:** `kernel/src/task.rs:16`, `kernel/src/ipc.rs:139-144`
- **Source Inspection:** `kernel/src/task.rs:82` statically defines `TASKS: [Option<Task>; 16]`. Tasks are registered once at boot (`main.rs:474-512`). There is no dynamic task creation syscall (`SYS_SPAWN`) or task table recycling logic.
- **Failure Scenario:** If a task terminates, it remains in `TASKS` with `state: Terminated`. It cannot be recycled today. However, Stage 3.5 explicitly introduces driver crash recovery and service re-spawn (`MILESTONE_3_DESIGN.md:326`). If a driver is re-spawned with the same `task_id` without clearing its CNode and CDT nodes, the new instance inherits stale capability links.
- **Classification:** **CONFIRMED BUT OVERSTATED**
- **Severity:** **MEDIUM** (Overstated from High; dormant until Stage 3.5, but mandatory as a design precondition).
- **Correction:** Implement `destroy_task_cspace(task_id)` now in Stage 3.1.

---

### F-05: Parent Deletion with Live Descendants
- **Cited Location:** `MILESTONE_3_DESIGN.md:250`, `282`
- **Source Inspection:** `MILESTONE_3_DESIGN.md` defines `SYS_CAP_REVOKE` (which clears children), but omits deletion semantics for the parent slot itself.
- **Failure Scenario:** Slot $P$ has child $C$. If $P$ is deleted or overwritten while $C$ exists, $C.parent$ still points to index $P$. When slot $P$ is later allocated for an `MmioFrame`, child $C$ appears to be derived from the MMIO frame. Subsequent revocation on $P$ revokes $C$, or $C$ bypasses attenuation checks.
- **Classification:** **CONFIRMED**
- **Severity:** **HIGH**
- **Correction:** Mandatory cascading deletion: deleting a slot must automatically invoke `revoke_descendants(slot)` before clearing the parent slot and unlinking it from its parent and siblings.

---

### F-06: Missing Cross-Task Capability Delegation Mechanism
- **Cited Location:** `MILESTONE_3_DESIGN.md:281`, `300`, `kernel/src/ipc.rs:114`
- **Source Inspection:** `SYS_CAP_MINT` in Table 4.4 only accepts `(src_slot, dest_slot)` within the caller's CNode. `kernel/src/ipc.rs:16` only transmits a 64-bit integer (`msg: u64`).
- **Failure Scenario:** Section 4.5.1 mandates that client tasks grant `Frame` capabilities to the storage driver via IPC (`SYS_CALL(kernel_block_endpoint, ..., grant_frame_cptr)`). Because no cross-task minting or capability transfer mechanism exists, clients cannot grant buffers to drivers, breaking Stage 3.4 driver isolation.
- **Classification:** **CONFIRMED**
- **Severity:** **HIGH**
- **Adversarial Refinement:** The minimal solution uses existing unused syscall arguments (`arg3` in `SYS_SEND`, `arg2` in `SYS_RECV`) during IPC rendezvous, detailed in Section 9.

---

### F-07: 16-Bit Generation Counter Wraparound
- **Cited Location:** `MILESTONE_3_DESIGN.md:114`, `241`
- **Source Inspection:** `kernel/src/capability.rs:6` defines `pub struct CPtr(pub usize)`. Syscalls accept raw slot indices `0..127`.
- **Deeper Defect Discovered:** The primary audit focused on whether 16 bits is sufficient. Our adversarial review reveals that **generation counters are currently completely disconnected from the syscall ABI**. Because user space passes only `slot: u32`, the kernel never compares `slot.generation` to any caller-provided generation!
- **Classification:** **PARTIALLY CONFIRMED**
- **Severity:** **MEDIUM**
- **Correction:**
  1. Define handle format: `CPtr` must carry generation: `(generation << 16) | slot_index`.
  2. Widen `generation` to `u32` by utilizing the 4 bytes of compiler alignment padding at offset 4..8 in `CNodeSlot`.

---

### F-08: Missing Rights Validation in Existing IPC & I/O Ports
- **Cited Location:** `kernel/src/ipc.rs:125`, `226`, `409`, `453`
- **Source Inspection:** Directly verified. Existing syscall handlers match only on enum discriminants (`Capability::Endpoint`, `Capability::IOPort`).
- **Failure Scenario:** A task holding an `Endpoint` capability with only `READ` rights can call `SYS_SEND`. A task holding an `IOPort` capability with only `READ` rights can call `SYS_OUT_PORT`. Rights attenuation is completely ignored.
- **Classification:** **CONFIRMED**
- **Severity:** **MEDIUM**
- **Correction:** Insert bitmask assertions (`slot.rights.contains(...)`) in `ipc.rs`.

---

### F-09: Redundant & Inconsistent `cdt_index` in `CNodeSlot`
- **Cited Location:** `MILESTONE_3_DESIGN.md:118`, `248`
- **Source Inspection:** `CNodeSlot` contains `pub cdt_index: u32`. `CdtNode` contains `pub owner: (u8, u8)`.
- **Failure Scenario:** If kernel code reads `slot.cdt_index` from memory rather than calculating it, memory corruption or a forged slot could hijack an arbitrary CDT node.
- **Classification:** **CONFIRMED**
- **Severity:** **MEDIUM**
- **Correction:** The kernel must always compute CDT indices arithmetically: `(task_id << 7) | slot_index`.

---

### F-10: Task Termination on Querying Non-Frame/IOPort Slots
- **Cited Location:** `kernel/src/ipc.rs:359-371`
- **Source Inspection:** `sys_cap_info` matches only `Frame` and `IOPort`. For all other variants (including `Empty`), it logs `[KERNEL SEC-FAULT]` and sets `task.state = Terminated`.
- **Failure Scenario:** A user-space library inspecting its CSpace to find an empty slot is immediately killed by the kernel.
- **Classification:** **CONFIRMED**
- **Severity:** **LOW**
- **Correction:** Return error code `!0` instead of terminating the calling task.

---

### F-11: Conflation of `CPtr` as Slot Index and Virtual Address
- **Cited Location:** `kernel/src/capability.rs:6`, `kernel/src/task.rs:57`, `kernel/src/main.rs:477`
- **Source Inspection:** `Task.root_cnode` is typed as `CPtr`, but stores `(cnode_paddr + PHYS_MEMORY_OFFSET) as usize`. Syscall slot arguments are also typed as `CPtr`.
- **Classification:** **CONFIRMED**
- **Severity:** **INFORMATIONAL**
- **Correction:** Use `*mut CNode` or `PhysAddr` for `Task.root_cnode`.

---

## 4. Corrected Severity Ranking

| ID | Title | Original Severity | Corrected Severity | Classification |
| :--- | :--- | :--- | :--- | :--- |
| **F-01** | Recursive Revocation Stack Overflow | Critical | **CRITICAL** | Confirmed |
| **F-02** | Self-Minting / Cyclic CDT Relationships | Critical | **CRITICAL** | Confirmed |
| **F-03** | Missing Virtual Address in `CNodeSlot` | Critical | **CRITICAL** | Confirmed |
| **F-05** | Parent Deletion with Live Descendants | High | **HIGH** | Confirmed |
| **F-06** | Missing Cross-Task Capability Delegation | High | **HIGH** | Confirmed |
| **F-04** | Stale CDT Links on Task Termination | High | **MEDIUM** | Confirmed but Overstated |
| **F-07** | Generation Disconnect & Wraparound | Medium | **MEDIUM** | Partially Confirmed (Deeper defect identified) |
| **F-08** | Missing Rights Validation in Syscalls | Medium | **MEDIUM** | Confirmed |
| **F-09** | Redundant / Inconsistent `cdt_index` | Medium | **MEDIUM** | Confirmed |
| **F-10** | Task Termination on Probing Empty Slots | Low | **LOW** | Confirmed |
| **F-11** | Conflation of `CPtr` Type | Informational | **INFORMATIONAL** | Confirmed |

---

## 5. Correct Capability Ownership & Mapping Model

### The 1-Capability-to-1-Mapping Invariant

Can a single physical frame have multiple virtual mappings? **Yes.**  
Can a single `Frame` capability slot have multiple virtual mappings? **NO.**

In Janus:
1. **Physical Leaf Frame:** Managed by `PhysFrameAllocator`. It has no reference count.
2. **Virtual Mapping:** An active translation entry in a task's PML4/PDPT/PD/PT leaf entry.
3. **Capability Slot:** Represents an authorized handle to a physical frame.

```
+-------------------------------------------------------------+
| Task A CNode Slot 3: Frame { paddr: P, payload_b: 0x400000 } | --> Mapped at 0x400000 in Task A
+-------------------------------------------------------------+
                              |
                     SYS_CAP_MINT (Derive)
                              v
+-------------------------------------------------------------+
| Task B CNode Slot 4: Frame { paddr: P, payload_b: 0x600000 } | --> Mapped at 0x600000 in Task B
+-------------------------------------------------------------+
```

#### Invariant Rules:
1. **Single Mapping Per Slot:** Each `Frame` or `MmioFrame` capability slot may represent **at most one active virtual mapping**.
2. **Tracking Active Mapping:**
   - When unmapped: `payload_b == u64::MAX`.
   - When mapped: `payload_b == vaddr`.
3. **Mapping Operation (`SYS_MAP_FRAME`):**
   - If `slot.payload_b != u64::MAX`: Reject with `Err(CapError::AlreadyMapped)`.
   - Map `paddr` at `vaddr` in caller's PML4.
   - Set `slot.payload_b = vaddr`.
4. **Unmapping Operation (`SYS_UNMAP_FRAME`):**
   - If `slot.payload_b == u64::MAX`: Reject with `Err(CapError::NotMapped)`.
   - Unmap `slot.payload_b` from caller's PML4 and flush TLB.
   - Set `slot.payload_b = u64::MAX`.
5. **Revocation (`SYS_CAP_REVOKE`):**
   - For every revoked child slot $C$:
     - If $C$ is `Frame` or `MmioFrame` and $C\text{.payload\_b} \neq \text{u64::MAX}$:
       - Retrieve owner task: `task_id = cdt_index >> 7`.
       - Retrieve owner PML4: `TASKS[task_id].page_table`.
       - Call `unmap_user_page(page_table, C.payload_b)`.
       - Set `C.payload_b = u64::MAX`.

This design preserves exact $O(1)$ unmapping, eliminates the 2-million PTE scan, and supports shared physical frames across multiple tasks without dynamic allocation.

---

## 6. In-Situ Iterative Revocation Algorithm

### Algorithm Design (Zero Auxiliary Memory, $O(1)$ Stack Space)

Rather than using an external worklist or recursion, the algorithm repeatedly prunes the leftmost leaf of the derivation subtree rooted at `root_idx`:

```rust
pub unsafe fn revoke_descendants_iterative(root_task: u8, root_slot: u8) {
    let root_idx = cdt_index(root_task, root_slot);

    while let Some(child_idx) = CDT[root_idx].first_child {
        // 1. Descend down first_child links to find the deepest leaf
        let mut leaf = child_idx;
        while let Some(grandchild) = CDT[leaf as usize].first_child {
            leaf = grandchild;
        }

        // 2. Leaf has first_child == None. Retrieve its parent.
        let parent_idx = CDT[leaf as usize].parent.expect("Leaf must have parent");

        // 3. Unlink leaf from its parent's child list.
        // Since leaf was reached by descending first_child, leaf MUST be CDT[parent_idx].first_child.
        CDT[parent_idx as usize].first_child = CDT[leaf as usize].next_sibling;

        // 4. Reset CDT node for leaf
        CDT[leaf as usize] = CdtNode {
            parent: None,
            first_child: None,
            next_sibling: None,
        };

        // 5. Clean up the revoked slot in the target task's CNode
        let leaf_task = (leaf >> 7) as u8;
        let leaf_slot = (leaf & 0x7F) as u8;
        cleanup_revoked_slot(leaf_task, leaf_slot);
    }
}
```

### Complexity & Safety Analysis
- **Heap Memory:** Exactly 0 bytes.
- **Stack Memory:** Strictly $O(1)$ (two `u16` variables: `leaf`, `parent_idx`).
- **Time Complexity:** For $N$ descendant capabilities in a tree of depth $D$, each leaf unlinking takes at most $D$ descent steps. Total steps: $O(N \cdot D)$. With $N \le 2,048$ and $D \le 128$, worst-case iterations are $\le 262,144$ CPU instructions, executing in under $100\,\mu\text{s}$ on modern hardware.

---

## 7. Task Lifecycle & Identity Guarantees

1. **Bootstrapping:** Tasks 0, 1, and 15 are statically registered during boot.
2. **Termination:** When a task faults or calls exit:
   - `task.state` transitions to `TaskState::Terminated`.
   - The scheduler never schedules a `Terminated` task.
3. **CSpace Teardown Invariant (`destroy_task_cspace`):**
   Prior to any future task-slot recycling (e.g. Stage 3.5 driver restart), the kernel must execute:
   ```rust
   pub unsafe fn destroy_task_cspace(task_id: u8) {
       for slot in 0..CNODE_SLOTS as u8 {
           let idx = cdt_index(task_id, slot);
           // Revoke all child capabilities derived from this slot
           if CDT[idx].first_child.is_some() {
               revoke_descendants_iterative(task_id, slot);
           }
           // Unlink this slot from its parent's sibling chain
           unlink_from_parent(idx);
           // Clear slot and increment generation
           clear_slot(task_id, slot);
           CDT[idx] = CdtNode::empty();
       }
   }
   ```

---

## 8. Generation & Stale-Reference Policy

### Root Defect Resolution
In the existing codebase, capability handles passed across syscalls are raw indices (`slot: u32`). Generation counters in `CNodeSlot` are completely ignored.

### Required Invariant:
1. **Handle Format (`CPtr`):**
   ```rust
   #[repr(transparent)]
   #[derive(Debug, Clone, Copy, PartialEq, Eq)]
   pub struct CPtr(pub u64);

   impl CPtr {
       pub const fn new(generation: u32, slot: u16) -> Self {
           CPtr(((generation as u64) << 16) | (slot as u64))
       }
       pub const fn slot(self) -> usize {
           (self.0 & 0xFFFF) as usize
       }
       pub const fn generation(self) -> u32 {
           (self.0 >> 16) as u32
       }
   }
   ```
2. **32-Bit Generation Counter in `CNodeSlot`:**
   Utilize the 4 bytes of natural alignment padding at offset 4..8 in `CNodeSlot`:
   ```rust
   #[repr(C)]
   #[derive(Debug, Clone, Copy, PartialEq, Eq)]
   pub struct CNodeSlot {
       pub cap_type: CapType,         // offset 0, size 1
       pub rights: CapRights,         // offset 1, size 1
       pub _padding: u16,             // offset 2, size 2
       pub generation: u32,           // offset 4, size 4 (32-bit generation!)
       pub payload_a: u64,            // offset 8, size 8
       pub payload_b: u64,            // offset 16, size 8
       pub cdt_index: u32,            // offset 24, size 4
       pub _reserved: [u8; 4],        // offset 28, size 4
   }
   const _: () = assert!(core::mem::size_of::<CNodeSlot>() == 32);
   ```
3. **Syscall Verification Invariant:**
   `lookup_slot(task, cptr)` must assert:
   ```rust
   if cptr.generation() != 0 && slot.generation != cptr.generation() {
       return Err(CapError::StaleGeneration);
   }
   ```
   (Generation 0 is accepted as a wildcard for raw slot testing in Stage 3.1 unit tests).

---

## 9. Cross-Task Capability Delegation Semantics

### Minimal Microkernel IPC Delegation Protocol

To allow client tasks to pass `Frame` capabilities to drivers without expanding the IPC message buffer or adding dynamic heap allocations:

1. **Syscall ABI Utilization:**
   Syscall entry (`syscall.rs:116`) provides 5 arguments: `(sys_num, arg1, arg2, arg3, arg4, arg5)`.
   - `SYS_SEND`: `arg1 = endpoint_cptr`, `arg2 = msg`, `arg3 = cap_to_grant` (0 = none).
   - `SYS_RECV`: `arg1 = endpoint_cptr`, `arg2 = dest_cap_slot` (0 = do not receive cap).
2. **Rendezvous Delegation Logic:**
   When sender and receiver meet in `ipc.rs`:
   ```rust
   if sender_grant_slot != 0 && receiver_dest_slot != 0 {
       // Validate sender capability has CapRights::GRANT
       // Validate receiver destination slot is Empty
       // Mint child capability directly into receiver's CNode
       // Set CDT[dest_idx].parent = Some(src_idx)
   }
   ```
3. **Stage 3.1 Scope Boundary:**
   Stage 3.1 implements and tests `mint_cap_cross_task(src_task, src_slot, dest_task, dest_slot, rights, badge)`. The IPC register wiring is exposed in **Stage 3.3**.

---

## 10. Exact Rights Enforcement Requirements

During Stage 3.1, every syscall handler that consumes a capability must enforce the following bitmask checks:

| Syscall | Operation | Required `CapType` | Required `CapRights` Bit |
| :--- | :--- | :--- | :--- |
| `SYS_SEND` | IPC Message Send | `Endpoint` | `CapRights::WRITE` |
| `SYS_RECV` | IPC Message Receive | `Endpoint` / `Irq` | `CapRights::READ` |
| `SYS_IN_PORT` | Port I/O Read | `IOPort` | `CapRights::READ` |
| `SYS_OUT_PORT` | Port I/O Write | `IOPort` | `CapRights::WRITE` |
| `SYS_CAP_MINT` | Derive Capability | Any non-empty | `CapRights::GRANT` |
| `SYS_CAP_COPY` | Duplicate Capability | Any non-empty | `CapRights::GRANT` |
| `SYS_CAP_REVOKE` | Revoke Descendants | Any non-empty | `CapRights::WRITE` (or owner) |
| `SYS_MAP_FRAME` | Map Virtual Page (Stage 3.3) | `Frame` / `MmioFrame` | `CapRights::MAP` |
| `SYS_IRQ_ACK` | Ack IRQ Line (Stage 3.2) | `Irq` | `CapRights::WRITE` |

---

## 11. Revised Stage 3.1 Implementation Scope

### Strictly in Scope for Stage 3.1:
1. `kernel/src/capability.rs`:
   - `CapType` (9 variants: Empty, Untyped, Endpoint, Notification, CNode, Frame, MmioFrame, IOPort, Irq).
   - `CapRights` bitflags (`READ`, `WRITE`, `EXECUTE`, `GRANT`, `MAP`, `CALL`).
   - `CNodeSlot` (32 bytes, 8-byte aligned, 32-bit generation counter, compile-time assertions).
   - `CNode` (4 KiB page, 128 slots, compile-time assertions).
   - `CdtNode` (12 bytes) and static `CDT: [CdtNode; 2048]`.
   - Primitives: `mint_cap`, `copy_cap`, `revoke_descendants_iterative`, `delete_cap`, `lookup_slot`, `destroy_task_cspace`.
2. `kernel/src/syscall.rs` & `kernel/src/ipc.rs`:
   - Dispatch for `SYS_CAP_MINT` (13), `SYS_CAP_REVOKE` (14), `SYS_CAP_COPY` (20), `SYS_CAP_DELETE` (21).
   - Update `sys_cap_info` (5) to return `(type | rights | generation)` without killing the task.
   - Insert rights checks into `sys_send`, `sys_recv`, `sys_in_port`, `sys_out_port`.
3. Unit Test Matrix:
   - 19 automated host unit tests in `kernel/src/capability.rs`.

### Strictly Deferred to Later Stages:
- `SYS_MAP_FRAME` and `SYS_UNMAP_FRAME` $\to$ **Stage 3.3**.
- Hardware interrupt PIC masking and `Notification` bitmasks $\to$ **Stage 3.2**.
- Embedded ELF Loader $\to$ **Stage 3.4**.
- VirtIO DMA bounce proxy $\to$ **Stage 3.4**.
- Driver fault recovery $\to$ **Stage 3.5**.

---

## 12. Concrete Implementation Acceptance Criteria

Stage 3.1 is approved for merge only when:

1. **Size and Alignment Assertions:**
   - `assert!(size_of::<CNodeSlot>() == 32)`
   - `assert!(align_of::<CNodeSlot>() == 8)`
   - `assert!(size_of::<CNode>() == 4096)`
   - `assert!(align_of::<CNode>() == 4096)`
2. **Zero Dynamic Allocation:** CDT uses static 24 KiB BSS array; zero heap calls.
3. **Iterative Revocation Verified:** Iterative revocation passes deep-tree unit test ($\text{depth} \ge 16$) with $O(1)$ stack usage.
4. **Rights Attenuation Enforced:** Escalation unit tests pass; child rights cannot exceed parent rights.
5. **Cycle Prevention Asserted:** Self-minting returns error code immediately.
6. **No Regressions:** All 43 existing paging unit tests pass cleanly.
7. **CI Green:** Host unit tests and code formatting checks pass in GitHub Actions.

---

## 13. Unresolved Questions

1. **Cross-Task CNode Authority:** Does Janus require hierarchical CSpace delegation (tasks owning `CNode` capabilities pointing to other tasks), or is bilateral IPC rendezvous transfer sufficient?  
   *Review Finding:* For Milestone 3's isolated driver model, bilateral IPC rendezvous transfer is completely sufficient and avoids complex multi-level CSpace path walks.
2. **Notification Event ID Allocation:** Notification objects will be addressed in Stage 3.2; `CapType::Notification` is provisionally included in the enum in Stage 3.1 to stabilize `CNodeSlot` layout.
