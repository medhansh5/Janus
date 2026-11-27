# Janus OS — Stage 3.2A Implementation Report
## Core Asynchronous Software Notifications

**Mode:** CONTROLLED IMPLEMENTATION — COMPLETED  
**Repository:** `medhansh5/Janus`  
**Working Directory:** `C:\Users\medha\.gemini\antigravity\scratch\Janus`  
**Branch:** `main` (tracking `origin/main`)  
**Baseline HEAD:** `f3130f9a19ab9efc2cc3e5b9a37ca76f36221b6a`  
**Parent:** `b58d23aebbee2a5ce625cb5b612b42274d5449c4`  
**Authoritative Design:** `docs/architecture/STAGE_3_2_NOTIFICATION_DESIGN.md` (Version 2.1.0)  
**Date:** 2026-11-26  

---

## 1. Baseline Verification & Repository State

Prior to code changes and across all validation stages, the repository state was strictly verified against the Stage 3.1 post-commit baseline:

- **Repository Root:** `C:\Users\medha\.gemini\antigravity\scratch\Janus`
- **Active Branch:** `main`
- **Baseline Commit:** `f3130f9a19ab9efc2cc3e5b9a37ca76f36221b6a` (Stage 3.1 Capability Security Engine commit)
- **Parent Commit:** `b58d23aebbee2a5ce625cb5b612b42274d5449c4`
- **Initial Verification:** All 78 baseline tests (35 capability, 43 paging) passed cleanly with 0 errors.

Following Stage 3.2A implementation, all modifications remain strictly unstaged in the local working tree:
- **Modified Tracked Files (4):**
  - `kernel/src/capability.rs`
  - `kernel/src/ipc.rs`
  - `kernel/src/lib.rs`
  - `kernel/src/task.rs`
- **New Files (2):**
  - `kernel/src/notification.rs`
  - `docs/architecture/STAGE_3_2A_IMPLEMENTATION_REPORT.md`
- **Staging / Commit Status:** 0 staged changes, 0 commits created, 0 branches created, 0 remotes contacted.

---

## 2. Architectural Compliance Matrix (Design v2.1.0 vs Implemented)

| Design v2.1.0 Requirement | Implementation Location | Compliance Status |
| :--- | :--- | :--- |
| Static pool of 64 `NotificationObject`s | `kernel/src/notification.rs` (`NOTIFICATIONS`) | **COMPLIANT** (Fixed array of 64 entries) |
| Generation tracking & saturation retirement | `NotificationObject::generation`, `reset_notification_object` | **COMPLIANT** (Permanently retired at `u32::MAX`) |
| `payload_a = (generation << 32) \| notif_id` | `install_notification_root`, `sys_notify_*` validation | **COMPLIANT** (Enforces both generation and ID match) |
| `payload_b = badge` (Bit 63 reserved) | `mint_cap_internal`, `sys_notify_signal` | **COMPLIANT** (Bit 63 rejected; Endpoint bit 63 preserved) |
| `u16` Reference counting | `NotificationObject::ref_count`, `increment_notification_ref` | **COMPLIANT** (Bounded to `u16::MAX`; increments on mint/copy) |
| Centralized cleanup via `cleanup_slot` | `kernel/src/capability.rs` (`cleanup_slot`) | **COMPLIANT** (Decrements refcount on revoke/delete/exit) |
| Single-waiter invariant | `NotificationObject::waiting_task` | **COMPLIANT** (`!0` busy rejection if waiter already registered) |
| Cooperative scheduling integration | `kernel/src/task.rs` (`BlockedOnNotification`) | **COMPLIANT** (Registers waiter, sets state, yields via `schedule()`) |
| Asynchronous event coalescing | `NotificationObject::pending_bits` | **COMPLIANT** (Bitwise OR with bits 0..=62; consumed on wait/poll) |
| Syscalls 9, 10, 15, 16 | `kernel/src/ipc.rs`, `kernel/src/notification.rs` | **COMPLIANT** (Wired to `handle_syscall`) |
| Return ABI: `!0` sentinel / bitmask | `sys_notify_create`, `signal`, `wait`, `poll` | **COMPLIANT** (0 or `!0` for signal/create; bitmask or `!0` for wait/poll) |
| Strict Stage 3.2B isolation | Hardware IRQ/PIC/IDT unchanged | **COMPLIANT** (Zero hardware/interrupt routing introduced) |

