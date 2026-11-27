# Janus OS — V1.1 Remediation & Release Plan
## Remediating Audit Findings for Persistent Storage, Capability-Enforced Services, IRQ Correctness, and Syscall Safety

**Date:** 2026-11-27  
**Repository:** `medhansh5/Janus`  
**Target Release:** Janus OS V1.1  
**Baseline HEAD:** `f3130f9a19ab9efc2cc3e5b9a37ca76f36221b6a`

---

## 1. Audit Findings Confirmation & Analysis

| Finding ID | Severity | Description | Audit Status | Root Cause & Technical Details |
|---|---|---|---|---|
| **STORAGE-01** | P2 (High Functional) | JanusFS is RAM-backed; VirtIO transport not implemented | **CONFIRMED** | `pci.rs` discovers VirtIO at `0x1AF4:0x1001` and enables PCI bus mastering, but `fs.rs` only read/writes to in-memory `BLOCK_CACHE`. No virtqueue rings or request descriptors are implemented. |
| **CAP-01** | P1 (High Security) | Console and filesystem syscalls operate via ambient authority | **CONFIRMED** | Syscalls 24–30 take direct user pointers without verifying a `CPtr` handle or validating rights. Violates capability-based security model. |
| **IRQ-01** | P2 (Medium Integrity) | Duplicate PIC EOI behavior | **CONFIRMED** | IDT IRQ dispatchers (vectors 33..47 in `idt.rs`) call `pic::end_of_interrupt()` immediately, and `sys_irq_ack` in `notification.rs` calls it again. In cascaded 8259 PIC, duplicate EOIs corrupt interrupt prioritization. |
| **SYSCALL-01** | P1 (High Concurrency) | Global syscall scratch registers unsafe for reentrancy | **CONFIRMED** | `USER_RSP_SCRATCH` and `KERNEL_RSP_SCRATCH` in `syscall.rs` are single mutable globals. While uniprocessor execution with `SFMask` clearing `IF` provides serialization, reentrancy or future multicore scaling causes stack clobbering. |
| **IRQ-02** | P3 (Low Security) | IRQ acknowledgement lacks WRITE-right validation | **CONFIRMED** | `sys_irq_ack` in `notification.rs` verifies `cap_type == CapType::Irq` but does not assert `slot.rights.contains(CapRights::WRITE)`. |

---

## 2. Architecture Decisions & Design Specifications

### 2.1 STORAGE-01: Legacy VirtIO-Blk Block Driver & JanusFS Disk Persistence
1. **Transport:** Implement VirtIO 0.9.5 Legacy PCI Block Driver in `kernel/src/virtio_blk.rs` (or `pci.rs` / `fs.rs`).
2. **Device State:**
   - Reset device: write 0 to Device Status (`io_base + 18`).
   - Acknowledge device: status |= 1 (`ACKNOWLEDGE`).
   - Driver status: status |= 2 (`DRIVER`).
   - Read/negotiate features (`io_base + 0` / `io_base + 4`).
   - Configure Queue 0: write 0 to Queue Select (`io_base + 14`), read Queue Size (`q_size = inw(io_base + 12)`).
   - Allocate contiguous DMA-safe physical memory for `VRingDesc` (16 bytes * `q_size`), `VRingAvail`, and 4KB-aligned `VRingUsed`.
   - Write physical page frame number (`paddr >> 12`) to Queue Address (`io_base + 8`).
   - Driver OK: status |= 4 (`DRIVER_OK`).
3. **Request Lifecycle:**
   - Three-descriptor chain:
     - Desc 0 (Header): `VirtioBlkOutHdr { type, ioprio: 0, sector }`, `flags = VRING_DESC_F_NEXT`.
     - Desc 1 (Data): 512-byte sector buffer, `flags = VRING_DESC_F_NEXT | (WRITE if read else 0)`.
     - Desc 2 (Status): 1-byte status, `flags = VRING_DESC_F_WRITE`.
   - Update `avail.ring` and `avail.idx`, execute memory fence, write 0 to Queue Notify (`io_base + 16`).
   - Bounded spin-poll on `used.idx` (timeout threshold e.g. 500,000 iterations).
   - Validate status byte `0` (`VIRTIO_BLK_S_OK`).
4. **JanusFS Integration:**
   - During `fs::init()`, read block 0 (sector 0) via VirtIO-Blk.
   - If magic matches `0x4A414E53`, load superblock and inode table directly from disk!
   - If disk is unformatted, format disk, write seeded files (`/welcome.txt`, `/help.txt`, `/version.txt`), and flush to disk.
   - All `write_file`, `create_file`, and `delete_file` operations flush modified blocks to disk sectors immediately.

### 2.2 CAP-01: Capability Enforcement for Console & Filesystem
1. **Capability Types:**
   - Expand `CapType` enum in `kernel/src/capability.rs`:
     - `CapType::Console = 7`
     - `CapType::Storage = 8`
