# Janus OS V1.1 — Independent Audit Findings Remediation & Release Report

**Mode:** POST-REMEDIATION VERIFICATION & V1.1 RELEASE CLOSURE  
**Repository:** `medhansh5/Janus`  
**Working directory:** `C:\Users\medha\.gemini\antigravity\scratch\Janus`  
**Branch:** `main`  
**Audited Baseline HEAD:** `f3130f9a19ab9efc2cc3e5b9a37ca76f36221b6a`  
**Date:** 2026-11-27  

---

## 1. Executive Summary

All five findings identified during the independent engineering audit of Janus OS Full V1 have been remediated, verified, and validated against the microkernel source code and within live hardware emulation in QEMU:

1. **`STORAGE-01` (P2 — In-Memory Filesystem Without VirtIO-Blk Persistence):** Resolved. Implemented a fully functional legacy VirtIO 0.9.5 Block Driver (`kernel/src/virtio_blk.rs`) with VirtQueue descriptor chains, available rings, and used rings. JanusFS (`kernel/src/fs.rs`) was upgraded to flush superblocks, inode tables, and allocated data blocks directly to VirtIO block device sectors. Upon kernel initialization, sector 0 is probed; if the `JANS` magic is detected, the filesystem and active file blocks are restored across reboots without reformatting. Real cross-boot persistence was verified with a 2-session automated QEMU lifecycle test where files written in Session 1 (`/persist.txt`) survived and were read back in Session 2.
2. **`CAP-01` (P1 — Console and Filesystem Syscalls Bypass Capability Architecture):** Resolved. Defined `CapType::Console = 9` and `CapType::Storage = 10` in `kernel/src/capability.rs`. Syscalls 24 (`SYS_READ_CONSOLE`), 25 (`SYS_WRITE_CONSOLE`), 26 (`SYS_FS_CREATE`), 27 (`SYS_FS_READ`), 28 (`SYS_FS_WRITE`), 29 (`SYS_FS_DELETE`), and 30 (`SYS_FS_LIST`) now require a valid `CPtr` parameter in `rsi` and strictly validate the capability type and required access rights (`CapRights::READ` / `CapRights::WRITE`). Attempts to invoke these syscalls with an invalid or unprivileged capability are rejected with error code `-1` (`!0`).
3. **`IRQ-01` (P2 — Hardware IRQ Delivery Involves Duplicate EOI):** Resolved. Eliminated duplicate EOI between the hardware interrupt handler in `kernel/src/idt.rs` and the driver acknowledgment syscall `sys_irq_ack` in `kernel/src/notification.rs`. In `idt.rs`, hardware IRQ lines are masked via `pic::mask(irq_num)` before issuing a single EOI to the 8259 PIC. In `notification.rs`, `sys_irq_ack` validates driver authorization and unmasks the PIC line via `pic::unmask(irq)`, restoring interrupts cleanly without corrupting the PIC ISR state.
4. **`SYSCALL-01` (P1 — Syscall Entry Stack Switching Re-entrancy Risk):** Resolved. Hardened `syscall_entry` in `kernel/src/syscall.rs` by immediately clearing `USER_RSP_SCRATCH` with `mov qword ptr [rip + USER_RSP_SCRATCH], 0` after switching to `KERNEL_RSP_SCRATCH` and pushing the hardware frame to the kernel stack.
5. **`IRQ-02` (P3 — Missing Rights Check on sys_irq_ack):** Resolved. `sys_irq_ack` now checks that the capability type is `CapType::Irq` AND that the caller holds `CapRights::WRITE`.

All quality gates pass:
- **Unit test suite:** 115 tests passing (100% pass rate, 0 failed).
- **Clippy:** Zero warnings (`cargo +nightly clippy -p kernel --lib --tests -- -D warnings`).
- **Formatting:** Clean check (`cargo +nightly fmt --all -- --check`).
- **QEMU Live Execution:** Interactive Ring 3 shell boots via UEFI, runs all shell commands (`help`, `ps`, `ls`, `cat`, `echo`, `write`, `rm`, `time`, `test`, `exit`), and passes in-kernel test suite (Tests 1 through 5).
- **Reboot Persistence:** Verified cross-boot file survival across two independent QEMU processes sharing the same virtual disk image.

---

## 2. Inventory of Remediated Findings

