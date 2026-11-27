# Stage 3.1 Capability Engine: Independent Architecture Audit

**Repository:** `medhansh5/Janus`  
**Baseline Commit:** `b58d23aebbee2a5ce625cb5b612b42274d5449c4` (merge commit of PR #4)  
**Parent Lineage:** `d288cad` (Parent 1, `main` baseline) + `ccff9eb` (Parent 2, `feat/stage-3-dynamic-user-vmm`)  
**Audit Target:** Stage 3.1 — Dynamic Capability Engine & Rights Attenuation  
**Mode:** STRICTLY READ-ONLY AUDIT — ZERO SOURCE CODE MODIFICATIONS  
**Audit Date:** 2026-11-24  

---

## 1. Executive Verdict

### Verdict: APPROVED WITH CONDITIONS

The architectural vision for Stage 3.1—introducing a rigorous object-capability model with fine-grained rights bitmasks, badged endpoints, 32-byte compile-time bounded CNode slots, a 2,048-entry deterministic Capability Derivation Tree (CDT), and cascading revocation—is **architecturally sound, mathematically elegant, and essential for Janus's microkernel security**.

However, this independent audit has identified **five high-severity architectural defects and omissions** in the design specification (`docs/architecture/MILESTONE_3_DESIGN.md`) that would cause Ring 0 stack exhaustion, deadlocks, silent memory aliasing, or cross-task security hijacking if implemented as currently documented:

1. **[CRITICAL] Unbounded Kernel Stack Recursion in Revocation:** The proposed recursive revocation algorithm (`SYS_CAP_REVOKE`) risks overflowing the 4 KiB per-task kernel stack at derivation depths $\ge 30$, causing a triple fault or kernel memory corruption. Revocation must be made strictly iterative ($O(1)$ stack space).
2. **[CRITICAL] Self-Minting / Cyclic Derivation Vulnerability:** The minting specification omits a check for `src_slot == dest_slot` and `dest_slot.is_empty()`. An adversarial user task could mint a capability into its own slot, creating a circular CDT loop that freezes Ring 0 in an infinite loop during revocation.
3. **[CRITICAL] Missing Virtual Address in `CNodeSlot` for Frame Revocation:** When a `Frame` or `MmioFrame` capability is revoked, the kernel must unmap the frame from the owning task's page table. However, `CNodeSlot` stores physical `paddr` but omits the mapped virtual address `vaddr`. Finding which page to unmap would require an intractable 2-million-entry PML4 tree traversal. `CNodeSlot`'s currently unused `payload_b` must store `mapped_vaddr`.
4. **[HIGH] Lack of CSpace Cleanup on Task Termination:** Current kernel code marks tasks as `Terminated` but does not destroy their CNodes or CDT nodes. When a `task_id` is reused, the new task inherits stale CDT parent/child links from foreign tasks, allowing cross-task capability theft or destruction.
5. **[HIGH] Unspecified Cross-Task Capability Transfer:** Milestone 3 relies on passing `Frame` capabilities to drivers via IPC (`SYS_CALL(..., grant_frame_cptr)`), but `kernel/src/ipc.rs` only passes a 64-bit integer, and `SYS_CAP_MINT` operates solely within the caller's CNode. The cross-task capability transfer mechanism is completely missing from the specification.

**Condition for Implementation Authorization:**  
Stage 3.1 implementation **must incorporate the five mandatory architectural corrections** detailed in Section 8 before any code is committed.

---

## 2. Verified Repository Baseline

Direct inspection of the repository working tree and GitHub remote confirms the exact baseline:

- **Repository Identity:** `medhansh5/Janus`
- **Current Branch:** `main`
- **HEAD Commit:** `b58d23aebbee2a5ce625cb5b612b42274d5449c4`
- **Upstream Tracking:** `origin/main` (synced, up to date)
- **PR #4 State:** `CLOSED` / `MERGED` at `2026-11-24T14:46:05Z` into `main`
- **Merge Strategy:** Standard merge commit (`d288cadd17aee34ddac16f95de89ddb16b329f92` + `ccff9eb2ee429f3ebc996f3abc5a708f3ba63408`)
- **CI Status on `main` (`b58d23a`):**
  - Workflow Run: `45417782710`
  - *Target Builds & Host Unit Tests:* `success`
  - *Code Formatting & Static Analysis:* `success`
- **Working Tree Integrity:** Clean. All pre-existing architecture documents under `docs/architecture/` (`MILESTONE_3_DESIGN.md`, `MILESTONE_3_REVIEW.md`, `MILESTONE_3_REVISION_NOTES.md`, `MILESTONE_3_SECOND_REVIEW.md`) are untracked, untouched, and fully preserved.

---

## 3. Existing Implementation Inventory

Direct audit of the Rust kernel codebase (`kernel/src/`) reveals the exact current state of capabilities, IPC, paging, and tasks:

### 3.1 Capability Subsystem (`kernel/src/capability.rs`)
- **Lines of Code:** 58 lines.
- **`CPtr` Definition (`line 6`):** `pub struct CPtr(pub usize);` — defined as an opaque index into a CNode.
- **`Capability` Enum (`lines 19-35`):**
  ```rust
  pub enum Capability {
      Empty,
      Untyped { paddr: u64, size: u64 },
      Endpoint { target_task_id: u64 },
      CNode { paddr: u64, size: u64 },
      Frame { paddr: u64 },
      IOPort { base: u16, size: u16 },
      InterruptEndpoint { irq: u8 },
  }
  ```
  *Audit Observation:* Size of `Capability` is 24 bytes (1 byte discriminant + 7 bytes padding + 16 bytes payload).
- **`CNode` Struct (`lines 41-44`):**
  ```rust
  #[repr(C, align(4096))]
  pub struct CNode {
      pub slots: [Capability; CNODE_SLOTS],
  }
  ```
  `CNODE_SLOTS = 128`. Size is $128 \times 24 = 3,072$ bytes.
- **Current Deficiencies:**
  - Zero rights bitmasks (`CapRights` does not exist).
  - Zero generation counters (`generation` does not exist).
  - Zero badged endpoints (`Endpoint` only stores raw `target_task_id: u64`).
  - Zero derivation tracking (no parent, child, or sibling links; no CDT).
  - Zero dynamic capability operations (no mint, copy, revoke, or delete).
  - Missing capability types: `Notification` and `MmioFrame` do not exist.

### 3.2 Task Subsystem (`kernel/src/task.rs`)
- **Task Table (`lines 79-84`):** Fixed static array `static mut TASKS: [Option<Task>; 16]`.
- **`Task` Representation (`lines 54-62`):**
  - Stores `root_cnode: CPtr`.
  - *Audit Finding:* In `main.rs:477`, `root_cnode` is initialized to `(cnode_paddr + PHYS_MEMORY_OFFSET) as usize`. `CPtr` is improperly conflated as both a virtual direct-map pointer and a slot index.
- **Lifecycle & Teardown:**
  - Tasks can transition to `TaskState::Terminated` (`line 16`).
  - When a task terminates, **no resource cleanup occurs**. The page table is not freed, the CNode is not freed, and slots are not cleared.

### 3.3 IPC & Capability Verification (`kernel/src/ipc.rs`)
- **Capability Lookup (`lines 98-111`):**
  `lookup_cap(root_cnode: CPtr, slot: CPtr) -> Result<Capability, CapLookupError>`.
  Checks bounds (`slot.0 < 128`), checks direct-map address range and 4 KiB alignment of `root_cnode`, and returns `cnode.slots[slot.0]`.
- **Authorization Enforcement:**
  - `sys_send` (`line 125`): Checks that slot holds `Capability::Endpoint { target_task_id }`. Does NOT check write/send rights.
  - `sys_recv` (`line 231`): Checks that slot holds `Capability::Endpoint` or `InterruptEndpoint`. Does NOT check read/recv rights.
  - `sys_in_port` (`line 409`) & `sys_out_port` (`line 453`): Checks `Capability::IOPort { base, size }` and `offset < size`. Does NOT check read or write rights.
  - `sys_cap_info` (`lines 355-372`): Allows querying `Frame` and `IOPort`. For all other variants (including `Empty`), **it logs a security fault and terminates the task**.

### 3.4 Paging Subsystem (`kernel/src/paging.rs`)
- Fully verified in Stage 3.0. Exposes `create_user_pml4`, `map_user_page`, `unmap_user_page`, and `destroy_user_pml4`.
- Enforces virtual W^X on leaf page table entries.
- Page table frames are tracked and zeroed.
- Leaf frames are caller-owned; `unmap_user_page` and `destroy_user_pml4` do NOT free leaf frames.
- *Audit Observation:* `paging.rs` is completely agnostic of capabilities; capability authority must sit above paging.

---

## 4. Exact Capability & CDT Data Model

### 4.1 CNodeSlot Memory Layout Audit

To satisfy the invariant $\text{Size}(\text{CNode}) \le 4096$ with $128$ slots, each slot has an exact budget of:
$$\text{Slot Budget} = \frac{4096}{128} = 32 \text{ bytes}$$

#### Struct Definition & Offset Map
```rust
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CNodeSlot {
    pub cap_type: CapType,         // offset  0, size 1, align 1
    pub rights: CapRights,         // offset  1, size 1, align 1
    pub generation: u16,           // offset  2, size 2, align 2
    // offset 4..8: 4 bytes compiler alignment padding
    pub payload_a: u64,            // offset  8, size 8, align 8
    pub payload_b: u64,            // offset 16, size 8, align 8
    pub cdt_index: u32,            // offset 24, size 4, align 4
    pub _reserved: [u8; 4],        // offset 28, size 4, align 1
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

#### Verification of Alignment Bug Resolution
The second architecture review (`MILESTONE_3_SECOND_REVIEW.md:246-272`) discovered that an earlier specification used `_reserved: [u8; 8]`, which when combined with compiler padding between `generation` and `payload_a` produced a 40-byte struct ($128 \times 40 = 5,120 \text{ bytes}$).  
With `_reserved: [u8; 4]`, the struct size is **strictly 32 bytes**, and the `assert!(size_of::<CNode>() == 4096)` static assertion is guaranteed to compile cleanly.

### 4.2 Payload Variant Mapping

`payload_a` and `payload_b` (16 bytes total) store variant-specific state across all capability types:

| `CapType` (u8) | `payload_a` (u64) | `payload_b` (u64) | Architectural Purpose & Invariants |
| :--- | :--- | :--- | :--- |
| `Empty (0)` | `0` | `0` | Inactive slot. `rights = 0`. |
| `Untyped (1)` | `paddr` (frame-aligned) | `size` (bytes) | Raw physical RAM region for retyping. |
| `Endpoint (2)` | `target_task_id: u32` | `badge: u64` | Synchronous IPC rendezvous. 0 = unbadged. |
| `Notification (3)` | `object_id: u32` | `0` (or `badge: u64`) | Asynchronous 64-bit event bitmask handle. |
| `CNode (4)` | `paddr` (4 KiB aligned) | `slot_count: u16` | Authority over a CSpace table. |
| `Frame (5)` | `paddr` (4 KiB aligned) | `mapped_vaddr: u64` | Cacheable RAM page. `payload_b` holds virtual mapping address (or `!0` if unmapped). |
| `MmioFrame (6)` | `paddr` (4 KiB aligned) | `mapped_vaddr: u64` | Uncacheable hardware MMIO page. `EXECUTE` right strictly forbidden. |
| `IOPort (7)` | `(base: u16) \| ((count: u16) << 16)` | `0` | I/O port range. Checked against exclusion table. |
| `Irq (8)` | `irq: u8` | `0` | Hardware interrupt line authority. |

> [!IMPORTANT]
> **Payload B for Frame / MmioFrame:** `payload_b` is explicitly assigned to store `mapped_vaddr: u64` (initialized to `u64::MAX` when unmapped). This resolves Finding 3 by allowing $O(1)$ page unmapping on revocation without traversing the page table tree.

### 4.3 Deterministic Capability Derivation Tree (CDT)

To prevent dynamic heap allocation, fragmentation, and memory exhaustion in Ring 0, the CDT is a statically sized array:

$$\text{Capacity} = \text{MAX\_TASKS} \times \text{CNODE\_SLOTS} = 16 \times 128 = 2,048 \text{ slots}$$

#### Node Structure
```rust
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CdtNode {
    pub parent: Option<u16>,       // 4 bytes: CDT index of parent
    pub first_child: Option<u16>,  // 4 bytes: CDT index of first child
    pub next_sibling: Option<u16>, // 4 bytes: CDT index of next sibling
}

const _: () = assert!(core::mem::size_of::<CdtNode>() == 12);

pub static mut CDT: [CdtNode; 2048] = [CdtNode {
    parent: None,
    first_child: None,
    next_sibling: None,
}; 2048];
```
- Total static CDT memory footprint: $2,048 \times 12 \text{ bytes} = 24 \text{ KiB}$ (6 frames in `.bss`).

#### Deterministic Indexing Invariant
The CDT index is **strictly derived from the task ID and slot index**:
$$\text{cdt\_index} = (\text{task\_id} \times 128) + \text{slot\_index} = (\text{task\_id} \ll 7) \mid \text{slot\_index}$$

$$\text{task\_id} = \lfloor \text{cdt\_index} / 128 \rfloor = \text{cdt\_index} \gg 7$$
$$\text{slot\_index} = \text{cdt\_index} \pmod{128} = \text{cdt\_index} \& 0x7F$$

- Every task/slot tuple has a unique, bijective mapping to exactly one CDT node.
- Memory allocation cannot fail during derivation.
- CDT table exhaustion is impossible.

---

## 5. Operation Semantics & Invariants

### 5.1 Capability Minting (`SYS_CAP_MINT`)
- **Syscall Arguments:** `arg1 = src_slot`, `arg2 = dest_slot`, `arg3 = rights_mask`, `arg4 = badge`.
- **Preconditions & Authorizations:**
  1. `src_slot < 128` and `dest_slot < 128`.
  2. `src_slot != dest_slot` (prevents self-referential cycle creation).
  3. `src_slot` must be non-empty (`cap_type != CapType::Empty`).
  4. `src_slot` must possess `CapRights::GRANT`.
  5. `dest_slot` must be strictly `Empty` (`cap_type == CapType::Empty`).
  6. Rights attenuation: Requested rights must be a subset of parent rights:
     $$\text{rights\_mask} \subseteq \text{src.rights} \iff (\text{rights\_mask} \ \& \ \neg\text{src.rights}) == 0$$
  7. Badge immutability:
     - If `src` is an `Endpoint`: if `src.badge != 0`, the child **cannot** alter or overwrite the badge (`new_badge == src.badge`). A new badge can only be minted if `src.badge == 0`.
     - For non-endpoint capabilities, `badge` must be `0`.
  8. For `MmioFrame`: `CapRights::EXECUTE` must be strictly rejected.
  9. For `Frame`: `CapRights::WRITE` and `CapRights::EXECUTE` cannot both be present.
- **State Mutations (Atomic):**
  1. Compute `src_idx = (caller_id << 7) | src_slot` and `dest_idx = (caller_id << 7) | dest_slot`.
  2. Populate `dest_slot`: copy type and payloads from `src_slot`, set attenuated rights, set badge.
  3. Increment `dest_slot.generation`: wraps from `0xFFFF` to `1` (skipping `0`).
  4. Link into CDT:
     - `CDT[dest_idx].parent = Some(src_idx)`
     - `CDT[dest_idx].next_sibling = CDT[src_idx].first_child`
     - `CDT[src_idx].first_child = Some(dest_idx)`
- **Rollback on Failure:** Return error code (`!0` or `ENOSYS`/`EINVAL`); zero mutations occur.

### 5.2 Capability Copying (`SYS_CAP_COPY`)
- Identical to `SYS_CAP_MINT`, but with:
  - $\text{rights} = \text{src.rights}$
  - $\text{badge} = \text{src.badge}$
- Creates a new child in the CDT derived from `src_slot`.

### 5.3 Recursive Revocation (`SYS_CAP_REVOKE`)
- **Syscall Arguments:** `arg1 = slot`.
- **Preconditions:**
  1. `slot < 128`.
  2. Caller must hold a non-empty capability in `slot`.
  3. Caller must have `CapRights::WRITE` or be the owner of the capability.
- **Semantics:**
  - Revokes **all descendants** derived from `slot`. The capability at `slot` itself **remains intact**.
- **Iterative Traversal (Non-Recursive):**
  To prevent kernel stack overflow, revocation must not use function recursion.
  1. Identify root CDT index: `root_idx = (caller_id << 7) | slot`.
  2. While `CDT[root_idx].first_child` is `Some(child_idx)`:
     - Find a leaf in the sub-tree rooted at `child_idx`: traverse down `first_child` until a node $L$ with `first_child == None` is reached.
     - Unlink $L$ from its parent and sibling chain.
     - Translate $L$ to `(target_task_id, target_slot)`.
     - If capability at $L$ is a mapped `Frame` or `MmioFrame` (`payload_b != u64::MAX`):
       - Look up `target_task.page_table`.
       - Invoke `unmap_user_page(target_task.page_table, payload_b)`.
     - Clear the slot at $L$: `cap_type = Empty`, `rights = 0`, `payload_a = 0`, `payload_b = 0`.
     - Increment `generation` at $L$ (skipping 0).
     - Reset `CDT[L] = CdtNode::empty()`.

### 5.4 Capability Deletion (Kernel Primitive & `SYS_CAP_DELETE`)
- **Semantics:** Removes the capability in `slot` itself.
- **Invariants:**
  1. If `slot` has children (`CDT[idx].first_child.is_some()`):
     - **Mandatory Cascading Revocation:** The kernel must invoke `revoke_descendants(idx)` *before* deleting the slot itself. A slot cannot be deleted while child references exist.
  2. Unlink `idx` from its parent's child list:
     - If `CDT[parent].first_child == Some(idx)`: `CDT[parent].first_child = CDT[idx].next_sibling`.
     - Else: traverse sibling chain from `CDT[parent].first_child` to find `prev` where `CDT[prev].next_sibling == Some(idx)`, and set `CDT[prev].next_sibling = CDT[idx].next_sibling`.
  3. Clear `slot`: reset to `Empty`, clear payloads, increment `generation`.
  4. Reset `CDT[idx]` to empty.

---

## 6. Security Threat Analysis & Vulnerability Findings

### Finding 1: Unbounded Kernel Stack Recursion in Revocation
- **Severity:** **CRITICAL**
- **Location:** `docs/architecture/MILESTONE_3_DESIGN.md:252`
- **Vulnerability Scenario:** The architecture specifies: *"Traverses first_child -> next_sibling recursively. For every child capability... clears the slot and unlinks the CDT node."*  
  Each task in Janus has a single 4 KiB kernel stack (`kernel/src/main.rs:466`, `kernel/src/task.rs:60`). A user task can mint capabilities in a chain: $C_0 \to C_1 \to C_2 \dots \to C_{40}$. When `SYS_CAP_REVOKE(C_0)` is executed, the recursive call frame exhausts the 4,096-byte stack, overwriting adjacent memory or causing a CPU Double/Triple Fault.
- **Required Invariant:** All capability operations in Ring 0 must run in strictly bounded $O(1)$ stack space.
- **Minimal Correction:** Mandate iterative post-order unlinking or an explicit fixed worklist array in `.bss` (max depth or 256-byte visited bitmask).

### Finding 2: Self-Minting / Cyclic Derivation Vulnerability
- **Severity:** **CRITICAL**
- **Location:** `docs/architecture/MILESTONE_3_DESIGN.md:281`
- **Vulnerability Scenario:** `SYS_CAP_MINT` takes `src_slot` and `dest_slot`. The design does not require `src_slot != dest_slot` or check that `dest_slot` is empty. If a caller passes `src_slot = 5, dest_slot = 5`, the kernel links slot 5 as its own parent/child: `CDT[5].first_child = Some(5)`. Any subsequent traversal or revocation enters an infinite loop in Ring 0 with interrupts disabled, permanently locking the CPU core.
- **Required Invariant:** $\text{src\_slot} \neq \text{dest\_slot}$, and $\text{dest\_slot.cap\_type} == \text{CapType::Empty}$.
- **Minimal Correction:** Enforce `if src_slot == dest_slot || dest.cap_type != CapType::Empty { return !0; }` before any CDT manipulation.

### Finding 3: Missing Virtual Address in `CNodeSlot` Breaks Frame Revocation
- **Severity:** **CRITICAL**
- **Location:** `docs/architecture/MILESTONE_3_DESIGN.md:111-120`, `docs/architecture/MILESTONE_3_DESIGN.md:252`
- **Vulnerability Scenario:** The design specifies that revoking a `Frame` or `MmioFrame` capability must immediately unmap the page from the owning task's virtual address space. However, `CNodeSlot` only stores `payload_a = paddr`. It does not store the mapped virtual address `vaddr`. Because an x86_64 PML4 maps virtual addresses to physical frames (not the reverse), finding which virtual page maps `paddr` requires walking all 512 entries across 4 table levels (up to 2,000,000 PTEs) during revocation. If this scan is omitted, the page remains mapped after revocation, creating a critical Use-After-Revocation / Physical Memory Aliasing vulnerability.
- **Required Invariant:** Unmapping a revoked frame must be an $O(1)$ operation.
- **Minimal Correction:** Utilize `payload_b: u64` in `CNodeSlot` to record `mapped_vaddr` when `map_user_page` succeeds. Revocation then directly invokes `unmap_user_page(pml4, slot.payload_b)`.

### Finding 4: Stale CDT Links on Task Termination
- **Severity:** **HIGH**
- **Location:** `kernel/src/task.rs:16`, `kernel/src/ipc.rs:139-144`
- **Vulnerability Scenario:** In `kernel/src/task.rs`, a faulted task transitions to `TaskState::Terminated`, but its CNode and CDT nodes remain active in memory. If a new task is spawned with the recycled `task_id`, it inherits the CDT nodes of the terminated task. If other tasks hold capabilities derived from the terminated task, they remain linked to the recycled task. A revocation initiated by an external task will delete capabilities in the new, unrelated task.
- **Required Invariant:** Task destruction must synchronously detach and invalidate all CSpace and CDT entries for that task.
- **Minimal Correction:** Introduce `task::destroy_task_cspace(task_id)` which revokes all child capabilities derived from `task_id`, unlinks all capabilities held by `task_id` from their parents, and zeros all 128 CDT nodes.

### Finding 5: Parent Deletion with Live Descendants
- **Severity:** **HIGH**
- **Location:** `docs/architecture/MILESTONE_3_DESIGN.md:250`, `docs/architecture/MILESTONE_3_DESIGN.md:282`
- **Vulnerability Scenario:** The architecture defines `SYS_CAP_REVOKE` (which clears children), but omits deletion semantics for the parent slot itself. If a parent slot is cleared or overwritten while children exist, the child nodes in the CDT still point to the parent's index. When the parent slot is subsequently re-allocated for a different capability (e.g. an `MmioFrame`), the orphaned children appear to be derived from the new capability.
- **Required Invariant:** A capability slot with active children cannot be deleted without first revoking all descendants.
- **Minimal Correction:** Require that deleting a slot automatically triggers recursive revocation of all descendants before unlinking and clearing the slot.

### Finding 6: Missing Cross-Task Capability Delegation Mechanism
- **Severity:** **HIGH**
- **Location:** `docs/architecture/MILESTONE_3_DESIGN.md:281`, `docs/architecture/MILESTONE_3_DESIGN.md:300`, `kernel/src/ipc.rs:114`
- **Vulnerability Scenario:** `MILESTONE_3_DESIGN.md` Section 4.5 specifies that clients grant `Frame` capabilities to drivers via IPC: `SYS_CALL(kernel_block_endpoint, ..., grant_frame_cptr)`. However, `SYS_CAP_MINT` is defined only for `(src_slot, dest_slot)` within the caller's CNode, and `kernel/src/ipc.rs` only transmits a 64-bit integer (`msg: u64`). There is no mechanism in the system to transfer a capability from one task to another.
- **Required Invariant:** A microkernel capability system must provide an unambiguous, authorized mechanism to delegate capabilities across task boundaries.
- **Minimal Correction:** Formally specify cross-task delegation: either via IPC capability transfer during rendezvous (with `GRANT` rights validation), or via `SYS_CAP_MINT` taking an authorized `dest_cnode_cap`.

### Finding 7: 16-Bit Generation Counter Wraparound (ABA Problem)
- **Severity:** **MEDIUM**
- **Location:** `docs/architecture/MILESTONE_3_DESIGN.md:114`, `docs/architecture/MILESTONE_3_DESIGN.md:241`
- **Vulnerability Scenario:** `generation` is a `u16` that increments on slot reuse and wraps from `0xFFFF` to `1`. In a tight loop, 65,535 allocations and revocations on a single slot wrap the generation back to `1`. If an asynchronous waiter or peer held a stale capability reference expecting generation 1, the reference becomes valid again for a completely different capability.
- **Required Invariant:** Stale capability references must never validate against reused slots.
- **Minimal Correction:** Document the 16-bit wraparound boundary. Note that offset 4..8 in `CNodeSlot` is currently compiler padding; widening `generation` to `u32` (or ensuring rendezvous checks task state and generation) eliminates practical wraparound.

### Finding 8: Missing Rights Validation in Existing IPC & I/O Ports
- **Severity:** **MEDIUM**
- **Location:** `kernel/src/ipc.rs:125`, `kernel/src/ipc.rs:226`, `kernel/src/ipc.rs:409`, `kernel/src/ipc.rs:453`
- **Vulnerability Scenario:** Current syscalls check only capability enum types. `sys_send` does not check `CapRights::WRITE`, `sys_recv` does not check `CapRights::READ`, and port I/O does not check `READ` or `WRITE`. Any task possessing an `Endpoint` or `IOPort` can perform all operations, bypassing attenuation.
- **Required Invariant:** Syscalls must enforce exact bitmask rights (`READ`, `WRITE`, `GRANT`).
- **Minimal Correction:** Update `handle_syscall` handlers in `ipc.rs` to assert required `CapRights` bits.

### Finding 9: Redundant & Inconsistent `cdt_index` in `CNodeSlot`
- **Severity:** **MEDIUM**
- **Location:** `docs/architecture/MILESTONE_3_DESIGN.md:118`, `docs/architecture/MILESTONE_3_DESIGN.md:248`
- **Vulnerability Scenario:** `CNodeSlot` contains `pub cdt_index: u32`, and `CdtNode` contains `pub owner: (u8, u8)`. Yet the architecture mandates that CDT indexing is purely deterministic: `(task_id << 7) | slot_index`. If kernel logic reads `slot.cdt_index` from memory rather than computing it, memory corruption or a forged slot could hijack another node's derivation tree.
- **Required Invariant:** The kernel must strictly compute CDT indices mathematically from `(task_id, slot_index)` and never trust cached in-memory index fields for authorization.
- **Minimal Correction:** Mandate that kernel capability logic always computes `cdt_index` arithmetically.

### Finding 10: Task Termination on Querying Non-Frame/IOPort Slots
- **Severity:** **LOW**
- **Location:** `kernel/src/ipc.rs:359-371`
- **Vulnerability Scenario:** `sys_cap_info` logs a security fault and terminates the task if the queried slot is not `Frame` or `IOPort` (e.g. `Empty` or `Endpoint`). This prevents user-space runtimes from safely inspecting their own capability slots.
- **Required Invariant:** Querying an invalid or empty slot should return an error code (`!0`), not terminate the calling task.
- **Minimal Correction:** Modify `sys_cap_info` to return metadata or error code `!0` without killing the task.

### Finding 11: Conflation of `CPtr` as Slot Index and Virtual Address
- **Severity:** **INFORMATIONAL**
- **Location:** `kernel/src/capability.rs:6`, `kernel/src/task.rs:57`, `kernel/src/ipc.rs:102`
- **Vulnerability Scenario:** `CPtr` is used both for slot indices (`CPtr(1)`) and for 64-bit kernel virtual addresses (`CPtr(PHYS_MEMORY_OFFSET + paddr)`).
- **Required Invariant:** Distinct types should represent distinct semantic concepts.
- **Minimal Correction:** Change `Task.root_cnode` to `PhysAddr` or `*mut CNode`.

---

## 7. Findings Summary Matrix

| ID | Title | Severity | Location | Invariant Violated |
| :--- | :--- | :--- | :--- | :--- |
| **F-01** | Unbounded Stack Recursion in Revocation | **CRITICAL** | `MILESTONE_3_DESIGN.md:252` | Kernel operations must run in $O(1)$ stack space. |
| **F-02** | Self-Minting / Cyclic CDT Derivation | **CRITICAL** | `MILESTONE_3_DESIGN.md:281` | CDT derivation must be strictly acyclic. |
| **F-03** | Missing Virtual Address in `CNodeSlot` | **CRITICAL** | `MILESTONE_3_DESIGN.md:111` | Revoked frames must be unmapped in $O(1)$ time. |
| **F-04** | Stale CDT Links on Task Termination | **HIGH** | `kernel/src/task.rs:16` | Terminated tasks must release all CSpace/CDT links. |
| **F-05** | Parent Deletion with Live Descendants | **HIGH** | `MILESTONE_3_DESIGN.md:250` | Slots with children cannot be deleted without auto-revoke. |
| **F-06** | Missing Cross-Task Capability Delegation | **HIGH** | `MILESTONE_3_DESIGN.md:281` | Microkernel must provide cross-task capability transfer. |
| **F-07** | 16-Bit Generation Wraparound | **MEDIUM** | `MILESTONE_3_DESIGN.md:114` | Stale references must not validate on reused slots. |
| **F-08** | Missing Rights Validation in Syscalls | **MEDIUM** | `kernel/src/ipc.rs:125` | All capability syscalls must verify `CapRights`. |
| **F-09** | Redundant & Inconsistent `cdt_index` | **MEDIUM** | `MILESTONE_3_DESIGN.md:118` | CDT indices must be computed mathematically. |
| **F-10** | Task Termination on Probing Empty Slots | **LOW** | `kernel/src/ipc.rs:359` | Slot introspection must return errors, not terminate. |
| **F-11** | Conflation of `CPtr` Type | **INFORMATIONAL** | `kernel/src/capability.rs:6` | Semantic distinction between indices and pointers. |

---

## 8. Required Architecture Corrections

Prior to starting Stage 3.1 implementation, the following concrete specifications are established:

1. **Iterative Revocation:** `revoke_descendants` must be implemented using an iterative leaf-unlinking algorithm or an explicit worklist in `.bss`, with a hard compile-time depth assertion ($\le 16$).
2. **Cycle Prevention:** `SYS_CAP_MINT` must assert:
   ```rust
   if src_slot == dest_slot || dest_slot >= CNODE_SLOTS || src_slot >= CNODE_SLOTS {
       return Err(CapError::InvalidSlot);
   }
   if dest_slot.cap_type != CapType::Empty {
       return Err(CapError::SlotOccupied);
   }
   ```
3. **Mapped Virtual Address Tracking:**
   `CNodeSlot.payload_b` for `CapType::Frame` and `CapType::MmioFrame` is defined as:
   - `payload_b == u64::MAX`: Frame is unmapped.
   - `payload_b < 0x0000_8000_0000_0000`: Frame is mapped at `vaddr = payload_b`.
4. **Cascading Deletion:** Deleting a slot automatically triggers recursive revocation of all child nodes prior to unlinking the slot.
5. **Deterministic Indexing Rule:** Kernel capability routines must compute CDT indices via:
   ```rust
   #[inline(always)]
   pub const fn cdt_index(task_id: u8, slot: u8) -> usize {
       ((task_id as usize) << 7) | (slot as usize)
   }
   ```
   Stored `slot.cdt_index` must never be used as an index into the global `CDT` array.

---

## 9. Proposed Stage 3.1 Implementation Scope

To prevent scope creep and maintain strict milestone boundaries:

### In Scope for Stage 3.1:
1. `kernel/src/capability.rs`:
   - `CapType` (all 9 variants).
   - `CapRights` bitflags (`READ`, `WRITE`, `EXECUTE`, `GRANT`, `MAP`, `CALL`).
   - `CNodeSlot` (exact 32-byte layout with static size/align assertions).
   - `CNode` (4 KiB page, 128 slots, with static size/align assertions).
   - `CdtNode` and statically allocated `CDT: [CdtNode; 2048]`.
   - Core kernel primitives: `mint_cap`, `copy_cap`, `revoke_cap`, `delete_cap`, `lookup_slot`.
2. `kernel/src/syscall.rs` & `kernel/src/ipc.rs`:
   - Expose `SYS_CAP_MINT` (13).
   - Expose `SYS_CAP_REVOKE` (14).
   - Expose `SYS_CAP_COPY` (20) and `SYS_CAP_DELETE` (21).
   - Update `sys_cap_info` (5) to return slot metadata safely without terminating.
   - Integrate `CapRights` checks into existing `sys_send`, `sys_recv`, `sys_in_port`, `sys_out_port`.
3. Unit Test Suite in `kernel/src/capability.rs`:
   - Comprehensive host-side unit test suite testing all layout, attenuation, derivation, revocation, and generation invariants.

### Strictly Out of Scope (Deferred to Later Stages):
- `SYS_MAP_FRAME` and `SYS_UNMAP_FRAME` $\to$ **Stage 3.3**.
- Asynchronous notifications, event bitmasks, PIC line masking in IDT, `SYS_NOTIFY_WAIT`, `SYS_NOTIFY_SIGNAL`, `SYS_IRQ_ACK` $\to$ **Stage 3.2**.
- Embedded ELF Loader $\to$ **Stage 3.4**.
- VirtIO DMA Bounce Proxy $\to$ **Stage 3.4**.
- Driver fault recovery $\to$ **Stage 3.5**.

---

## 10. Concrete Test Matrix

All capability engine logic can and must be validated through host unit tests (`cargo test -p kernel`) without requiring QEMU.

### 10.1 Host Unit Test Suite (Automated in CI)

| Test Identifier | Category | Scenario / Assertion Tested |
| :--- | :--- | :--- |
| `test_cnodeslot_layout_and_size` | Layout | `size_of::<CNodeSlot>() == 32`, `align_of == 8`, offsets `(0, 1, 2, 8, 16, 24, 28)`. |
| `test_cnode_layout_and_size` | Layout | `size_of::<CNode>() == 4096`, `align_of == 4096`, 128 slots. |
| `test_cdt_deterministic_indexing` | CDT | Bijective round-trip `(task, slot) -> idx -> (task, slot)` for all 2,048 entries. |
| `test_valid_mint_attenuation` | Minting | Parent with `READ|WRITE|GRANT` mints child with `READ|GRANT`. |
| `test_mint_rights_escalation_rejected`| Security | Parent with `READ` attempts to mint child with `WRITE`; rejected with `Err`. |
| `test_mint_without_grant_rejected` | Security | Parent lacking `GRANT` attempts to mint child; rejected with `Err`. |
| `test_mint_into_occupied_slot_rejected`| Security | Attempting to mint into non-empty slot; rejected without mutating destination. |
| `test_mint_self_reference_rejected` | Security | Attempting `src == dest`; rejected immediately without CDT cycle. |
| `test_badged_endpoint_minting` | Badging | Unbadged endpoint (`badge=0`) mints child with `badge=0x1234`. |
| `test_badged_endpoint_tamper_rejected`| Security | Badged endpoint (`badge=0x1234`) attempts to mint child with `badge=0x5678`; rejected. |
| `test_mmio_execute_rejected` | Security | Attempting to mint `MmioFrame` with `CapRights::EXECUTE`; rejected. |
| `test_frame_wx_conflict_rejected` | Security | Attempting to mint `Frame` with both `WRITE` and `EXECUTE`; rejected. |
| `test_cdt_sibling_chain_integrity` | Derivation | Parent mints 3 children; verify `first_child` and `next_sibling` traversal. |
| `test_revocation_descendants_only` | Revocation | Parent $P$ has child $C$ and grandchild $G$. Revoking $P$ deletes $C$ and $G$; $P$ intact. |
| `test_iterative_revocation_deep_tree`| Stress | Mint chain of depth 16; verify iterative revocation succeeds without stack overflow. |
| `test_generation_counter_increment` | Lifecyle | Slot reuse increments `generation` from 1 to 2; empty slot has generation 0. |
| `test_generation_wraparound_skips_zero`| Lifecyle | Slot at `0xFFFF` wrapped on reuse becomes `1`, skipping `0`. |
| `test_cascading_deletion` | Deletion | Deleting parent with live children recursively revokes children and clears parent. |
| `test_task_cspace_teardown` | Teardown | `destroy_task_cspace` cleans all 128 slots and unlinks all CDT links cleanly. |

### 10.2 Integration / QEMU Validation (Stage 3.3+)
- Testing Ring 3 `syscall` instruction dispatch for `SYS_CAP_MINT` and `SYS_CAP_REVOKE`.
- Validating page table unmapping upon frame revocation with TLB invalidation.

---

## 11. Explicit Acceptance Criteria

Stage 3.1 will be considered complete and ready for integration when:

1. **Static Assertions Pass:** `size_of::<CNodeSlot>() == 32`, `size_of::<CNode>() == 4096`, and alignment assertions compile cleanly.
2. **Zero Dynamic Allocation:** The capability subsystem introduces zero dynamic heap allocations. CDT is statically sized to 2,048 entries.
3. **Non-Recursive Revocation:** Revocation is provably iterative with bounded $O(1)$ stack usage.
4. **Rights Attenuation Enforced:** No capability can be minted or copied with greater rights than its parent.
5. **No Regressions:** All existing 43 unit tests in `paging.rs` and all kernel build targets continue to compile and pass with zero warnings.
6. **New Test Suite:** All 19 unit tests specified in the test matrix pass in host CI.
7. **Clean Scope:** No Stage 3.2, 3.3, or 3.4 code (frame mapping syscalls, notifications, ELF loader, virtio bounce buffers) is introduced.

---

## 12. Remaining Uncertainties & Deferred Work

1. **Cross-Task IPC Capability Passing Protocol:** Milestone 3 design requires client tasks to pass `Frame` capabilities to storage drivers. In Stage 3.1, capability derivation is established within task CSpaces and between tasks. The exact IPC message register ABI for capability transfer (e.g. transmitting a capability in `arg3`/`arg4` during `SYS_SEND`/`SYS_CALL`) is scheduled for final standardization in **Stage 3.3**.
2. **Notification Object Allocation:** `CapType::Notification` is defined in the enum, but the notification bitmask state and PIC masking logic belong strictly to **Stage 3.2**.
3. **Physical Memory Aliasing Registry:** Full system-wide physical aliasing enforcement across multiple tasks is governed by `Frame` derivation constraints in Stage 3.1, with user mapping interfaces deferred to **Stage 3.3**.
