# Janus OS — V1.1 Final Release Evidence Audit

**Audit Date:** 2026-11-27  
**Mode:** STRICTLY READ-ONLY ADVERSARIAL RELEASE AUDIT  
**Repository:** `medhansh5/Janus` (`C:/Users/medha/.gemini/antigravity/scratch/Janus`)  
**Branch:** `main` (tracking `origin/main`)  
**HEAD:** `f3130f9a19ab9efc2cc3e5b9a37ca76f36221b6a`  
**Parent Commit:** `b58d23aebbee2a5ce625cb5b612b42274d5449c4`  

---

## 1. Repository Baseline & Working-Tree State

- **Repository Root:** `C:/Users/medha/.gemini/antigravity/scratch/Janus`
- **Active Branch:** `main`
- **HEAD Commit:** `f3130f9a19ab9efc2cc3e5b9a37ca76f36221b6a`
- **Parent Commit:** `b58d23aebbee2a5ce625cb5b612b42274d5449c4`
- **Remote:** `origin` -> `https://github.com/medhansh5/Janus.git` (fetch/push)
- **Git Safety:** No commits, stages, resets, checkouts, branches, merges, or remote operations were performed. The dirty working tree was preserved intact.
- **Change Since `V1_1_REMEDIATION_REPORT.md`:** No source files were modified after `V1_1_REMEDIATION_REPORT.md` (2026-11-27 10:09:48). Prior audit verification updated `test.img` (1,048,576 bytes) and `docs/architecture/serial_session.log` (4,871 bytes) at 10:28:11–10:28:12. This audit performed its QEMU persistence test in an isolated temporary directory (`%TEMP%\janus_audit_*`) so no existing repository artifacts were overwritten.

### `git status --short`
- **Modified Tracked Files (11):** `bootloader/src/main.rs`, `kernel/src/capability.rs`, `kernel/src/idt.rs`, `kernel/src/ipc.rs`, `kernel/src/lib.rs`, `kernel/src/main.rs`, `kernel/src/paging.rs`, `kernel/src/pci.rs`, `kernel/src/pic.rs`, `kernel/src/syscall.rs`, `kernel/src/task.rs` (`11 files changed, 1266 insertions(+), 239 deletions(-)`).
- **Untracked Entries:** `audit_runtime_test.py`, `docs/architecture/`, `kernel/src/console.rs`, `kernel/src/fs.rs`, `kernel/src/notification.rs`, `kernel/src/virtio_blk.rs`, `run_interactive_test.py`.

---

## 2. Finding-by-Finding Audit Verdict

| Finding ID | Severity | Audit Verdict | Primary Source Evidence |
|:---|:---:|:---|:---|
| **`STORAGE-01`** | P2 | **RESOLVED — independently verified** | `kernel/src/pci.rs:103-114`, `kernel/src/virtio_blk.rs:103-297`, `kernel/src/fs.rs:111-229, 311-383` |
| **`CAP-01`** | P1 | **RESOLVED — independently verified** | `kernel/src/capability.rs:71-72, 880-899`, `kernel/src/ipc.rs:102-108`, `kernel/src/console.rs:163-214`, `kernel/src/fs.rs:461-579` |
| **`IRQ-01`** | P2 | **RESOLVED — independently verified** | `kernel/src/idt.rs:372-394`, `kernel/src/pic.rs:70-120`, `kernel/src/notification.rs:380-404` |
| **`IRQ-02`** | P3 | **RESOLVED — independently verified** | `kernel/src/notification.rs:390` |
| **`SYSCALL-01`** | P1 | **RESOLVED — independently verified** *(under UP invariant)* | `kernel/src/syscall.rs:35, 38-114`, `kernel/src/task.rs:238-243` |

---

## 3. Technical Evidence by Subsystem