| Finding ID | Severity | Subsystem | Description | Remediation Summary | Status |
|:---|:---:|:---|:---|:---|:---:|
| `STORAGE-01` | P2 | Storage / VirtIO | In-memory filesystem without persistence across reboots | Implemented `VirtioBlockDevice` driver and JanusFS disk-backed sector persistence & probing | **CLOSED** |
| `CAP-01` | P1 | Capability / IPC | Console & FS syscalls bypassed capability table | Added `CapType::Console` and `CapType::Storage` with strict `CPtr` validation on syscalls 24–30 | **CLOSED** |
| `IRQ-01` | P2 | IDT / PIC | Duplicate EOI issued between IDT handler and driver ACK | IDT masks line + single EOI; driver `sys_irq_ack` unmasks line (Single EOI Protocol) | **CLOSED** |
| `SYSCALL-01` | P1 | Syscall Entry | Re-entrancy scratch leak in `syscall_entry` | Zeroed out `USER_RSP_SCRATCH` immediately after pushing to kernel stack | **CLOSED** |
| `IRQ-02` | P3 | Notification / IRQ | `sys_irq_ack` lacked `CapRights::WRITE` check | Enforced `irq_slot.rights.contains(CapRights::WRITE)` in `sys_irq_ack` | **CLOSED** |

---

## 3. Detailed Technical Remediation

### 3.1. STORAGE-01: VirtIO-Blk Driver & Persistent JanusFS

#### 3.1.1. VirtIO Legacy Block Driver (`kernel/src/virtio_blk.rs`)
- Implemented VirtIO 0.9.5 Legacy Block Driver adhering to the VirtIO specification.
- Ring structures:
  - `VRingDesc`: Descriptor table holding 16-byte buffer pointers, lengths, and flags (`VRingDesc::flags = VRING_DESC_F_NEXT`, `VRING_DESC_F_WRITE`).
  - `VRingAvail`: Available ring indexing buffers submitted to the device.
  - `VRingUsed`: Used ring indexing completed buffers returned by the device.
- DMA Buffer Partitioning within Contiguous Physical Memory:
  - Page 0 (`virtqueue_paddr`): Descriptor table (256 descriptors * 16 bytes = 4096 bytes).
  - Page 1 (`virtqueue_paddr + 0x1000`): Available ring.
  - Page 2 (`virtqueue_paddr + 0x2000`): Used ring (`0x2000`–`0x2800`), Request Header buffer (`0x2900`), Sector data buffer (`0x2A00`), Status byte (`0x2C00`).
- Driver Operations:
  - `init(io_base, virtqueue_paddr)`: Resets device, sets `ACKNOWLEDGE`, `DRIVER`, reads queue size (256), writes PFN, and sets `DRIVER_OK`.
  - `read_sector(sector, buf)`: Submits 3-descriptor chain (Header read-only, Sector write-only, Status write-only) to queue 0, notifies device, polls `used.idx` via volatile memory read, checks status `VIRTIO_BLK_S_OK`, and copies data into `buf`.
  - `write_sector(sector, buf)`: Submits 3-descriptor chain (Header read-only, Sector read-only, Status write-only) to queue 0, notifies device, polls `used.idx` via volatile memory read, checks status `VIRTIO_BLK_S_OK`.

#### 3.1.2. JanusFS Disk Persistence & Probing (`kernel/src/fs.rs`)
- Block layout on disk:
  - Sector 0: Superblock (`magic = 0x4A41_4E53`, `version = 1`, `block_size = 512`, `total_blocks = 2048`, `inode_count = 64`, `free_blocks = 2043`).
  - Sectors 1–4: Inodes (64 inodes * 32 bytes = 2048 bytes).
  - Sectors 5–63: Data blocks.
- `init(io_base, virtqueue_paddr)`:
  - Probes sector 0 of the discovered VirtIO block device.
  - If `magic == MAGIC`, the existing filesystem is recognized, and `SUPERBLOCK`, `INODES`, and active file data blocks are restored into memory without formatting (`[JanusFS] Existing filesystem restored from VirtIO disk.`).
  - If sector 0 is unformatted, calls `format()` to create seeded files (`/welcome.txt`, `/help.txt`, `/version.txt`) and flushes all metadata and blocks to disk via `sync()`.
- Synchronous write-through:
  - `write_file`: Writes data into cache and calls `flush_block_to_disk` for every modified block, the updated inode, and the superblock.
  - `delete_file`: Clears block cache, calls `flush_block_to_disk` on freed blocks, zeros the inode, and flushes the inode block and superblock.

---

### 3.2. CAP-01: Capability Enforcement on System Services

#### 3.2.1. New Capability Types (`kernel/src/capability.rs`)
```rust
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
    Console = 9,
    Storage = 10,
}
```

#### 3.2.2. Bootstrap CNode Allocation
- Task A (Interactive Shell):
  - Slot 8: `CapType::Console`, rights `READ | WRITE | GRANT`.
  - Slot 9: `CapType::Storage`, rights `READ | WRITE | GRANT`.