---

## 3. Object Pool & Generation Encoding

### 3.1 Object Layout & Static Allocation
`NotificationObject` is defined in `kernel/src/notification.rs` with exact 8-byte alignment:
```rust
pub struct NotificationObject {
    pub pending_bits: u64,           // 8 bytes (coalescing event mask)
    pub generation: u32,             // 4 bytes (generational ABA guard)
    pub ref_count: u16,              // 2 bytes (active capability reference counter)
    pub active: bool,                // 1 byte  (allocation state)
    pub owner_task: Option<u8>,      // 1 byte  (task that created the object)
    pub waiting_task: Option<u8>,    // 1 byte  (single registered waiter)
    pub bound_irq: Option<u8>,       // 1 byte  (reserved for Stage 3.2B)
}
```
The pool `NOTIFICATIONS` is statically allocated:
```rust
pub static mut NOTIFICATIONS: [NotificationObject; MAX_NOTIFICATIONS] =
    [NotificationObject::empty(); MAX_NOTIFICATIONS];
```
Zero heap allocations (`alloc`) are performed across any notification operation.

### 3.2 Generational ABA Protection
Each Notification capability encodes both the generation and object ID in `payload_a`:
$$\text{payload\_a} = (\text{generation} \ll 32) \mid \text{notif\_id}$$
During any operation (`sys_notify_signal`, `sys_notify_wait`, `sys_notify_poll`), the kernel validates:
1. `slot.cap_type == CapType::Notification`
2. `notif_id < MAX_NOTIFICATIONS`
3. `notif.active == true`
4. `(slot.payload_a >> 32) as u32 == notif.generation`
5. `(slot.payload_a & 0xFFFF_FFFF) as usize == notif_id`

If an object is recycled, its generation is incremented during `reset_notification_object`. Stale capabilities possessing older generation tags immediately fail validation and return `!0`. When generation reaches `u32::MAX`, the object is permanently retired and skipped by `allocate_notification`.

---

## 4. Reference Counting Lifecycle & Capability Engine Integration

### 4.1 Reference Counting Bound & Validation
`ref_count` is tracked as `u16`. In Janus OS, each task has 256 CNode slots, and there are 16 tasks (maximum 4096 CSpace slots system-wide). Thus, the maximum possible simultaneous capability references to any single object cannot exceed 4096, which is strictly less than $u16::\text{MAX} = 65535$.

### 4.2 Increments on Derivation
In `kernel/src/capability.rs` (`mint_cap_internal`), when deriving a Notification capability via `mint_cap` or `copy_cap`:
```rust
if src_slot_ref.cap_type == CapType::Notification {
    let notif_id = (src_slot_ref.payload_a & 0xFFFF_FFFF) as usize;
    crate::notification::increment_notification_ref(notif_id)?;
}
```
If `ref_count` would overflow, derivation is safely aborted with `CapError::SlotExhausted` before any CNode or CDT mutation occurs.

### 4.3 Decrements on Slot Cleanup
In `kernel/src/capability.rs` (`cleanup_slot`):
```rust
if slot_ref.cap_type == CapType::Notification {
    let notif_id = (slot_ref.payload_a & 0xFFFF_FFFF) as usize;
    crate::notification::decrement_notification_ref(notif_id);
}
```
Because all deletion paths (`delete_cap`, `revoke_cap`, `revoke_descendants_iterative`, and `cleanup_task_cspace`) route directly through `cleanup_slot`, reference decrements are guaranteed to execute exactly once per erased capability slot.

When `ref_count` drops to 0, `reset_notification_object` is automatically invoked:
- Cancels any waiting task by waking it with `!0` in `ipc_message`.
- Clears pending bits and metadata.
- Advances `generation` by 1 (or saturates at `u32::MAX`).
- Sets `active = false`.

---

## 5. Capability Badging & Attenuation

In accordance with REV-02 resolution:
- **Notification Capabilities:** In `mint_cap_internal`, badge mutation is restricted to bits 0..=62:
  ```rust
  CapType::Notification => {
      if (new_badge & (1u64 << 63)) != 0 {
          return Err(CapError::InvalidBadge);
      }
      if src_slot_ref.payload_b != 0 && new_badge != src_slot_ref.payload_b {
          return Err(CapError::InvalidBadge);
      }
  }
  ```
  Once badged (`payload_b != 0`), the badge is strictly immutable and cannot be overwritten or cleared by child derivations.