### 3.1. `STORAGE-01` — VirtIO-Blk Driver & Persistent JanusFS
- **PCI Discovery (`kernel/src/pci.rs:103-114`):** Locates vendor `0x1AF4`, device `0x1001`, extracts `io_base = (bar0 & !0x3) as u16`, reads IRQ from `0x3C`, and enables PCI Bus Mastering (`1 << 2`) and I/O Space (`1 << 0`) in the Command register (`0x04`).
- **Legacy VirtIO Init (`kernel/src/virtio_blk.rs:103-151`):** Performs Reset (`0`), `ACKNOWLEDGE` (`1`), `DRIVER` (`3`), reads host features and writes `0` guest features, selects Queue 0, checks `q_size == 0` -> `VIRTIO_STATUS_FAILED` + `Err(())`, zeroes 12,288 bytes (3 contiguous 4 KiB frames allocated via `allocator.allocate_contiguous_frames(3)` in `kernel/src/main.rs:270`), writes `virtqueue_paddr >> 12` to `VIRTIO_PCI_QUEUE_PFN`, and sets `DRIVER_OK` (`7`).
- **VirtQueue & DMA Layout (`kernel/src/virtio_blk.rs:165-225, 236-293`):**
  - Page 0 (`+0x0000`): 256 `VRingDesc` entries (4,096 B).
  - Page 1 (`+0x1000`): `VRingAvail` (516 B).
  - Page 2 (`+0x2000`): `VRingUsed` (2,052 B, ends at `+0x2804`), followed by non-overlapping DMA buffers: `VirtioBlkOutHdr` at `+0x2900`, 512-byte sector buffer at `+0x2A00`, and 1-byte status at `+0x2C00`.
  - Uses physical addresses for DMA descriptors, `fence(Ordering::SeqCst)` around `avail.idx` updates, `outw(QUEUE_NOTIFY, 0)`, `core::ptr::read_volatile` on `used.idx` with a bounded spin limit (`spins > 2_000_000`), and `read_volatile(status_ptr) == VIRTIO_BLK_S_OK`.
- **Filesystem Persistence (`kernel/src/fs.rs:111-229, 311-383`):**
  - `fs::init()` reads sector 0; if `MAGIC == 0x4A41_4E53`, restores `SUPERBLOCK`, `INODES` (sectors 1..4), and active `direct_blocks` (`5..63`) from disk, logging `[JanusFS] Existing filesystem restored from VirtIO disk.`. Otherwise formats and calls `sync()`.
  - `write_file()` and `delete_file()` flush modified data blocks, the affected inode sector, and the superblock to VirtIO sectors.

### 3.2. `CAP-01` — Capability Enforcement on Syscalls 24–30
- **Dispatch (`kernel/src/ipc.rs:102-108`):** All syscalls 24–30 (`SYS_READ_CONSOLE`, `SYS_WRITE_CONSOLE`, `SYS_FS_CREATE`, `SYS_FS_READ`, `SYS_FS_WRITE`, `SYS_FS_DELETE`, `SYS_FS_LIST`) pass `CPtr(arg1)` from `rsi`. No alternate console or filesystem syscall bypasses these checks.
- **Validation (`kernel/src/console.rs:163-176`, `kernel/src/fs.rs:461-474`):** Both `check_console_cap` and `check_storage_cap` execute `capability::lookup_slot(task_id, cptr)` (enforcing `cptr.is_canonical()`, `slot < 128`, non-empty slot, and `slot.generation == cptr.generation()`) and check `CapType::Console`/`CapType::Storage` plus `CapRights::READ` (syscalls 24, 27, 30) or `CapRights::WRITE` (syscalls 25, 26, 28, 29) **before** any user-pointer copy or state mutation.
- **User Copy Safety (`kernel/src/paging.rs:381-531`):** `copy_from_user` and `copy_to_user` pre-validate null pointers, arithmetic overflow, kernel-space bounds, non-canonical addresses, page presence, and `USER_ACCESSIBLE` flags across all spanned pages.
- **Least Privilege Bootstrap:** Task A (`kernel/src/capability.rs:880-899`) receives `Console` (slot 8) and `Storage` (slot 9). Task B (`kernel/src/capability.rs:902-969`) receives neither.
- **Denied-Caller Verification:** Live QEMU `test` Test 5 (`kernel/src/main.rs:956-965`) invokes `SYS_WRITE_CONSOLE` with invalid `CPtr(0x00019999)` and verifies rejection (`rax == -1`).