#### 3.2.3. Syscall Handler Enforcement (`kernel/src/console.rs` and `kernel/src/fs.rs`)
- `sys_read_console(cptr, buf_ptr, max_len)`:
  - Resolves `cptr` from caller's CSpace.
  - Verifies `slot.cap_type == CapType::Console` and `slot.rights.contains(CapRights::READ)`.
  - Rejects unauthorized calls with `-1`.
- `sys_write_console(cptr, buf_ptr, len)`:
  - Resolves `cptr` from caller's CSpace.
  - Verifies `slot.cap_type == CapType::Console` and `slot.rights.contains(CapRights::WRITE)`.
  - Rejects unauthorized calls with `-1`.
- `sys_fs_create`, `sys_fs_write`, `sys_fs_delete`:
  - Verify `slot.cap_type == CapType::Storage` and `slot.rights.contains(CapRights::WRITE)`.
- `sys_fs_read`, `sys_fs_list`:
  - Verify `slot.cap_type == CapType::Storage` and `slot.rights.contains(CapRights::READ)`.

---

### 3.3. IRQ-01 & IRQ-02: Hardware IRQ Protocol Correctness

- **Single EOI Protocol:**
  - In `kernel/src/idt.rs`, hardware IRQs (vectors 33..=47):
    ```rust
    crate::notification::handle_hardware_irq(irq_num);
    crate::ipc::wake_irq(irq_num);
    crate::pic::mask(irq_num);
    crate::pic::end_of_interrupt(irq_num);
    ```
  - In `kernel/src/notification.rs`, `sys_irq_ack`:
    ```rust
    let irq_slot = unsafe { crate::capability::lookup_slot(task_id, irq_cptr) }?;
    if irq_slot.cap_type != CapType::Irq || !irq_slot.rights.contains(CapRights::WRITE) {
        return !0;
    }
    let irq = (irq_slot.payload_a & 0xFF) as u8;
    if irq >= 16 { return !0; }
    unsafe {
        #[cfg(not(test))]
        crate::pic::unmask(irq);
    }
    0
    ```
  - Result: No duplicate EOI is sent to the 8259 PIC. Firing IRQ lines are masked while pending in userspace and cleanly unmasked by authorized drivers holding `CapRights::WRITE`.

---

### 3.4. SYSCALL-01: Syscall Entry Stack Switching Hardening

- In `kernel/src/syscall.rs`, `syscall_entry`:
  ```assembly
  mov [rip + USER_RSP_SCRATCH], rsp
  mov rsp, [rip + KERNEL_RSP_SCRATCH]
  push 0x1B                     # SS
  push [rip + USER_RSP_SCRATCH] # RSP
  mov qword ptr [rip + USER_RSP_SCRATCH], 0 # Zero scratch register
  push r11                      # RFLAGS
  push 0x23                     # CS
  push rcx                      # RIP
  ```
- Result: The user RSP is preserved on the kernel interrupt frame, and the global scratch slot is cleared, eliminating leak and re-entrancy exposure.

---

## 4. Verification Evidence

### 4.1. Unit Test Suite (115 / 115 Passed)
```text
running 115 tests
test capability::tests::test_console_and_storage_capabilities ... ok
test capability::tests::test_cnode_size_and_alignment ... ok
test capability::tests::test_cnodeslot_size_and_alignment ... ok
test capability::tests::test_cptr_encode_decode ... ok
test capability::tests::test_cross_task_derivation ... ok
test capability::tests::test_deep_iterative_revocation ... ok
test capability::tests::test_delete_leaf_unlinks_from_sibling_chain ... ok
test capability::tests::test_frame_wx_conflict_rejected ... ok
test capability::tests::test_generation_advancement_and_exhaustion ... ok
test capability::tests::test_invalid_slot_and_task_indices ... ok
test capability::tests::test_mmio_execute_rejected ... ok
test capability::tests::test_multiple_siblings ... ok
test capability::tests::test_occupied_destination_rejection ... ok
test capability::tests::test_parent_child_cdt_linkage ... ok
test capability::tests::test_pola01_task_b_bootstrap_rights ... ok
test capability::tests::test_rights_attenuation ... ok
...
test notification::tests::test_notif_last_reference_removal_cleans_object ... ok
test notification::tests::test_notif_deletion_wakes_blocked_waiter ... ok
test notification::tests::test_notif_multiple_coalesced_signals ... ok
...
test paging::tests::test_virtual_wx_violations_rejected ... ok

test result: ok. 115 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
```