- **Endpoint Capabilities:** Completely unconstrained by the notification mask rule; full 64-bit badges (including bit 63) remain permitted.
- **Signal Semantics:** If `slot.payload_b != 0`, `sys_notify_signal` ignores user-supplied bits and sets only the capability's badge bitmask (`notif.pending_bits |= slot.payload_b`).

---

## 6. Syscall Subsystem Implementation

The four Stage 3.2A syscalls were assigned and implemented:

| Syscall | Number | Arguments | Return Value | Semantics |
| :--- | :--- | :--- | :--- | :--- |
| `SYS_NOTIFY_CREATE` | 9 | `arg1`: dest_slot | 0 on success, `!0` on error | Allocates object from pool, installs root capability with `READ \| WRITE \| GRANT` |
| `SYS_NOTIFY_SIGNAL` | 10 | `arg1`: cptr, `arg2`: bits | 0 on success, `!0` on error | Requires `WRITE` rights. Coalesces bits (or badge) into `pending_bits`. Wakes waiter if present |
| `SYS_NOTIFY_WAIT` | 15 | `arg1`: cptr | bitmask or `!0` on error | Requires `READ` rights. Fast path if bits pending; otherwise blocks caller on object |
| `SYS_NOTIFY_POLL` | 16 | `arg1`: cptr | bitmask (0 if none) or `!0` | Requires `READ` rights. Non-blocking atomic read-and-clear |

---

## 7. Cooperative Scheduling & IPC Delivery

When a task calls `sys_notify_wait` on a notification object with no pending events (`pending_bits == 0`):
1. The calling task is registered in `notif.waiting_task = Some(task_id)`.
2. The calling task's state is transitioned to `TaskState::BlockedOnNotification { notification_id: notif_id as u16 }`.
3. The scheduler yields execution to the next runnable task via `unsafe { crate::task::schedule() }`.
4. When `sys_notify_signal` is later invoked:
   - The registered waiter's `ipc_message` is populated with the consumed bitmask: `waiter.ipc_message = pending`.
   - `notif.pending_bits` is atomically cleared to 0.
   - `notif.waiting_task` is reset to `None`.
   - The waiter's state is transitioned to `TaskState::Runnable`.
5. When the waiter is resumed by the scheduler, its syscall return handler delivers `current_task.ipc_message`.

---

## 8. Unit Test Suite & Regression Verification

A suite of 31 comprehensive unit tests was developed in `kernel/src/notification.rs::tests`. All 109 kernel unit tests pass with zero failures:

```
running 109 tests
test capability::tests::test_badged_endpoint_immutability ... ok
test capability::tests::test_cnode_size_and_alignment ... ok
test capability::tests::test_cnodeslot_size_and_alignment ... ok
test capability::tests::test_cptr_encode_decode ... ok
test capability::tests::test_cross_task_derivation ... ok
test capability::tests::test_cycle_detection_and_prevention ... ok
test capability::tests::test_deep_iterative_revocation ... ok
test capability::tests::test_delete_leaf_unlinks_from_sibling_chain ... ok
test capability::tests::test_frame_wx_conflict_rejected ... ok
test capability::tests::test_generation_advancement_and_exhaustion ... ok
test capability::tests::test_invalid_slot_and_task_indices ... ok
test capability::tests::test_mmio_execute_rejected ... ok
test capability::tests::test_multiple_siblings ... ok
test capability::tests::test_occupied_destination_rejection ... ok
test capability::tests::test_parent_child_cdt_linkage ... ok
test capability::tests::test_parent_deletion_cascades ... ok
test capability::tests::test_pola01_task_b_bootstrap_rights ... ok
test capability::tests::test_revocation_preflight_cycle_rejected_without_mutation ... ok
test capability::tests::test_revocation_preflight_empty_slot_rejected_without_mutation ... ok
test capability::tests::test_revocation_preflight_malformed_parent_rejected_without_mutation ... ok
test capability::tests::test_revocation_preflight_mismatched_cdt_index_rejected_without_mutation ... ok
test capability::tests::test_revocation_preflight_out_of_bounds_child_rejected_without_mutation ... ok
test capability::tests::test_revocation_preflight_valid_deep_tree ... ok
test capability::tests::test_rights_attenuation ... ok
test capability::tests::test_rights_escalation_rejection ... ok
test capability::tests::test_sec01_generation_zero_rejected_across_all_ops ... ok
test capability::tests::test_sec02_revoke_handles_and_authority ... ok
test capability::tests::test_sec03_canonical_cptr_reserved_bits ... ok
test capability::tests::test_sec04_io_port_validation ... ok
test capability::tests::test_self_mint_rejection ... ok
test capability::tests::test_stale_handle_rejection ... ok
test capability::tests::test_task_cspace_cleanup ... ok
test capability::tests::test_valid_capability_lookup ... ok
test capability::tests::test_valid_copy ... ok
test capability::tests::test_valid_mint ... ok
test notification::tests::test_endpoint_badge_immutability_preserved ... ok
test notification::tests::test_endpoint_bit63_permitted ... ok
test notification::tests::test_endpoint_copy_and_mint_preserved ... ok
test notification::tests::test_endpoint_full_64bit_badge_permitted ... ok
test notification::tests::test_notif_badged_signal_enforces_badge_and_ignores_caller_bits ... ok
test notification::tests::test_notif_copy_increments_refcount ... ok
test notification::tests::test_notif_copy_mint_identity_preservation ... ok
test notification::tests::test_notif_create_initializes_refcount_one ... ok
test notification::tests::test_notif_delete_decrements_refcount ... ok
test notification::tests::test_notif_deletion_wakes_blocked_waiter ... ok
test notification::tests::test_notif_empty_slot_cleanup_does_not_decrement ... ok
test notification::tests::test_notif_failed_mint_leaves_refcount_unchanged ... ok
test notification::tests::test_notif_generation_exhaustion_permanently_retires ... ok
test notification::tests::test_notif_invalid_id_rejected ... ok
test notification::tests::test_notif_invalid_mask_bit63_rejected ... ok
test notification::tests::test_notif_last_reference_removal_cleans_object ... ok
test notification::tests::test_notif_mint_increments_refcount ... ok
test notification::tests::test_notif_multiple_coalesced_signals ... ok
test notification::tests::test_notif_payload_a_generation_encoding ... ok
test notification::tests::test_notif_poll_empty_returns_zero ... ok
test notification::tests::test_notif_poll_with_pending_bits ... ok
test notification::tests::test_notif_recycled_object_stale_cap_rejected ... ok
test notification::tests::test_notif_recursive_revoke_decrements_descendants ... ok
test notification::tests::test_notif_refcount_overflow_defense ... ok
test notification::tests::test_notif_rights_failures ... ok
test notification::tests::test_notif_signal_after_wake_before_consumption ... ok
test notification::tests::test_notif_signal_followed_by_wait_fast_path ... ok
test notification::tests::test_notif_signal_without_waiter_coalesces ... ok
test notification::tests::test_notif_single_waiter_busy_rejection ... ok
test notification::tests::test_notif_stale_generation_rejected ... ok
test notification::tests::test_notif_wait_followed_by_signal ... ok
test paging::tests::test_allocation_failure_during_create_user_pml4 ... ok
test paging::tests::test_copy_1gb_huge_page_reserved_bits ... ok
test paging::tests::test_copy_1gb_huge_page_with_pat ... ok
test paging::tests::test_copy_2mb_huge_page_reserved_bits ... ok
test paging::tests::test_copy_2mb_huge_page_with_pat ... ok
test paging::tests::test_copy_arithmetic_overflow ... ok
test paging::tests::test_copy_cross_page_boundary ... ok
test paging::tests::test_copy_cross_page_boundary_atomic_no_partial_mutation ... ok
test paging::tests::test_copy_direct_map_address ... ok
test paging::tests::test_copy_kernel_address ... ok
test paging::tests::test_copy_non_canonical_address ... ok
test paging::tests::test_copy_null_pointer ... ok
test paging::tests::test_copy_physical_address_exceeds_direct_map ... ok
test paging::tests::test_copy_pml4_huge_page_reserved ... ok
test paging::tests::test_copy_supervisor_only_pd ... ok
test paging::tests::test_copy_supervisor_only_pdpt ... ok
test paging::tests::test_copy_supervisor_only_pml4 ... ok
test paging::tests::test_copy_supervisor_only_pt ... ok
test paging::tests::test_copy_unmapped_address ... ok
test paging::tests::test_copy_valid_mapped_page ... ok
test paging::tests::test_create_user_pml4_frees_root_frame_on_higher_half_copy_failure ... ok
test paging::tests::test_destroy_user_pml4_does_not_free_leaf_frames ... ok
test paging::tests::test_destroy_user_pml4_protects_authoritative_kernel_root_even_when_cr3_differs ... ok
test paging::tests::test_destroy_user_pml4_protects_currently_active_root ... ok
test paging::tests::test_empty_address_space_destruction ... ok
test paging::tests::test_failed_mapping_preserves_existing_page_tables ... ok
test paging::tests::test_failure_at_intermediate_allocation_rolls_back ... ok
test paging::tests::test_huge_page_encounters_during_map_and_unmap ... ok
test paging::tests::test_kernel_half_and_noncanonical_addresses_rejected ... ok
test paging::tests::test_map_already_mapped_returns_error ... ok
test paging::tests::test_map_user_page_triggers_tlb_invalidation_only_on_success_in_active_address_space ... ok
test paging::tests::test_multiple_mappings_in_independent_address_spaces ... ok
test paging::tests::test_page_table_frames_zeroed_before_linking ... ok
test paging::tests::test_physical_address_direct_map_limit ... ok
test paging::tests::test_reader_unaligned_page_table_address ... ok
test paging::tests::test_repeated_map_unmap_no_leaks ... ok
test paging::tests::test_shared_leaf_frames_survive_independent_unmap ... ok
test paging::tests::test_teardown_preserves_higher_half_kernel_entries ... ok
test paging::tests::test_unaligned_addresses_rejected ... ok
test paging::tests::test_unmap_does_not_free_leaf_frame ... ok
test paging::tests::test_unmap_missing_intermediate_entries ... ok
test paging::tests::test_user_mappings_cannot_obtain_supervisor_access ... ok
test paging::tests::test_virtual_wx_violations_rejected ... ok

test result: ok. 109 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
```

