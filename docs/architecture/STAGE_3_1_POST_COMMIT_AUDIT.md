# Stage 3.1 Capability Engine: Post-Commit Independent Audit

**Repository:** `medhansh5/Janus`  
**Branch:** `main`  
**Committed HEAD:** `f3130f9a19ab9efc2cc3e5b9a37ca76f36221b6a`  
**Parent Commit:** `b58d23aebbee2a5ce625cb5b612b42274d5449c4` (`main`)  
**Commit Message:** `feat(capability): implement secure capability delegation and revocation`  
**Audit Mode:** STRICTLY READ-ONLY INDEPENDENT AUDIT  
**Date:** 2026-11-25  

---

## 1. Executive Summary & Audit Verdict

### Final Verdict: APPROVED

An exhaustive, adversarial, independent audit was conducted on the committed Stage 3.1 implementation in `medhansh5/Janus` at commit `f3130f9a19ab9efc2cc3e5b9a37ca76f36221b6a`.

The implementation faithfully satisfies all invariants and architectural guarantees established in `STAGE_3_1_FINAL_SPEC.md`:
1. **Handle Security:** 64-bit generation-bearing `CPtr` encoding enforces canonical zero-bit masks (`0xFFFF_0000_0000_FF80`), rejects generation zero, and enforces strict generation equality across all resolution paths.
2. **Permanent Retirement:** Slots reaching generation `u32::MAX` are permanently retired via saturating addition and allocation rejection, eliminating ABA recycling vulnerabilities.
3. **Revocation Integrity:** In-situ leaf-pruning revocation is protected by a complete, read-only preflight traversal (`validate_revocation_subtree`) executing in $O(1)$ stack space (256-byte bitset) without dynamic memory allocation or recursion. All structural errors, malformed links, and cycles are rejected prior to any mutation.
4. **Least Authority (POLA-01):** Production bootstrap helpers (`init_task_a_bootstrap_cnode` and `init_task_b_bootstrap_cnode`) are shared between `kernel_main` and unit tests. Task B driver capabilities (`IOPort` and `Irq`) are stripped of `CapRights::GRANT`.
5. **Hardware Isolation:** Multi-byte I/O port arithmetic overflow and boundary overruns are checked before inline `in` / `out` assembly.
6. **Isolation:** Cross-task capability delegation is strictly kernel-internal.

All 78 unit tests pass (43 paging tests + 35 capability tests). Formatting, Clippy, workspace, and bare-metal target checks pass with zero errors and zero warnings.

Stage 3.1 is **APPROVED** without reservation as a robust, mathematically sound foundation for Stage 3.2.

---

## 2. Repository & Commit Baseline Verification

The repository baseline was independently confirmed via Git plumbing:

- **Repository Origin:** `https://github.com/medhansh5/Janus.git`
- **Active Branch:** `main`
- **HEAD Commit SHA:** `f3130f9a19ab9efc2cc3e5b9a37ca76f36221b6a`
- **Parent Commit SHA:** `b58d23aebbee2a5ce625cb5b612b42274d5449c4` (`feat(mm): complete dynamic user address spaces`)
- **Commit Log:**
  ```text
  f3130f9 feat(capability): implement secure capability delegation and revocation
  b58d23a feat(mm): complete dynamic user address spaces
  ```
- **Committed File Scope:** Exactly three files modified:
  - `kernel/src/capability.rs` (+2212, -3 lines)
  - `kernel/src/ipc.rs` (+136, -136 lines)
  - `kernel/src/main.rs` (+12, -35 lines)
- **Whitespace / Diff Integrity:** `git diff HEAD^ HEAD --check` produced zero warnings or formatting errors.
- **Untracked Directory:** `docs/architecture/` remains intact, untracked, and untouched by Git operations.
- **Working Tree:** 100% clean of tracked changes; zero staged changes.

---

## 3. Inspected Source Files & Subsystem Map

The audit inspected the following committed functions and subsystems:

| File | Functions / Components Inspected | Lines | Responsibility |
| :--- | :--- | :---: | :--- |
| `kernel/src/capability.rs` | `CPtr`, `CNodeSlot`, `CNode`, `CdtNode` | 1–173 | Layout, alignment assertions, canonical encoding |
| `kernel/src/capability.rs` | `lookup_slot`, `get_task_cnode_mut` | 246–263 | CSpace handle resolution & direct-map validation |
| `kernel/src/capability.rs` | `mint_cap_internal`, `mint_cap`, `copy_cap` | 269–417 | Derivation, attenuation, badge immutability, W^X |
| `kernel/src/capability.rs` | `mint_cap_cross_task` | 423–434 | Kernel-internal delegation primitive |
| `kernel/src/capability.rs` | `validate_revocation_subtree` | 490–617 | $O(1)$ stack preflight traversal |
| `kernel/src/capability.rs` | `revoke_descendants_iterative`, `revoke_cap` | 624–679 | In-situ iterative leaf pruning |
| `kernel/src/capability.rs` | `delete_cap`, `cleanup_slot`, `unlink_node` | 437–473, 689–717 | Cascading deletion & slot sanitization |
| `kernel/src/capability.rs` | `validate_io_port_access` | 721–760 | Checked arithmetic & multi-byte bounds |
| `kernel/src/capability.rs` | `init_task_a_bootstrap_cnode`, `init_task_b_bootstrap_cnode` | 777–878 | Shared bootstrap initialization |
| `kernel/src/capability.rs` | `destroy_task_cspace` | 761–774 | CSpace teardown primitive |
| `kernel/src/capability.rs` | Unit Test Suites (35 tests) | 881–2197 | Regression verification & malformed snapshot tests |
| `kernel/src/ipc.rs` | `lookup_cap`, `handle_syscall` | 18–30, 94–111 | Syscall dispatch & handle lookup |
| `kernel/src/ipc.rs` | `sys_send`, `sys_recv`, `sys_in_port`, `sys_out_port` | 114–160, 399–478 | Authorized IPC and hardware I/O dispatch |
| `kernel/src/ipc.rs` | `sys_cap_mint`, `sys_cap_copy`, `sys_cap_revoke`, `sys_cap_delete` | 480–528 | Capability management syscalls |
| `kernel/src/main.rs` | `kernel_main_stage2` | 394–455 | Production bootstrap CNode initialization |

---

## 4. Specification Compliance Matrix

| Specification Invariant (`STAGE_3_1_FINAL_SPEC.md`) | Implementation Mechanism | Status | Notes |
| :--- | :--- | :---: | :--- |
| **Exact 32-Byte `CNodeSlot` Budget** | Compile-time assertions: `size_of::<CNodeSlot>() == 32`, `align_of == 8` | **COMPLIANT** | Offsets verified: `cap_type: 0`, `rights: 1`, `_reserved1: 2`, `generation: 4`, `payload_a: 8`, `payload_b: 16`, `cdt_index: 24`, `_reserved2: 28`. |
| **Exact 4096-Byte `CNode` Page** | Compile-time assertions: `size_of::<CNode>() == 4096`, `align_of == 4096` | **COMPLIANT** | 128 slots fit cleanly in a 4 KiB physical page frame. |
| **Deterministic Static CDT (2048 entries)** | `pub static mut CDT: [CdtNode; 2048]`, size of `CdtNode == 12` | **COMPLIANT** | Index computed via `((task << 7) \| (slot & 0x7F))`. BSS footprint = 24 KiB. |
| **64-bit Generation-Bearing `CPtr`** | `(self.0 & 0xFFFF_0000_0000_FF80) == 0 && generation != 0` | **COMPLIANT** | Reserved bits 7..15 and 48..63 strictly rejected; generation zero rejected. |
| **Stale Handle Rejection** | Exact check: `slot.generation == cptr.generation()` | **COMPLIANT** | Reused slots increment generation, invalidating outstanding handles. |
| **Permanent Slot Retirement** | `saturating_add(1)` on delete; `generation == u32::MAX => SlotExhausted` | **COMPLIANT** | Prevents ABA cycle wraparound. |
| **Rights Attenuation Invariant** | `src_slot_ref.rights.contains(new_rights)` | **COMPLIANT** | Rights amplification impossible. `GRANT` required on source capability. |
| **Global Physical W^X on Frames** | `!(new_rights.contains(WRITE) && new_rights.contains(EXECUTE))` | **COMPLIANT** | Enforced at mint/copy time on `CapType::Frame`. |
| **MMIO Execution Forbidden** | `!new_rights.contains(CapRights::EXECUTE)` | **COMPLIANT** | Enforced at mint/copy time on `CapType::MmioFrame`. |
| **Endpoint Badge Immutability** | If `src.payload_b != 0`, `new_badge == src.payload_b` | **COMPLIANT** | Existing badges cannot be overwritten or stripped. |
| **1-Slot-to-1-Mapping Invariant** | `payload_b` records `UNMAPPED_VADDR` on mint; unmapped on cleanup | **COMPLIANT** | Enables $O(1)$ page unmapping on revocation (Stage 3.3). |
| **Read-Only Preflight Revocation** | `validate_revocation_subtree` in $O(1)$ auxiliary stack space | **COMPLIANT** | 256-byte stack bitset; checks bounds, loops, parent consistency, and slot alignment. |
| **In-Situ Iterative Revocation** | Iterative leaf unlinking; $O(1)$ stack space; zero dynamic allocation | **COMPLIANT** | Destructive phase prunes descendants cleanly and preserves root capability. |
| **Least Privilege Drivers (POLA-01)** | Task B slots 5 (`IOPort`) and 6 (`Irq`) assigned `READ \| WRITE` (no `GRANT`) | **COMPLIANT** | Production bootstrap helpers shared with unit tests. |
| **I/O Port Checked Arithmetic (SEC-04)** | `validate_io_port_access`: `offset + width <= size`, `port + width - 1 <= 0xFFFF` | **COMPLIANT** | Executed before inline `in` / `out` assembly. Width restricted to 1, 2, or 4 bytes. |
| **Cross-Task Mint Isolation** | `mint_cap_cross_task` has no syscall exposure | **COMPLIANT** | Kernel-internal only. Untrusted user space cannot mint cross-task. |