2. **Rights Model:**
   - `CapType::Console`:
     - `CapRights::READ`: Allows reading console input (`SYS_READ_CONSOLE`).
     - `CapRights::WRITE`: Allows writing to console output (`SYS_WRITE_CONSOLE`).
   - `CapType::Storage`:
     - `CapRights::READ`: Allows directory listing (`SYS_FS_LIST`) and reading file contents (`SYS_FS_READ`).
     - `CapRights::WRITE`: Allows file creation (`SYS_FS_CREATE`), file writing (`SYS_FS_WRITE`), and file deletion (`SYS_FS_DELETE`).
3. **Syscall Signatures:**
   - Syscall 24: `SYS_READ_CONSOLE(cptr, buf_ptr, max_len)`
   - Syscall 25: `SYS_WRITE_CONSOLE(cptr, buf_ptr, len)`
   - Syscall 26: `SYS_FS_CREATE(cptr, path_ptr, path_len)`
   - Syscall 27: `SYS_FS_READ(cptr, path_ptr, path_len, buf_ptr, max_len)`
   - Syscall 28: `SYS_FS_WRITE(cptr, path_ptr, path_len, buf_ptr, len)`
   - Syscall 29: `SYS_FS_DELETE(cptr, path_ptr, path_len)`
   - Syscall 30: `SYS_FS_LIST(cptr, buf_ptr, max_len)`
4. **Bootstrap & Shell Authority:**
   - In `kernel/src/main.rs`, during task setup:
     - Install `CapType::Console` in Task A (Shell) at slot 8 with rights `READ | WRITE`.
     - Install `CapType::Storage` in Task A (Shell) at slot 9 with rights `READ | WRITE`.
   - Update user-space shell payload (`task_a_payload`) to pass CPtr 8 for console and CPtr 9 for storage.
   - Add negative tests verifying that unprivileged tasks or attenuated rights are rejected with error `!0`.

### 2.3 IRQ-01 & IRQ-02: IRQ Protocol & Rights Correctness
1. **Single EOI Protocol:**
   - In `kernel/src/idt.rs`:
     - When hardware interrupt vectors (33..47) fire:
       - Check if spurious (IRQ 7/15). If spurious IRQ 7, return with NO EOI. If spurious IRQ 15, send master EOI and return.
       - If genuine IRQ bound to a notification: **Mask the IRQ line in PIC**, deliver notification event, and **send single EOI immediately**.
   - In `kernel/src/notification.rs` (`sys_irq_ack`):
     - Validate `irq_slot.cap_type == CapType::Irq`.
     - **Enforce `irq_slot.rights.contains(CapRights::WRITE)`** (Fixes `IRQ-02`).
     - **Unmask the IRQ line in PIC** to re-enable interrupts on that line (Fixes `IRQ-01`).
     - **Do NOT send a second EOI** to the PIC!

### 2.4 SYSCALL-01: Syscall Entry & Stack Safety
1. **Uniprocessor Invariant:**
   - On x86_64, `syscall` automatically clears `RFlags::INTERRUPT_FLAG` via `SFMask`.
   - Interrupts are disabled from the first instruction of `syscall_entry` until `iretq` or explicit re-enable.
2. **Hardening Stack Switch:**
   - Structure `SyscallEntryState` with clear ownership.
   - Clear `USER_RSP_SCRATCH` immediately upon pushing user stack onto kernel stack:
     ```asm
     mov [rip + USER_RSP_SCRATCH], rsp
     mov rsp, [rip + KERNEL_RSP_SCRATCH]
     push 0x1B
     push [rip + USER_RSP_SCRATCH]
     mov qword ptr [rip + USER_RSP_SCRATCH], 0
     ```
   - Ensure scheduler context switches always update `KERNEL_RSP_SCRATCH` to the active task's `kernel_stack_top`.

---

## 3. Implementation Sequence

1. **Step 1:** Update `kernel/src/capability.rs` with `CapType::Console = 7` and `CapType::Storage = 8`, adding helper minting and validation functions.
2. **Step 2:** Remediate `kernel/src/idt.rs` and `kernel/src/notification.rs` for single EOI protocol and `CapRights::WRITE` check.
3. **Step 3:** Implement VirtIO-Blk block driver and sector read/write in `kernel/src/fs.rs` / `kernel/src/pci.rs`, connecting JanusFS to disk.
4. **Step 4:** Enforce capabilities across syscalls 24–30 in `kernel/src/ipc.rs`, `kernel/src/console.rs`, and `kernel/src/fs.rs`.
5. **Step 5:** Update `kernel/src/main.rs` to bootstrap Console and Storage capabilities into Task A, and update `task_a_payload` shell assembly to pass capability handles.
6. **Step 6:** Harden syscall stack entry in `kernel/src/syscall.rs`.
7. **Step 7:** Run full quality gates: `cargo test`, `cargo clippy`, `cargo fmt`, kernel & bootloader builds.
8. **Step 8:** Run end-to-end QEMU reboot persistence verification and generate `docs/architecture/V1_1_REMEDIATION_REPORT.md`.