---

## 9. Stage 3.2B Isolation & Non-Interference

The scope of Stage 3.2A was strictly constrained to software notifications:
- **Zero changes** to `kernel/src/pic.rs`, `kernel/src/idt.rs`, or `kernel/src/pci.rs`.
- **Zero changes** to hardware interrupt lines, interrupt vectors, or device dispatch.
- **Zero drivers or DMA/MMIO mappings** modified or added.
- `NotificationObject::bound_irq` is reserved and initialized to `None`. No binding syscall or routing logic has been implemented.

---

## 10. Build & Verification Results

All required verification suites were executed and passed cleanly:

1. **Source Code Formatting Check:**
   ```bash
   cargo +nightly fmt --all --manifest-path Cargo.toml -- --check
   ```
   *Result:* Exit 0 (Clean formatting).
2. **Clippy Linter (`-D warnings`):**
   ```bash
   cargo +nightly clippy --manifest-path Cargo.toml -p kernel --lib --tests -- -D warnings
   ```
   *Result:* Exit 0 (Zero warnings).
3. **Workspace Type-Check:**
   ```bash
   cargo +nightly check --manifest-path Cargo.toml --workspace
   ```
   *Result:* Exit 0 (All workspace crates compile cleanly).
4. **Bare-Metal Target Compilation:**
   ```bash
   cargo +nightly check --manifest-path Cargo.toml -p kernel --target x86_64-unknown-none
   ```
   *Result:* Exit 0 (Clean compilation on bare-metal x86_64 target).
5. **Full Unit Test Suite:**
   ```bash
   cargo +nightly test --manifest-path Cargo.toml -p kernel --lib
   ```
   *Result:* Exit 0 (109 passed, 0 failed).

---

## 11. Final Verdict

**VERDICT: STAGE 3.2A IMPLEMENTATION COMPLETE AND VERIFIED**

The Stage 3.2A asynchronous notification system is fully implemented in the working tree, verified against the approved Version 2.1.0 specification, and strictly validated across host unit tests and bare-metal targets. No Git commits, branch modifications, or remote operations have been performed.