---

## 5. Security and Correctness Audit Findings

### 5.1 Finding Post-Audit-01 (INFORMATIONAL): Direct-Map Boundary Validation
- **Source Location:** `kernel/src/capability.rs:229-232` (`get_task_cnode_mut`), `kernel/src/ipc.rs:99-102` (`lookup_cap`)
- **Severity:** **INFORMATIONAL**
- **Description:**  
  When dereferencing a task's `root_cnode.0` virtual address, the kernel checks:
  ```rust
  if vaddr < crate::paging::PHYS_MEMORY_OFFSET
      || (vaddr & 0xFFF) != 0
      || vaddr >= crate::paging::PHYS_MEMORY_OFFSET + crate::paging::DIRECT_MAP_LIMIT
  {
      return Err(CapError::InvalidCNodePointer);
  }
  ```
  The lower and upper bounds ensure `root_cnode` resides within the valid physical direct map. `vaddr & 0xFFF == 0` guarantees 4 KiB frame alignment. This defends the kernel from executing arbitrary pointer dereferences if a task structure were somehow corrupted.
- **Assessment:** Excellent defensive design.

### 5.2 Finding Post-Audit-02 (INFORMATIONAL): Malformed Snapshot Test Sequence
- **Source Location:** `kernel/src/capability.rs:1974-2197`
- **Severity:** **INFORMATIONAL**
- **Description:**  
  In the five malformed-tree regression tests (`test_revocation_preflight_out_of_bounds_child_rejected_without_mutation`, `test_revocation_preflight_malformed_parent_rejected_without_mutation`, `test_revocation_preflight_cycle_rejected_without_mutation`, `test_revocation_preflight_empty_slot_rejected_without_mutation`, `test_revocation_preflight_mismatched_cdt_index_rejected_without_mutation`), the pre-call snapshot is captured **immediately after injecting the corruption and prior to calling `revoke_cap`**.
  The post-error assertions compare all 128 CNode slots and 2048 CDT entries directly against this corrupted pre-call snapshot:
  ```rust
  assert_eq!(revoke_cap(0, CPtr::new(1, 1)), Err(CapError::OutOfBounds));
  let cnode_after = *get_task_cnode_mut(0).unwrap();
  for i in 0..CNODE_SLOTS {
      assert_eq!(cnode_after.slots[i], cnode_snapshot.slots[i]);
  }
  for i in 0..TOTAL_CDT_SLOTS {
      assert_eq!(CDT[i], cdt_snapshot[i]);
  }
  ```
  The test harness only cleans up the corrupted field *after* the comparison to prevent state pollution in subsequent tests.
- **Assessment:** Fully satisfies the snapshot audit methodology required by previous reviews. Proves zero-mutation behavior on validation failure.

---

## 6. Detailed Subsystem Analysis

### 6.1 Handle Encoding & Wildcard Prevention (SEC-01, SEC-03)
- Canonical validation mask `CPtr::RESERVED_MASK = 0xFFFF_0000_0000_FF80`.
- Bits 7..15 and 48..63 must be zero. Slot 128 (bit 7 set) is rejected.
- Generation 0 is rejected as non-canonical.
- `lookup_slot` in `capability.rs:259` and `lookup_cap` in `ipc.rs:107` enforce `slot.generation == cptr.generation()`.
- Result: Wildcard handles, stale handles, and aliased slot indices are mathematically impossible.

### 6.2 Revocation Authority & Preflight Algorithm (SEC-02)
- `sys_cap_revoke` extracts `current_task_id` strictly from `crate::task::current_task_id()`.
- `revoke_cap` executes `lookup_slot(caller_task, cptr)?` and verifies `slot.rights.contains(CapRights::GRANT)`. Holders without derivation authority cannot revoke.
- `validate_revocation_subtree`:
  - Visited set: `[u64; 32]` (256 bytes) allocated on the stack.
  - Step counter ceiling: $2 \times \text{TOTAL\_CDT\_SLOTS} = 4096$.
  - Traversals correctly ascend via parent links and cross via sibling links.
  - Self-loops, back-pointers to `root_idx`, parent-mismatches, out-of-bounds indices, and empty/mismatched CNode slots return immediate errors before any mutation.

