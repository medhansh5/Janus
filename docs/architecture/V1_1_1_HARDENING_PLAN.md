# Janus OS — V1.1.1 Hardening Implementation Plan

**Repository:** `medhansh5/Janus`  
**Branch:** `main`  
**Baseline HEAD:** `f3130f9a19ab9efc2cc3e5b9a37ca76f36221b6a`  
**Date:** 2026-11-27  

---

## 1. Scope & Objectives

This hardening milestone closes four specific gaps identified during the V1.1 Final Release Evidence Audit without expanding feature scope (no SMP, no NVMe, no modern VirtIO, no async block I/O):

1. **Capability Syscall Negative Tests (Syscalls 24–30)**
2. **Transactional Filesystem Allocation & Error Handling (`kernel/src/fs.rs`)**
3. **IRQ Ownership Lifecycle & Cleanup (`kernel/src/capability.rs`, `kernel/src/notification.rs`, `kernel/src/idt.rs`)**
4. **Hardware & Architecture Support Documentation (`docs/ARCHITECTURE.md`, `docs/LIMITATIONS.md`, `README.md`)**

---

## 2. Detailed Implementation Plan & Acceptance Criteria

### 2.1. Capability Syscall Negative Tests (`kernel/src/ipc.rs`)
- **Design:** Add direct unit tests in `kernel/src/ipc.rs` (`#[cfg(test)] mod tests`) that invoke `crate::ipc::handle_syscall(...)` directly across syscalls 24–30 (`SYS_READ_CONSOLE`, `SYS_WRITE_CONSOLE`, `SYS_FS_CREATE`, `SYS_FS_READ`, `SYS_FS_WRITE`, `SYS_FS_DELETE`, `SYS_FS_LIST`).
- **Cases Covered per Syscall:**
  - Invalid `CPtr` (non-canonical reserved bits, out-of-bounds slot, empty slot).
  - Stale `CPtr` generation (after capability deletion/revocation).
  - Wrong capability type (`CapType::Storage` for console syscalls; `CapType::Console` for storage syscalls).
  - Missing `CapRights::READ` (write-only cap for syscalls 24, 27, 30).
  - Missing `CapRights::WRITE` (read-only cap for syscalls 25, 26, 28, 29).
  - Unauthorized task (Task 1 lacking Console/Storage capabilities).
  - Positive control (Task 0 with valid capabilities succeeds).
- **Acceptance Criteria:**
  - Every unauthorized call returns `!0`.
  - User output buffers, console input queue, and JanusFS state remain completely unmodified on rejected calls.
  - Rejected calls with null/kernel pointers still fail at capability validation before user-memory copy.

### 2.2. Transactional Filesystem Allocation (`kernel/src/fs.rs`)
- **Design:**
  - Change `flush_block_to_disk(block_idx: u16) -> Result<(), ()>` and propagate disk write errors from `write_file`, `delete_file`, and `sync`.
  - Refactor `write_file(name: &str, data: &[u8]) -> Result<usize, ()>` to:
    1. Validate `data.len() <= MAX_FILE_SIZE` and non-empty filename before any mutation.
    2. Locate existing inode or reserve a free inode index without committing.
    3. Compute `needed_blocks` and pre-allocate all missing blocks before mutating `INODES` or `BLOCK_CACHE`; if allocation fails (disk full), leave `INODES` and `BLOCK_CACHE` completely unchanged.
    4. Save old `Inode` and old `BLOCK_CACHE` contents of modified blocks; write data blocks and flush data blocks -> inode sector -> superblock in order.
    5. On any disk flush failure, roll back `INODES` and modified `BLOCK_CACHE` blocks and return `Err(())`.
  - Clamp `inode.size` against `MAX_FILE_SIZE` in `read_file`.
  - Add deterministic test-only disk write failure injection (`TEST_FAIL_ON_WRITE`) to exercise I/O error rollback.
- **Explicit Limitation:** Without journaling or copy-on-write, true power-loss atomicity across multi-sector flushes is not claimed; guarantees cover in-memory consistency, disk-full rejection, and synchronous VirtIO write-error rollback.
- **Acceptance Criteria:** All 7 required FS regression tests pass (create, overwrite, disk-full create, disk-full overwrite preserving old contents, mid-operation I/O failure rollback, delete after write, post-failure consistency).

### 2.3. IRQ Ownership Lifecycle (`kernel/src/notification.rs`, `kernel/src/capability.rs`, `kernel/src/idt.rs`)
- **Design:**
  - Track per-IRQ masked state (`IRQ_MASKED: [bool; 16]`) in `notification.rs`, updated when `idt.rs` masks an IRQ (`mark_irq_masked`) and when `sys_irq_ack` unmasks it (`mark_irq_unmasked`).
  - Add `notification::cleanup_irq_on_cap_removal(irq: u8)` invoked from `capability::cleanup_slot(task_id, slot)` whenever a `CapType::Irq` slot is cleared (via `delete_cap`, `revoke_cap`, or `destroy_task_cspace`).
  - `cleanup_irq_on_cap_removal(irq)` scans active `CNode`s across all `MAX_TASKS` (after the slot is cleared) to determine whether any remaining `CapType::Irq` capability for `irq` still exists.
  - Only if **zero** remaining `CapType::Irq` capabilities for `irq` exist:
    - Unbind `IRQ_BINDINGS[irq]` and clear `bound_irq` on any bound notification object.
    - If `IRQ_MASKED[irq]` is true, clear `IRQ_MASKED[irq] = false` and call `crate::pic::unmask(irq)` (under `#[cfg(not(test))]`).
  - If another task or slot still owns a `CapType::Irq` for `irq`, do **not** unmask or unbind it.
  - Preserve existing single-EOI-per-real-IRQ and spurious IRQ7/IRQ15 handling in `idt.rs` and `pic.rs`.
- **Acceptance Criteria:** All 6 required IRQ lifecycle tests pass (task destruction owning IRQ, task destruction while IRQ masked, IRQ cap revocation, multiple caps to same IRQ, ack after revocation, destruction of non-owning task).

### 2.4. Hardware & Architecture Documentation
- Update `docs/ARCHITECTURE.md`, `docs/LIMITATIONS.md`, and `README.md` to accurately reflect:
  - Uniprocessor x86_64 execution, no SMP, `USER_RSP_SCRATCH` / `KERNEL_RSP_SCRATCH` with `SFMask` `IF=0` assumptions.
  - Legacy VirtIO 0.9.5 block transport (PCI `0x1AF4:0x1001`, I/O BAR, 256-entry queue, synchronous polling, 2M spin timeout, no async IRQ11 completion).
  - JanusFS 512B block layout, 64-inode / 64-cached-block / 2560B max-file limits, cross-reboot persistence, disk-full rollback, and lack of power-loss journaling.