### 3.3. `IRQ-01` and `IRQ-02` — PIC/IDT Protocol
- **Spurious & Real IRQ Handling (`kernel/src/idt.rs:372-394`, `kernel/src/pic.rs:70-120`):**
  - Spurious IRQ7 (`(read_isr(false) & 0x80) == 0`) returns with **zero** EOIs; spurious IRQ15 (`(read_isr(true) & 0x80) == 0`) sends EOI only to master (`end_of_interrupt(7)`) and returns.
  - Real IRQs (vectors 33..=47) deliver notifications (`handle_hardware_irq` / `wake_irq`), call `pic::mask(irq_num)`, and issue **one** EOI (`pic::end_of_interrupt(irq_num)`, sending slave then master if `irq >= 8`).
- **Acknowledgment (`kernel/src/notification.rs:380-404`):** `sys_irq_ack` validates `irq_slot.cap_type == CapType::Irq && irq_slot.rights.contains(CapRights::WRITE)` (`IRQ-02`), bounds-checks `irq < 16`, and calls `pic::unmask(irq)` with no duplicate EOI.
- **Notification Delivery (`kernel/src/notification.rs:410-430`):** Coalesces `irq_bit` into `pending_bits` and atomically transfers/clears `pending_bits` when waking a single `waiting_task`.

### 3.4. `SYSCALL-01` — Syscall Entry Stack & Scratch Assumptions
- **Mechanism (`kernel/src/syscall.rs:35, 38-114`):** `SFMask::write(RFlags::INTERRUPT_FLAG)` clears `IF` on `syscall` entry. `syscall_entry` saves `rsp` to `USER_RSP_SCRATCH`, loads `KERNEL_RSP_SCRATCH` (updated on task switch in `task.rs:242` alongside TSS `RSP0`), pushes `0x1B` and `[rip + USER_RSP_SCRATCH]`, and zeroes `USER_RSP_SCRATCH` before pushing the remainder of the frame. Return pops GPRs, skips saved `rax`, and executes `iretq`.
- **Explicit Architectural Classification:**
  - **Safe under current uniprocessor assumptions:** **YES** (`IF=0` via `SFMask` prevents maskable interrupts and task switches during the 4-instruction scratch window).
  - **Reentrant under all circumstances:** **NO** (`USER_RSP_SCRATCH` and `KERNEL_RSP_SCRATCH` are global `static mut` slots; an NMI or fault inside the entry window before line 48 would overwrite `USER_RSP_SCRATCH`).
  - **SMP-safe:** **NO** (single global scratch slots cannot support concurrent execution on multiple cores).

---

## 4. Reproduced Quality Gates, Builds & ELF Verification

| Check | Command | Exit Code | Result |
|:---|:---|:---:|:---|
| Formatting | `cargo +nightly fmt --all -- --check` | `0` | Clean |
| Host Unit Tests | `cargo +nightly test -p kernel --lib` | `0` | **115 passed, 0 failed, 0 ignored** |
| Clippy Lints | `cargo +nightly clippy -p kernel --lib --tests -- -D warnings` | `0` | Clean (0 warnings) |
| Kernel Release Build | `cargo +nightly build -p kernel --target x86_64-unknown-none --release "-Zbuild-std=core,compiler_builtins"` | `0` | `154,424` bytes (`kernel`) |
| Bootloader Release Build | `cargo +nightly build -p bootloader --target x86_64-unknown-uefi --release "-Zbuild-std=core,alloc,compiler_builtins"` | `0` | `bootloader.efi` built |

- **Kernel ELF Verification (`target/x86_64-unknown-none/release/kernel` & `esp/kernel.elf`):**
  - Identical SHA-256: `080a57643efe72e3c9d199fd89e5261301844fffdf82887452f4d717ce24334b`
  - Entry point: `0xffffffff80001fc0`
  - `PT_LOAD` segments strictly W^X: `[0] RX`, `[1] R`, `[2] RW`, `[3] RW`, `PT_GNU_STACK RW` (non-executable).

---

## 5. Live QEMU Runtime & Cross-Boot Persistence Evidence

