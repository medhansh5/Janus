# Janus OS — Milestone 3 Architecture Revision Notes
**Date:** November 24, 2026  
**Document Version:** 1.1.0 (Stage 3.0 Final Clarification Amendment)  
**Baseline Commit:** `d288cadd17aee34ddac16f95de89ddb16b329f92` (`main`)  
**Associated Documents:**
- Revised Design: `docs/architecture/MILESTONE_3_DESIGN.md` (v2.1.0)
- Independent Review: `docs/architecture/MILESTONE_3_REVIEW.md` (v1.0.0)
- Second Independent Review: `docs/architecture/MILESTONE_3_SECOND_REVIEW.md` (v1.0.0)

---

## 1. Executive Overview & Purpose

Following the second independent security and systems review of Milestone 3 (`MILESTONE_3_SECOND_REVIEW.md`), this document summarizes the final architectural amendments incorporated into `MILESTONE_3_DESIGN.md` (v2.1.0). These amendments close the remaining memory ownership, physical aliasing, CNode struct layout, and DMA proxy authority questions before authorizing Stage 3.0 implementation.

---

## 2. Summary of Prior Architectural Revisions (v2.0.0)

1. **Re-Sequenced Dependency Pipeline:** Stage 3.0 (Dynamic Virtual Memory Management Subsystem) established as an internal prerequisite. User-facing mapping syscalls deferred to Stage 3.3.
2. **DMA Threat Boundary:** Acknowledged hardware non-IOMMU limitations; prohibited direct bus-mastering DMA; mandated Trusted Kernel Bounce-Buffer Proxy.
3. **Driver Binary Packaging:** Statically linked ELF64 executables embedded in `.rodata` with strict kernel validation pipeline.
4. **Split Capability Derivation:** 32-byte CNode slots with derivation links tracked in a centralized kernel Capability Derivation Table (CDT).

---

## 3. Final Stage 3.0 Amendments & Corrections (v2.1.0)

### 3.1 New Memory Ownership Contract
The paging subsystem (`kernel/src/paging.rs`) explicitly classifies physical frames into three distinct categories with strict ownership invariants:
- **Page-Table Frames (Kernel-Owned):**
  - Dynamically allocated by `map_user_page` and `create_user_pml4` via `PhysFrameAllocator`.
  - **Zeroing Invariant:** Newly allocated frames from the freelist contain unzeroed stale pointers; every table frame must be zeroed across all 4,096 bytes via direct-map (`PHYS_MEMORY_OFFSET + paddr`) before use.
  - Reclaimed only upon `destroy_user_pml4` or empty intermediate table cleanup.
  - Higher-half kernel entries (256..511) must never be traversed or freed during user address space teardown.
- **Leaf Frames (Caller-Owned):**
  - Supplied by caller to `map_user_page`. The mapper does not acquire lifecycle ownership.
  - `unmap_user_page` clears the leaf PT entry, flushes TLB, and returns the physical address `paddr` **without** freeing the frame back to `PhysFrameAllocator`.
  - `destroy_user_pml4` reclaims intermediate page-table structures, but **does not free leaf frames**. The caller retains exclusive responsibility for leaf-frame lifetime.
- **Shared Frames:**
  - A physical leaf frame may be mapped into multiple address spaces simultaneously. Unmapping in one address space leaves other mappings intact.
  - Because `FrameAllocator` lacks reference counts, Stage 3.0 relies on a **caller-enforced lifetime invariant**: callers must ensure all mappings are unmapped before freeing a shared frame.
- **Failure & Rollback Semantics:**
  - Intermediate table allocations during `map_user_page` are tracked atomically. Partial failure rolls back and frees only newly allocated intermediate tables, preserving pre-existing tables.