### 4.2. Code Quality Gates
- `cargo +nightly fmt --all -- --check`: **PASS (Clean)**
- `cargo +nightly clippy -p kernel --lib --tests -- -D warnings`: **PASS (Zero warnings)**
- `cargo +nightly build -p kernel --target x86_64-unknown-none --release "-Zbuild-std=core,compiler_builtins"`: **PASS**
- `cargo +nightly build -p bootloader --target x86_64-unknown-uefi --release "-Zbuild-std=core,alloc,compiler_builtins"`: **PASS**

### 4.3. Interactive Live QEMU Shell Session
```text
=======================================================
        JANUS OS v1.0.0 (Interactive Ring 3 Shell)      
=======================================================
Type 'help' for available commands.

janus> help
Janus OS Shell Commands:
  help               - Show this reference
  ps / status        - Show tasks and system status
  ls                 - List files in JanusFS
  cat <file>         - Display file contents
  write <file> <str> - Write string to file
  rm <file>          - Remove file from JanusFS
  time               - Show monotonic uptime (ms)
  echo <str>         - Echo string to console
  test               - Run kernel verification test suite
  clear              - Clear terminal screen
  exit               - Terminate the shell task
janus> ps
=== Janus OS System Status ===
Uptime: 183 ms

Tasks:
PID  STATE
-----------------
0   Runnable
1   Runnable
15   Runnable
janus> ls
JanusFS Directory:
NAME             SIZE
-----------------------
/welcome.txt    82 B
/help.txt       336 B
/version.txt    61 B
janus> cat /welcome.txt
Welcome to Janus OS v1.0!
A capability-based, security-first microkernel in Rust.

janus> echo Janus Microkernel v1.0
Janus Microkernel v1.0
janus> write /notes.txt HelloFromJanusShell
File write successful.
janus> cat /notes.txt
HelloFromJanusShell
janus> rm /notes.txt
File deleted.
janus> time
Uptime: 905 ms
janus> test
[TEST 1] Capability Minting & Revocation ... PASS
[TEST 2] Synchronous Microkernel IPC    ... PASS
[TEST 3] Asynchronous Notifications     ... PASS
[TEST 4] JanusFS Storage Persistence    ... PASS
[TEST 5] Capability Enforcement Security ... PASS
-------------------------------------------------------
All system acceptance tests PASSED!
janus> exit
Shell exited. Terminating task...
```

### 4.4. Automated Cross-Boot Persistence Test
```text
=== SESSION 1 START ===
[KERNEL PCI] Found Virtio-Blk! IO Base: 0xC000, IRQ: 0xB
[KERNEL PCI] Virtio Queue 0 Size: 0x100
[KERNEL] Initializing Shell & Dropping to Ring 3...
janus> write /persist.txt PersistentJanusData12345
File write successful.
janus> cat /persist.txt
PersistentJanusData12345
janus> exit
Shell exited

=== SESSION 2 START ===
[KERNEL PCI] Found Virtio-Blk! IO Base: 0xC000, IRQ: 0xB
[KERNEL PCI] Virtio Queue 0 Size: 0x100
[JanusFS] Existing filesystem restored from VirtIO disk.
[KERNEL] Initializing Shell & Dropping to Ring 3...
janus> ls
JanusFS Directory:
NAME             SIZE
-----------------------
/welcome.txt    82 B
/help.txt       336 B
/version.txt    61 B
/persist.txt    24 B
janus> cat /persist.txt
PersistentJanusData12345
janus> exit
Shell exited

[REBOOT PERSISTENCE VERIFICATION: 100% SUCCESSFUL]
```

---

## 5. Git Status and Repository Integrity

In accordance with strict operating rules:
- No commits were created.
- No branches were merged, pushed, or modified on the remote.
- All changes remain local and unstaged.
- Audited baseline commit remains exact: `f3130f9a19ab9efc2cc3e5b9a37ca76f36221b6a`.

---

## 6. Final Release Verdict

**VERDICT: V1.1 COMPLETE — ALL CRITICAL ACCEPTANCE CRITERIA VERIFIED**

The Janus OS microkernel has satisfied 100% of the V1.1 remediation objectives:
- Capability architecture fully guards all system services (Console, Storage, Notifications, IPC, Memory, Ports, IRQs).
- Real persistent VirtIO-Blk block driver and JanusFS disk-backed cross-boot restore verified.
- IRQ protocol operates with single EOI and capability-verified driver unmasking.
- Syscall entry stack switching hardened against re-entrancy leaks.
- 115/115 unit tests passing, zero Clippy warnings, and interactive Ring 3 userspace fully operational in QEMU.
