# Stage 3.1 Dynamic Capability Engine: Independent Implementation Audit

**Repository:** `medhansh5/Janus`  
**Baseline Commit:** `b58d23aebbee2a5ce625cb5b612b42274d5449c4` (`main`)  
**Scope:** Stage 3.1 implementation in `kernel/src/capability.rs`, `kernel/src/ipc.rs`, and `kernel/src/main.rs`  
**Specification:** `docs/architecture/STAGE_3_1_FINAL_SPEC.md`  
**Mode:** STRICTLY READ-ONLY AUDIT (Zero Source Code Modifications)  
**Date:** 2026-11-25  

---

## 1. Executive Summary & Verdict

### Final Verdict: APPROVED WITH CONDITIONS

An exhaustive, adversarial audit of the Stage 3.1 Dynamic Capability Engine implementation was conducted across `kernel/src/capability.rs`, `kernel/src/ipc.rs`, and `kernel/src/main.rs`.

The core architectural foundations of Stage 3.1—namely the 32-byte `CNodeSlot` layout, 4096-byte `CNode`, 12-byte `CdtNode`, deterministic 24 KiB static CDT in `.bss`, non-recursive $O(1)$-stack iterative revocation, and kernel-isolated cross-task derivation—are successfully implemented, mathematically sound, compile-time asserted, and validated by 67 passing kernel unit tests.

However, the audit identified **two security-critical vulnerabilities** in handle validation and revocation dispatch, alongside two medium-severity bounds-checking defects, which must be remediated before committing Stage 3.1:

1. **CRITICAL (SEC-01): Generation 0 Wildcard Bypass.** Across all lookup and derivation paths (`lookup_slot`, `mint_cap_internal`, `delete_cap`, and `lookup_cap`), the generation check evaluates `if cptr.generation() != 0 && slot.generation != cptr.generation()`. When a user space caller passes generation 0, handle validation is completely bypassed, defeating the 32-bit generation-bearing protection against stale handles and slot reuse.
2. **HIGH (SEC-02): `SYS_CAP_REVOKE` Unvalidated Handle Dispatch.** `sys_cap_revoke(cptr)` passes `cptr.slot()` directly to `revoke_descendants_iterative` without verifying that the slot is non-empty, without checking handle generation, and without checking caller authority. A stale handle from a previously deleted capability can revoke descendants of a newly allocated capability in that slot.
3. **MEDIUM (SEC-03): CPtr Reserved Bit Validation Omission.** Bits 7–15 and Bits 48–63 are specified as reserved (must be zero) in `STAGE_3_1_FINAL_SPEC.md`. The implementation masks out these bits rather than rejecting non-zero reserved bits, accepting malformed handles.
4. **MEDIUM (SEC-04): I/O Port Checked Arithmetic & Multi-Byte Boundary Overrun.** In `sys_in_port` and `sys_out_port`, `base + offset` is unguarded against arithmetic overflow, and `offset < size` fails to account for operation width (2 or 4 bytes), allowing reads and writes up to 3 bytes beyond the capability's authorized port window.

Commit and PR creation are authorized **strictly upon resolving the required remediation conditions** detailed in Section 3.

---

## 2. Git and Scope Preflight Verification

Independent repository inspection confirmed the exact baseline state:

- **Repository:** `medhansh5/Janus` (Remote: `https://github.com/medhansh5/Janus.git`)
- **Active Branch:** `main` (Up to date with `origin/main`)
- **Exact HEAD Commit:** `b58d23aebbee2a5ce625cb5b612b42274d5449c4` (Merge PR #4: `feat(mm): complete dynamic user address spaces`)
- **Working Tree State:**
  - Modified tracked files:
    - `kernel/src/capability.rs` (+1266, -34 lines)
    - `kernel/src/ipc.rs` (+187, -47 lines)
    - `kernel/src/main.rs` (+125, -40 lines)
  - Staged changes: **0 files staged** (`git status --short` shows only unstaged modifications `M`).
  - Untouched files: `kernel/src/paging.rs`, `Cargo.toml`, `Cargo.lock`, CI workflows, Makefile, and linker scripts are completely untouched.
  - Untracked files: All 7 architecture documents in `docs/architecture/` remain intact and untracked.

---

## 3. Findings & Required Remediations Table

| ID | Severity | File & Line Reference | Finding Description | Security / Operational Impact | Required Remediation |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **SEC-01** | **CRITICAL** | `kernel/src/capability.rs:247`<br>`kernel/src/capability.rs:285`<br>`kernel/src/capability.rs:519`<br>`kernel/src/ipc.rs:105` | **Generation 0 Wildcard Bypass:** `lookup_slot`, `mint_cap_internal`, `delete_cap`, and `lookup_cap` check `if cptr.generation() != 0 && slot.generation != cptr.generation()`. | Any user process passing `cptr = CPtr(slot)` (generation 0) matches ANY generation, bypassing stale-handle rejection and use-after-revocation protection. | Remove `cptr.generation() != 0 &&`. Enforce `cptr.generation() != 0 && slot.generation == cptr.generation()`, rejecting generation 0 with `CapError::StaleHandle`. |
| **SEC-02** | **HIGH** | `kernel/src/ipc.rs:513-524`<br>`kernel/src/capability.rs:468-472` | **`SYS_CAP_REVOKE` Handle Unchecked:** `sys_cap_revoke` extracts `cptr.slot()` and invokes `revoke_descendants_iterative` without verifying that the slot is non-empty or that `cptr.generation()` matches. | A stale handle to a slot that has been recycled into a new capability tree can maliciously revoke descendants of the new capability. | In `sys_cap_revoke`, call `lookup_slot(current_task_id, cptr)?` to validate slot non-empty and generation match before revoking. |
| **SEC-03** | **MEDIUM** | `kernel/src/capability.rs:14-33`<br>`kernel/src/capability.rs:232-235`<br>`kernel/src/ipc.rs:93` | **CPtr Reserved Bits Not Validated:** `CPtr::slot()` and `CPtr::generation()` bitwise-mask fields without validating that Bits 7–15 and Bits 48–63 are zero. | Violates ABI specification. User space can pass malformed handles with garbage in bits 48..63 without rejection. | Validate `(cptr.0 & 0xFFFF_0000_0000_FF80) == 0`. Reject non-zero reserved bits with `CapError::OutOfBounds`. |
| **SEC-04** | **MEDIUM** | `kernel/src/ipc.rs:411-417`<br>`kernel/src/ipc.rs:457-463` | **I/O Port Arithmetic Overflow & Width Overrun:** `port = base + offset` can overflow `u16`. In addition, `offset < size` does not account for multi-byte `width` (2 or 4 bytes). | Integer overflow in debug builds; in release, reading/writing up to 3 bytes beyond the capability's authorized port window. | Use `(offset as u32) + (width as u32) <= (size as u32)` and checked addition `base.checked_add(offset)`. |
| **ABI-01** | **LOW** | `kernel/src/ipc.rs:497`<br>`kernel/src/ipc.rs:508` | **`SYS_CAP_MINT` / `SYS_CAP_COPY` Return 0:** Syscalls return `0` on success rather than the newly minted generation-bearing `CPtr`. | User space cannot discover the new handle's generation without probing `SYS_CAP_INFO`. | Return the raw `u64` of the new `CPtr` (or document the required user space probe sequence). |
| **POLA-01**| **LOW** | `kernel/src/main.rs:520-548` | **Excessive Rights on Bootstrap Capabilities:** Task B's root `IOPort` and `Irq` capabilities are granted `CapRights::GRANT`. | Violates least-authority principle. Drivers need operational rights (`READ \| WRITE`), not derivation rights. | Remove `CapRights::GRANT` from Task B's root `IOPort` and `Irq` slots in `main.rs`. |
| **ARCH-01**| **INFO** | `kernel/src/capability.rs:323`<br>`kernel/src/capability.rs:459` | **Permanent Slot Retirement vs Wrap-to-1:** Spec specified wrapping generation from `u32::MAX` to `1`. Implementation chose permanent retirement via `saturating_add(1)` and `dest.generation == u32::MAX => SlotExhausted`. | Permanent retirement is mathematically safer against ABA wraparound than wrapping. Diverges from spec text but is architecturally superior. | Formally record permanent retirement in `STAGE_3_1_FINAL_SPEC.md`. |
| **ARCH-02**| **INFO** | `kernel/src/capability.rs:538` | **`destroy_task_cspace` Not Wired into Scheduler:** Function is implemented and tested, but never called from `task.rs` or fault termination. | No immediate impact since Janus only runs static Tasks A & B; dynamic process termination is deferred to Stage 3.4/3.5. | Formally document `destroy_task_cspace` integration as deferred to Stage 3.4 process lifecycle. |

---

## 4. Detailed Engineering Audit by Subsystem

### 4.1 Critical ABI and Memory Layout

#### `CNodeSlot` (32 Bytes, 8-Byte Aligned)
Inspected `kernel/src/capability.rs:65-110`:
- **Size and Alignment:** Compile-time static assertions `size_of::<CNodeSlot>() == 32` and `align_of::<CNodeSlot>() == 8` are present and verified.
- **Field Offsets:**
  - `cap_type`: offset 0, size 1 (`repr(u8)`)
  - `rights`: offset 1, size 1 (`CapRights`)
  - `_reserved1`: offset 2, size 2 (`u16`, zeroed)
  - `generation`: offset 4, size 4 (`u32`)
  - `payload_a`: offset 8, size 8 (`u64`)
  - `payload_b`: offset 16, size 8 (`u64`)
  - `cdt_index`: offset 24, size 4 (`u32`)
  - `_reserved2`: offset 28, size 4 (`[u8; 4]`, zeroed)
- **Compile-Time Assertions:** `core::mem::offset_of!` assertions for all 8 fields are verified in source.
- **Empty Slot Initialization:** `CNodeSlot::empty()` explicitly sets all payloads, rights, and reserved fields to zero.

#### `CNode` (4096 Bytes, 4096-Byte Aligned)
Inspected `kernel/src/capability.rs:114-134`:
- `#[repr(C, align(4096))]` ensures page alignment.
- Exactly 128 slots ($128 \times 32 = 4096$).
- Compile-time assertions `size_of::<CNode>() == 4096` and `align_of::<CNode>() == 4096` verified.

#### `CdtNode` (12 Bytes, 4-Byte Aligned)
Inspected `kernel/src/capability.rs:136-161`:
- Fields: `parent: Option<u16>` (4B), `first_child: Option<u16>` (4B), `next_sibling: Option<u16>` (4B).
- Rust's niche optimization for `Option<u16>` is not assumed; size is verified with `size_of::<CdtNode>() == 12` and offset assertions at 0, 4, 8.

#### CDT Global Layout & Arithmetic Indexing
Inspected `kernel/src/capability.rs:163-186`:
- `pub static mut CDT: [CdtNode; 2048] = [CdtNode::empty(); 2048];` (24 KiB in `.bss`).
- Bijective arithmetic formulas:
  - `cdt_index(task, slot) = ((task as usize) << 7) | (slot & 0x7F)`
  - `cdt_task_id(idx) = (idx >> 7) as u8`
  - `cdt_slot_idx(idx) = (idx & 0x7F) as u8`
- Task bounds (`task < 16`) and slot bounds (`slot < 128`) are checked prior to indexing. User space cannot supply an arbitrary `cdt_index` to gain authority; `cdt_index` is purely an internal kernel identifier.

#### `CPtr` Encoding and Reserved Bits (SEC-01 & SEC-03)
Inspected `kernel/src/capability.rs:10-33`:
- Slot extracted as `(self.0 & 0xFFFF) as usize`. While `slot >= 128` triggers `OutOfBounds`, bits 7..15 and bits 48..63 are not checked to be zero.
- Generation extracted as `((self.0 >> 16) & 0xFFFF_FFFF) as u32`.
- **Defect (SEC-01):** Generation 0 is permitted to act as a wildcard in `lookup_slot`, `delete_cap`, and `lookup_cap`.

---

### 4.2 Generation Lifecycle and Slot Exhaustion

1. **Slot Initialization:** All empty slots are initialized with `generation: 0`. At boot, live slots for Task A and Task B are initialized with `generation: 1`.
2. **Handle Allocation & Minting:** `dest_slot_ref.generation` is checked against `u32::MAX`. If `< u32::MAX`, it advances `next_gen = dest_slot_ref.generation + 1`. The issued handle has `generation = next_gen >= 1`.
3. **Handle Invalidation on Deletion:** When a capability is deleted or revoked, `cleanup_slot` executes `slot_ref.generation = slot_ref.generation.saturating_add(1)`. This immediately invalidates the handle issued for that generation.
4. **Permanent Slot Retirement vs Saturating Add:**
   - If a slot reaches `u32::MAX` during active use, upon deletion `cleanup_slot` saturates at `u32::MAX`.
   - On subsequent minting, `mint_cap_internal` checks:
     ```rust
     if dest_slot_ref.generation == u32::MAX {
         return Err(CapError::SlotExhausted);
     }
     ```
   - The slot is permanently retired. It cannot be minted into, copied into, or delegated into.
   - Any query via `lookup_slot` on an exhausted slot returns `Err(CapError::EmptySlot)` because `slot.cap_type == CapType::Empty`.
   - Stale handle reuse is provably impossible under permanent retirement.
5. **Vulnerability (SEC-01):** Despite the generation counter advancement, any handle constructed with `CPtr(slot)` (generation 0) matches the slot regardless of how many times its generation has advanced, completely puncturing this lifecycle guarantee.

---

### 4.3 Capability Authority and Derivation Rules

1. **CSpace Isolation:**
   - In `sys_cap_mint`, `sys_cap_copy`, `sys_cap_revoke`, and `sys_cap_delete`, the task ID is obtained strictly from `crate::task::current_task_id()`. User space has no argument to specify a target task.
   - Cross-task derivation (`mint_cap_cross_task`) is `pub unsafe fn` and is NOT exposed to user space dispatch.
2. **Rights Attenuation Invariant:**
   - `src_slot_ref.rights.contains(new_rights)` is strictly verified.
   - Escalation (e.g. attempting to gain `WRITE` when parent only has `READ`) returns `Err(CapError::RightsEscalation)`.
   - Parent must possess `CapRights::GRANT`.
3. **Physical W^X and MMIO Execution Prohibition:**
   - For `CapType::Frame`: `!(new_rights.contains(WRITE) && new_rights.contains(EXECUTE))` is enforced.
   - For `CapType::MmioFrame`: `new_rights.contains(EXECUTE)` is strictly rejected with `CapError::InvalidRights`.
   - Since derivation is strictly monotonic, an executable frame can never derive a writable capability, and vice versa.
4. **Badge Immutability:**
   - For `CapType::Endpoint`: if `src_slot_ref.payload_b != 0` (badged), `new_badge` must equal `src_slot_ref.payload_b`. Overwriting or stripping an existing badge returns `Err(CapError::InvalidBadge)`.
   - Non-endpoint capabilities require `new_badge == 0`.
5. **Derivation Acyclicity:**
   - `src_task == dest_task && src_slot == dest_slot` is rejected as `CapError::SelfMint`.
   - Occupied destination slots are rejected with `CapError::OccupiedSlot`.
   - Destination CDT links must be all `None` before linking (`CapError::CycleDetected`).

---

### 4.4 Iterative Revocation and Deletion Algorithm

Inspected `kernel/src/capability.rs:468-503`:
1. **$O(1)$ Stack Space Proof:**
   - The algorithm maintains two `usize` indices: `root_idx` and `leaf`.
   - No recursion is used.
   - No dynamic memory allocation or worklist array is used.
2. **Descent & Unlink Mechanics:**
   - The inner loop `while let Some(grandchild) = CDT[leaf].first_child` descends strictly along `first_child` pointers to locate the leftmost leaf.
   - Because `leaf` is reached strictly by following `first_child`, `leaf` is guaranteed to equal `CDT[parent_idx].first_child`.
   - Unlinking updates `CDT[parent_idx].first_child = CDT[leaf].next_sibling`.
   - `leaf` is cleaned up and detached.
3. **Root Preservation:**
   - The outer loop condition is `while let Some(child_idx) = CDT[root_idx].first_child`.
   - The root node `root_idx` is never passed to `cleanup_slot`. Its capability slot remains intact, with its `first_child` becoming `None`.
4. **Cascading Deletion:**
   - `delete_cap` checks `if CDT[idx].first_child.is_some()`, calling `revoke_descendants_iterative` before unlinking the root node via `unlink_node` and invoking `cleanup_slot`.
5. **Cycle Guard:**
   - Both the outer loop and the inner descent loop maintain bounded iteration counters (`iterations > TOTAL_CDT_SLOTS` $\implies$ `Err(CapError::CycleDetected)`), preventing infinite loops in the presence of corrupted CDT data.
6. **Vulnerability (SEC-02):** `revoke_descendants_iterative` is safe when called internally, but `sys_cap_revoke` calls it without pre-validating the `cptr` handle against the calling task's CNode slot.

---

### 4.5 Syscall ABI and Dispatch

Inspected `kernel/src/ipc.rs:18-78`:
- `SYS_CAP_MINT = 13`: Maps to `sys_cap_mint(CPtr(arg1), arg2 as usize, arg3 as u8, arg4)`
- `SYS_CAP_REVOKE = 14`: Maps to `sys_cap_revoke(CPtr(arg1))`
- `SYS_CAP_COPY = 20`: Maps to `sys_cap_copy(CPtr(arg1), arg2 as usize)`
- `SYS_CAP_DELETE = 21`: Maps to `sys_cap_delete(CPtr(arg1))`
- `SYS_CAP_INFO = 5`: Maps to `sys_cap_info(CPtr(arg1))`
- **Syscall Number Collision Check:**
  - Existing syscalls: `SYS_SEND(1)`, `SYS_RECV(2)`, `SYS_REPLY(3)`, `SYS_YIELD(4)`, `SYS_CAP_INFO(5)`, `SYS_IN_PORT(6)`, `SYS_OUT_PORT(7)`, `SYS_LOG(8)`.
  - New capability syscalls: `13`, `14`, `20`, `21`.
  - No collisions exist.
- **`SYS_CAP_INFO` Non-Termination Contract:**
  - When invoked on an empty slot or invalid CPtr, `sys_cap_info` returns `!0` (`u64::MAX`).
  - It does NOT terminate the task or trigger a kernel panic.
  - For `Frame`/`MmioFrame` and `IOPort`, it returns `payload_a` to maintain compatibility with existing VirtIO driver discovery. For other capabilities, it returns `(type | (rights << 8) | (generation << 16))`.

---

### 4.6 Rights Enforcement Across Syscalls

1. **`sys_send`:** Checks `s.cap_type == CapType::Endpoint` and `s.rights.contains(CapRights::WRITE)`.
2. **`sys_recv`:**
   - For `CapType::Endpoint`: checks `cap.rights.contains(CapRights::READ)`.
   - For `CapType::Irq`: checks `cap.rights.contains(CapRights::READ)`.
3. **`wake_irq`:** Matches `slot.cap_type == CapType::Irq` and `slot.payload_a as u8 == irq`.
4. **`sys_in_port` & `sys_out_port`:**
   - `sys_in_port` checks `cap.cap_type == CapType::IOPort` and `cap.rights.contains(CapRights::READ)`.
   - `sys_out_port` checks `cap.cap_type == CapType::IOPort` and `cap.rights.contains(CapRights::WRITE)`.
   - **Defect (SEC-04):** Port arithmetic `port = base + offset` is unguarded against overflow and width overrun.

---

### 4.7 Task CSpace Lifecycle

- **Bootstrap Initialization:** Task A and Task B have their root CNodes allocated and populated in `kernel/src/main.rs`. All active slots are assigned `generation: 1`.
- **CSpace Isolation:** Tasks cannot inspect or manipulate other tasks' CNodes.
- **`destroy_task_cspace`:**
  - Implemented in `kernel/src/capability.rs:538`.
  - Iterates through all 128 slots, revoking descendants, unlinking CDT nodes, and clearing slots.
  - Tested in unit test `test_task_cspace_cleanup`.
  - **Status:** Not currently wired into task fault/termination handlers. Correctly classified as deferred functionality pending Stage 3.4 process management.

---

## 5. Independent Verification Suite Results

All five required verification commands were executed directly in the repository:

1. **Code Formatting:**
   ```bash
   $ cargo fmt --all -- --check
   Exit Code: 0 (Clean)
   ```
2. **Unit Test Suite:**
   ```bash
   $ cargo test -p kernel --lib
   Exit Code: 0
   Results: 67 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
   - 43 Paging / VMM Unit Tests: ALL PASSED
   - 24 Capability Unit Tests: ALL PASSED
   ```
3. **Clippy Static Analysis:**
   ```bash
   $ cargo clippy -p kernel --lib --tests -- -D warnings
   Exit Code: 0 (Zero warnings / Zero errors)
   ```
4. **Workspace Check:**
   ```bash
   $ cargo check --workspace
   Exit Code: 0 (Clean)
   ```
5. **Bare-Metal Target Check:**
   ```bash
   $ cargo check -p kernel --target x86_64-unknown-none
   Exit Code: 0 (Clean)
   ```

*Note on QEMU Runtime Verification:* Direct QEMU boot execution was not run during this turn. All verification was conducted via host unit tests, compile-time assertions, and bare-metal target compilation.

---

## 6. Scope and Regression Audit

- **Paging / VMM Integrity:** `kernel/src/paging.rs` is byte-identical to `main` at `b58d23a`. All 43 paging unit tests pass without deviation.
- **File Boundary Enforcement:** Exactly three tracked files are modified (`capability.rs`, `ipc.rs`, `main.rs`). No CI scripts, Cargo manifests, bootloader files, or architecture docs were modified.
- **Stage Boundary Enforcement:** No Stage 3.2 notifications, IDT PIC routing, or Stage 3.3 dynamic mapping syscalls were introduced.

---

## 7. Mandatory Remediation Checklist for Commit Authorization

To transition this audit verdict from **APPROVED WITH CONDITIONS** to **FINAL COMMIT AUTHORIZATION**, the following code changes must be applied:

- [ ] **Remedy SEC-01 (Generation-0 Wildcard):**
  - In `kernel/src/capability.rs` (`lookup_slot`, `mint_cap_internal`, `delete_cap`):
    Change:
    ```rust
    if cptr.generation() == 0 || slot.generation != cptr.generation() {
        return Err(CapError::StaleHandle);
    }
    ```
  - In `kernel/src/ipc.rs` (`lookup_cap`):
    Change:
    ```rust
    if slot.generation() == 0 || s.generation != slot.generation() {
        return Err(CapLookupError::StaleHandle);
    }
    ```
  - Update `test_stale_handle_rejection` to assert that `lookup_slot(0, CPtr(1))` returns `Err(CapError::StaleHandle)`.
- [ ] **Remedy SEC-02 (`sys_cap_revoke` Validation):**
  - In `kernel/src/ipc.rs:sys_cap_revoke`:
    ```rust
    fn sys_cap_revoke(cptr: CPtr) -> u64 {
        let current_task_id = match crate::task::current_task_id() {
            Some(id) => id as u8,
            None => return !0,
        };
        // Validate handle before revoking
        if unsafe { crate::capability::lookup_slot(current_task_id, cptr) }.is_err() {
            return !0;
        }
        match unsafe {
            crate::capability::revoke_descendants_iterative(current_task_id, cptr.slot() as u8)
        } {
            Ok(()) => 0,
            Err(_) => !0,
        }
    }
    ```
- [ ] **Remedy SEC-03 (CPtr Reserved Bits):**
  - In `kernel/src/capability.rs`:
    In `CPtr`:
    ```rust
    #[inline(always)]
    pub const fn is_canonical(self) -> bool {
        (self.0 & 0xFFFF_0000_0000_FF80) == 0 && self.generation() != 0
    }
    ```
    In `lookup_slot`, reject `!cptr.is_canonical()` with `CapError::OutOfBounds`.
- [ ] **Remedy SEC-04 (I/O Port Range Bounds):**
  - In `sys_in_port` and `sys_out_port`:
    ```rust
    let end_offset = match (offset as u32).checked_add(width as u32) {
        Some(e) => e,
        None => return !0,
    };
    if end_offset > (size as u32) {
        return !0;
    }
    let port = match base.checked_add(offset) {
        Some(p) => p,
        None => return !0,
    };
    ```
- [ ] **Remedy POLA-01 (Bootstrap Minimal Rights):**
  - In `kernel/src/main.rs`: remove `CapRights::GRANT` from Task B's root `IOPort` (slot 5) and `Irq` (slot 6).

---

*Report independently prepared for Janus OS project governance.*