### 3.2 Physical Aliasing & W^X Policy
- **Virtual vs. Physical W^X:**
  - `map_user_page` enforces virtual W^X on each mapping (rejecting pages with both `WRITABLE` and executable).
  - However, checking a single mapping cannot prevent physical aliasing (mapping physical frame $P$ as writable at $V_1$ and executable at $V_2$).
  - Full physical aliasing enforcement requires a centralized physical frame mapping registry, which is formally deferred to Stage 3.1 capability authorization.
  - **Documented Boundary:** Stage 3.0 provides per-mapping virtual W^X; global physical W^X is caller-enforced until Stage 3.1.
- **User/Supervisor Boundary:** Intermediate user entries have `USER_ACCESSIBLE` set to satisfy x86_64 architectural traversal; higher-half kernel entries have `U/S = 0` (supervisor only).

### 3.3 Corrected CNode Slot Layout (Strict 32 Bytes)
The previous layout had an alignment bug where natural 8-byte alignment padding before `payload_a` expanded the struct to 40 bytes when combined with `_reserved: [u8; 8]`. The corrected layout uses `_reserved: [u8; 4]`:
```rust
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CNodeSlot {
    pub cap_type: CapType,         // offset  0, size 1
    pub rights: CapRights,         // offset  1, size 1
    pub generation: u16,           // offset  2, size 2
    // 4 bytes natural alignment padding (offset 4..8)
    pub payload_a: u64,            // offset  8, size 8 (alignment 8)
    pub payload_b: u64,            // offset 16, size 8 (alignment 8)
    pub cdt_index: u32,            // offset 24, size 4
    pub _reserved: [u8; 4],        // offset 28, size 4 (padding to 32 bytes)
}
const _: () = assert!(core::mem::size_of::<CNodeSlot>() == 32);
const _: () = assert!(core::mem::align_of::<CNodeSlot>() == 8);
```
- **Deterministic CDT Indexing:** Statically mapped as `CDT[(task_id * 128) + slot_index]` across the 2,048 system-wide slots, eliminating CDT allocation failures and exhaustion.
- **Generation Wraparound:** Generation is a 16-bit integer; incrementing wraps from `0xFFFF` to `1` (skipping `0`, which is reserved for empty/unallocated slots).

### 3.4 Corrected DMA Proxy Authority Boundary
- **Zero Register Delegation:** All text and diagrams granting the Ring 3 driver an `IOPort` capability to the VirtIO BAR (`0xC000..0xC040`) have been eliminated.
- The Ring 3 driver receives **NO** `IOPort` or `MmioFrame` capability to VirtIO registers.
- All hardware registers (ports `0xCF8/0xCFC`, BAR `0xC000..0xC040`), virtqueues, and bounce buffers are owned **exclusively by the kernel**.
- The driver communicates with the block subsystem purely via a high-level IPC request interface (`SYS_CALL` with `BLOCK_READ`/`BLOCK_WRITE`).
- **Conservative Hardware Quiescence:** PCI Command Register Bit 2 (Bus Master) is cleared, followed by a PCI readback flush to force completion of posted PCI cycles, followed by writing status 0 and polling status with a watchdog timeout.

---

## 4. Remaining Deferred Limitations

1. **Global Physical-Frame Aliasing Registry:** Deferred to Stage 3.1 capability authority.
2. **Hardware IOMMU (Intel VT-d / AMD-Vi):** Deferred to Milestone 4. Milestone 3 operates exclusively via trusted kernel bounce-buffer proxying.
3. **Symmetric Multiprocessing (SMP) & Cross-Core TLB Shootdown:** Deferred to Milestone 5. Janus operates strictly as a uniprocessor kernel.
4. **General-Purpose Dynamic Linker & Multi-Page CNode Trees:** Deferred.

---

## 5. Stage 3.0 MVP Scope & Approval Status

- **Permitted File Modifications:** STRICTLY `kernel/src/paging.rs` and its internal test module.
- **Prohibited:** Zero modifications to `main.rs`, `syscall.rs`, `ipc.rs`, `task.rs`, `capability.rs`, `Cargo.toml`, or CI workflows.
- **Status:** **READY FOR STAGE 3.0 IMPLEMENTATION PROMPT.**