### 5.1. Existing Interactive Suite (`run_interactive_test.py` -> `docs/architecture/serial_session.log`)
- **QEMU Command:** `qemu-system-x86_64.exe -drive if=pflash,format=raw,readonly=on,file=...\edk2-x86_64-code.fd -drive format=raw,file=fat:rw:esp -drive file=test.img,format=raw,if=virtio -serial tcp:127.0.0.1:4444,server,nowait -display none -no-reboot`
- **Verified Excerpt (`docs/architecture/serial_session.log:47-50, 102-110`):**
  ```text
  [KERNEL PCI] Found Virtio-Blk! IO Base: 0xC000, IRQ: 0xB
  [KERNEL PCI] Virtio Queue 0 Size: 0x100
  [JanusFS] Existing filesystem restored from VirtIO disk.
  ...
  [TEST 1] Capability Minting & Revocation ... PASS
  [TEST 2] Synchronous Microkernel IPC    ... PASS
  [TEST 3] Asynchronous Notifications     ... PASS
  [TEST 4] JanusFS Storage Persistence    ... PASS
  [TEST 5] Capability Enforcement Security ... PASS
  -------------------------------------------------------
  All system acceptance tests PASSED!
  ```
  All 11 shell commands executed cleanly (`exit` code `0`).

### 5.2. Isolated 3-Boot Cross-Reboot Persistence Verification (`%TEMP%\janus_audit_*`)
- **Disk Image:** Temporary 1,048,576-byte raw VirtIO image (`audit_persist.img`), reused across 3 independent QEMU boots:
  - **Initial SHA-256:** `30e14955ebf1352266dc2ff8067e68104607e750abb9d3b36582b8af909fcb58`
  - **Boot 1:** Formatted fresh disk, ran `write /audit.txt AuditPersist987`, verified `cat /audit.txt`, wrote `/delme.txt`, ran `test`, exited cleanly (`SHA-256` -> `096a77ce484e0968685b6fa0eeec845c92678d7e9b25a1160e7860cd50c0332e`).
  - **Boot 2:** Restored from disk (`[JanusFS] Existing filesystem restored from VirtIO disk.`), verified `ls` & `cat /audit.txt` (`AuditPersist987`), ran `rm /delme.txt`, exited cleanly (`SHA-256` -> `cb1d00929c86cf5b058bd474b8e80836ec58266321d741e48ac9c786b1e8b4f2`).
  - **Boot 3:** Restored from disk, verified `cat /delme.txt` -> `cat: file not found` and `cat /audit.txt` -> `AuditPersist987`.
- **Conclusion:** Cross-boot creation, content retention, and deletion persistence are genuinely verified against VirtIO disk sectors.

---

## 6. Remaining Limitations & Newly Observed Non-Blocking Items

1. **Uniprocessor-Only Syscall Scratch (`SYSCALL-01`):** Safe on UP with `IF=0`, not SMP-safe and not NMI-reentrant.
2. **Synchronous VirtIO Polling:** `read_sector`/`write_sector` poll `used.idx` synchronously rather than using IRQ-driven completion; hardware IRQ11 delivery is not exercised by the VirtIO driver at runtime (only unit-tested in `notification::tests::test_irq_bind_and_ack`).
3. **PCI BAR / Queue-Size Hardcoding (Low):** `pci::discover_virtio()` (`pci.rs:105-106`) masks `bar0 & !0x3` without validating bit 0 (I/O space) and panics if VirtIO-Blk is absent; `virtio_blk.rs:125-130` checks `q_size == 0` but does not reject `q_size > 256`.
4. **FS Partial-Allocation & Corrupt-Disk Resilience (Low):** `fs::write_file()` does not roll back partial `direct_blocks` if `allocate_data_block()` runs out of space mid-file; no crash-consistency journal exists.
5. **IRQ Mask Cleanup on Task Exit (Low):** `capability::destroy_task_cspace()` / `cleanup_slot()` does not unmask a PIC line if a driver task exits while holding a masked `CapType::Irq`.
6. **Host Unit-Test Gap for Syscalls 24–30 (Low):** Wrong-type, stale-generation, and insufficient-rights rejections on syscalls 24–30 are verified by code inspection and QEMU Test 5 (`SYS_WRITE_CONSOLE` invalid `CPtr`), but lack dedicated host unit tests.

---

## 7. Final Verdict

**V1.1 RELEASE-READY — ALL CRITICAL EVIDENCE VERIFIED**