### 6.3 Driver Authority (POLA-01)
- `init_task_b_bootstrap_cnode` initializes:
  - Slot 5: `CapType::IOPort`, rights `READ | WRITE`, payload `(virtio_io_base as u64) | (64u64 << 16)`.
  - Slot 6: `CapType::Irq`, rights `READ | WRITE`, payload `virtio_irq as u64`.
- Neither capability possesses `GRANT`.
- The driver cannot derive, mint, or transfer hardware I/O or IRQ capabilities.

### 6.4 I/O Port Checked Arithmetic (SEC-04)
- `validate_io_port_access` strictly restricts `width` to `1`, `2`, or `4`.
- Bounds check: `(offset as u32) + (width as u32) <= (size as u32)`.
- Base addition: `base.checked_add(offset)` prevents 16-bit wraparound.
- Port space check: `(port as u32) + (width as u32) - 1 <= 0xFFFF`.
- Syscalls `sys_in_port` and `sys_out_port` call validation before inline assembly. Hardware `in`/`out` instructions are completely unreachable upon validation failure.

---

## 7. Deferred Lifecycle Limitations

The audit confirmed the exact boundary between Stage 3.1 guarantees and deferred features:

1. **`destroy_task_cspace` Runtime Integration (Deferred to Stage 3.4):**
   - `destroy_task_cspace(task_id: u8)` is implemented in `capability.rs:761-774` and verified in unit tests.
   - It is currently not called from `task.rs` or from fault termination in `sys_send`.
   - **Safety Justification:** Janus currently allocates only static tasks (Task A = ID 0, Task B = ID 1, Idle = ID 15). There is no dynamic task creation (`fork`/`spawn`), no exit syscall, and no task-ID recycling mechanism. Consequently, no process can ever inherit an existing task ID or its orphaned slots.
   - **Stage 3.4 Dependency:** When dynamic task creation and process reaping are introduced in Stage 3.4, `destroy_task_cspace` must be invoked upon task exit before any task ID can be recycled.
2. **Dynamic User Page Mapping / IPC Transfer (Deferred to Stage 3.3):**
   - Syscalls `SYS_MAP_FRAME` and `SYS_UNMAP_FRAME`, as well as cross-task capability transfer during synchronous IPC, remain scheduled for Stage 3.3.

---

## 8. Independent Verification Suite Results

All five independent verification checks were executed on the committed tree:

| # | Verification Command | Exit Code | Result Status |
| :-: | :--- | :---: | :--- |
| **1** | `cargo +nightly fmt --all -- --check` | `0` | **PASS** (Zero diffs) |
| **2** | `cargo +nightly test -p kernel --lib` | `0` | **PASS** (**78 passed, 0 failed**, 0 ignored) |
| **3** | `cargo +nightly clippy -p kernel --lib --tests -- -D warnings` | `0` | **PASS** (Zero warnings) |
| **4** | `cargo +nightly check --workspace` | `0` | **PASS** (Clean) |
| **5** | `cargo +nightly check -p kernel --target x86_64-unknown-none` | `0` | **PASS** (Clean) |

- **Unit Test Breakdown:**
  - 43 Paging / VMM Unit Tests: ALL PASSED
  - 35 Capability Engine Unit Tests: ALL PASSED

*(Bare-metal QEMU boot execution was not performed in this task; all validation was established via host test suites, compile-time assertions, and bare-metal target compilation).*

---

## 9. Stage 3.2 Readiness Assessment

Stage 3.2 is **Asynchronous Notifications & IRQ Routing**. The audit evaluated whether the committed Stage 3.1 capability engine provides a sound foundation:

1. **Notification Object Representation:** `CapType::Notification` is already assigned discriminant `3` in `CapType`. `payload_a` is designated for the notification object ID, and `payload_b` can store a badge or event mask.
2. **Access Control & Rights:**
   - Signaling/binding a notification requires `CapRights::WRITE`.
   - Waiting on a notification requires `CapRights::READ`.
   - Deriving/delegating a notification requires `CapRights::GRANT`.
3. **Delegation & Revocation:** Notification capabilities integrate seamlessly into `mint_cap`, `copy_cap`, `revoke_cap`, and `delete_cap`. Revoking a parent notification capability will cleanly prune derived handles held by worker threads or child tasks.
4. **IRQ Routing Synergy:** Hardware interrupts can bind to notification objects or wake blocked tasks (as prototyped in `ipc.rs:381-397` for `CapType::Irq`).
5. **Readiness Verdict:** There are **zero blockers** in the capability subsystem for Stage 3.2.

---

## 10. Audit Conclusion

The committed Stage 3.1 implementation at `f3130f9a19ab9efc2cc3e5b9a37ca76f36221b6a` is correct, secure, mathematically sound, and fully verified.

### **FINAL AUDIT VERDICT: APPROVED**
