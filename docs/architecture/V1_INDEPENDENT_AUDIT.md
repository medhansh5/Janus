# Janus OS — Usable V1 Independent Engineering Audit Report
## Post-Implementation Verification, Security Review & Release Readiness

**Audit Date:** 2026-11-27  
**Auditor Mode:** Strictly Read-Only Independent Adversarial Source & Runtime Audit  
**Repository:** `medhansh5/Janus`  
**Working Directory:** `C:\Users\medha\.gemini\antigravity\scratch\Janus`  
**Current Branch:** `main`  
**Current Baseline HEAD:** `f3130f9a19ab9efc2cc3e5b9a37ca76f36221b6a`  
**Parent Commit:** `b58d23a105063065b796d1947b744dffaa8e7c10`  
**Working Tree State:** Dirty (modified tracked and untracked implementation files, strictly unstaged)  
**Final Release Verdict:** **V1 FUNCTIONALLY USABLE — LIMITATIONS DOCUMENTED**

---

## 1. Executive Summary

An exhaustive, adversarial engineering audit of the Janus OS V1 implementation was conducted across all source code, assembly stubs, linker scripts, build configurations, unit tests, and live UEFI runtime execution in QEMU.

The audit verified that the implementation achieves an impressive architectural milestone: Janus OS boots through UEFI via OVMF, parses and verifies its higher-half 64-bit ELF kernel, enforces W^X page protection, transitions from physical memory to direct higher-half mapping, initializes GDT/TSS/IDT and dual 8259 PICs, configures STAR/LSTAR syscall MSRs, and successfully drops to **Ring 3 user mode** via `iretq`. In Ring 3, the interactive shell (`janus_shell`) runs, accepting keyboard/serial input and executing all 11 user commands (`help`, `ps`, `ls`, `cat`, `write`, `rm`, `time`, `echo`, `test`, `clear`, `exit`). All 114 unit tests pass with zero failures and zero compiler or Clippy warnings.

However, an adversarial inspection of the underlying driver and filesystem implementation revealed critical discrepancies between claimed capabilities and actual physical reality:
1. **No Disk Sector Persistence (VirtIO-Blk Stub):** While PCI discovery correctly identifies the VirtIO-Blk device at I/O base 0xC000 and reads queue 0 size, **no virtqueue descriptors, available rings, or used rings are allocated or polled**. JanusFS operates exclusively as an **in-memory RAM filesystem** (`BLOCK_CACHE`). Files created or written during execution **do not survive a system reboot**.
2. **Ambient Authority in Storage & Console Syscalls:** Syscalls 24–30 (`SYS_READ_CONSOLE`, `SYS_WRITE_CONSOLE`, `SYS_FS_CREATE`, `SYS_FS_READ`, `SYS_FS_WRITE`, `SYS_FS_DELETE`, `SYS_FS_LIST`) take direct raw pointers and lengths with **no capability parameter (`CPtr`)**. They operate via ambient authority rather than capability-based access control.
3. **Double EOI on Hardware IRQs:** Hardware interrupt dispatchers in `kernel/src/idt.rs` send End-of-Interrupt (`crate::pic::end_of_interrupt`) immediately upon vector invocation, while `sys_irq_ack` (Syscall 18) sends an unverified second EOI to the 8259 PIC.
4. **Non-Reentrant Global Syscall Scratch Register:** `syscall_entry` in `kernel/src/syscall.rs` relies on a single global `static mut USER_RSP_SCRATCH` without per-CPU or per-thread stack isolation.

Based on these verified findings, the official audit verdict is **V1 FUNCTIONALLY USABLE — LIMITATIONS DOCUMENTED**.

---

## 2. Repository & Changeset Forensics

### Baseline State
- **Repository:** `medhansh5/Janus`
- **Branch:** `main` (tracking `origin/main`)
- **HEAD:** `f3130f9a19ab9efc2cc3e5b9a37ca76f36221b6a` (`feat(capability): implement secure capability delegation and revocation`)
- **Parent Commit:** `b58d23a105063065b796d1947b744dffaa8e7c10`
- **Working Tree:** 9 tracked files modified, 5 untracked entries. No changes have been staged or committed.

### Full Inventory of Modified and Untracked Files

#### Modified Tracked Files (9 files, +1115 / -234 lines)
1. `bootloader/src/main.rs` (+22 / -6): Resized kernel loading buffer to 1MB to accommodate full kernel ELF; added UEFI filesystem protocol error reporting.
2. `kernel/src/capability.rs` (+79 / -4): Added `install_notification_root()`, payload generation validation, and reference accounting integration.
3. `kernel/src/idt.rs` (+166 / -12): Added assembly stubs and IDT gates for vectors 32..47 (Timer + IRQ 1..15); integrated monotonic tick counter, sleeping task wakeups, console input polling, and spurious IRQ filtering.
4. `kernel/src/ipc.rs` (+146 / -6): Registered syscalls 17–32 (`SYS_IRQ_BIND`, `SYS_IRQ_ACK`, `SYS_SLEEP`, `SYS_EXIT`, `SYS_GET_TIME`, `SYS_READ_CONSOLE`, `SYS_WRITE_CONSOLE`, `SYS_FS_*`, `SYS_TASK_STATUS`).
5. `kernel/src/lib.rs` (+43 / -17): Exported `console` and `fs` modules; updated 16550 UART initialization with 115200 baud divisor, 16-byte FIFO enable (`0xC7`), and RTS/DSR control (`0x0B`).
6. `kernel/src/main.rs` (+685 / -180): Added Ring 3 user page-table mappings (Task A code, Task B code, user stacks, shared buffer); configured Ring 3 `iretq` drop; embedded interactive `janus_shell` assembly payload.
7. `kernel/src/paging.rs` (+70 / -4): Implemented `copy_to_user` and `copy_from_user` with full PML4..PT structure pre-validation, direct physical map limits, non-canonical address checks, and supervisor-flag rejection.
8. `kernel/src/pic.rs` (+23 / -1): Implemented `read_isr(pic2: bool)` and `is_spurious(irq: u8)` with `PIC_READ_ISR = 0x0B`.
9. `kernel/src/task.rs` (+109 / -4): Added `TaskState::BlockedOnSleep { wake_tick }`; added `wake_sleeping_tasks()`, `terminate_current_task()`, `sys_sleep()`, and `get_task_info()`. Removed verbose per-tick context switch debug logging.

#### Untracked Files
1. `kernel/src/console.rs` (253 lines): Circular 512-byte input ring buffer, PS/2 Set 1 scancode translator with Shift state and mouse packet filter (bit 5), COM1 serial receiver with CRLF deduplication, and console syscall handlers.
2. `kernel/src/fs.rs` (479 lines): JanusFS superblock, 64-inode table, 512-byte block cache, seeded files, and filesystem syscall handlers.
3. `kernel/src/notification.rs` (1171 lines): 64-object notification pool, 64-bit coalescing pending event bitmask, single-waiter invariant, badged signal authority, IRQ binding array, and syscall handlers.
4. `run_interactive_test.py` / `audit_runtime_test.py`: Automated QEMU TCP serial validation test harnesses.
5. `docs/architecture/*`: Architecture specifications, review records, and audit reports.

---

## 3. Milestone-by-Milestone Acceptance Matrix (Sections A – M)

| Section | Acceptance Criteria | Verified Implementation Reality | Verdict |
|---|---|---|---|
| **A** | **Boot & Kernel Initialization** | UEFI bootloader reads ELF segments, enforces W^X page flags, transitions to kernel `_start`, initializes GDT, TSS, IDT (256 gates), PIC, and COM1 serial diagnostics. | **VERIFIED** |
| **B** | **Memory Management & Isolation** | Physical bitmap frame allocator; direct physical offset map (`0xFFFF_8000_...`); isolated user PML4s with higher-half kernel sharing; safe `copy_to_user`/`copy_from_user` validating addresses and permissions. | **VERIFIED** |
| **C** | **Process & Task Management** | Preemptive & cooperative round-robin scheduling; PID 15 idle task; task lifecycle states; clean CSpace teardown on `SYS_EXIT`. | **VERIFIED** |
| **D** | **Capability Security** | Guarded CSpace, CDT derivation tree, rights attenuation, 64-bit generation recycling protection, recursive revocation without sibling corruption. | **VERIFIED** |
| **E** | **IPC & Notifications** | Synchronous rendezvous IPC with badge delivery; 64 notification objects with 64-bit coalescing event bitmask; single-waiter invariant; non-blocking poll and blocking wait. | **VERIFIED** |
| **F** | **Hardware Interrupts & IRQ Routing** | Dual 8259 PIC remapped to vectors 32–47; spurious IRQ7/15 handling. `SYS_IRQ_BIND` routes to notifications. *(Note: Double EOI identified).* | **PARTIALLY VERIFIED** |
| **G** | **Timers & Timekeeping** | PIT Channel 0 at 1000Hz (1ms ticks); monotonic tick counter; `SYS_SLEEP` (19) putting tasks into `BlockedOnSleep`; `SYS_GET_TIME` (23) returning milliseconds. | **VERIFIED** |
| **H** | **Device Support** | Serial 115200 8N1 FIFO verified; PS/2 keyboard Set 1 scancode decoder verified. **VirtIO-Blk block transport not implemented** (PCI discovery only). | **PARTIALLY VERIFIED** |
| **I** | **Storage & JanusFS** | Superblock, inodes, directory listing, seeded files, and CRUD syscalls work **in RAM only**. **No block persistence to VirtIO disk across reboots.** | **PARTIALLY VERIFIED** |
| **J** | **User-Space Execution** | Ring 3 transition via `iretq` (`CS=0x1B`, `SS=0x23`); user stacks; stable syscall entry and exit via `STAR`/`LSTAR`/`FMASK` MSRs. | **VERIFIED** |
| **K** | **User-Space System Services** | Console I/O syscalls (24/25), process termination (22), and system status (32) functional. *(Note: Operate via ambient authority).* | **PARTIALLY VERIFIED** |
| **L** | **Interactive Shell** | Ring 3 interactive `janus_shell` executing `help`, `ps`, `ls`, `cat`, `write`, `rm`, `time`, `echo`, `test`, `clear`, `exit`. | **VERIFIED** |
| **M** | **Diagnostics & Quality** | 114 unit tests passing, 0 clippy warnings, reproducible build and runtime verification. | **VERIFIED** |

---

## 4. Independent Build and ELF Verification

### Build Commands & Exit Codes
1. **Bootloader Build:**
   ```powershell
   cargo +nightly build --manifest-path Cargo.toml -p bootloader --target x86_64-unknown-uefi "-Zbuild-std=core,compiler_builtins,alloc"
   ```
   - **Exit Code:** `0` (Success, 0 errors, 0 warnings).
2. **Kernel Build:**
   ```powershell
   cargo +nightly rustc --manifest-path Cargo.toml -p kernel --bin kernel --release --target x86_64-unknown-none "-Zbuild-std=core,compiler_builtins,alloc" -- -C link-arg="-TC:\Users\medha\.gemini\antigravity\scratch\Janus\kernel\linker.ld" -C relocation-model=static
   ```
   - **Exit Code:** `0` (Success, 0 errors, 0 warnings).

### Kernel ELF Header & Segment Inspection
- **ELF Class:** 64-bit (`ELFClass: 2`)
- **Data Encoding:** 2's complement, little-endian (`ELFData: 1`)
- **Machine Architecture:** AMD x86-64 (`0x3E`)
- **Entry Point Address:** `0xFFFFFFFF_80001DE0` (Higher-half virtual address)
- **Program Headers (PT_LOAD):**
  - Segment 0 (`.text`): `VAddr: 0xFFFFFFFF80000000`, `FileSz: 0xBF6E`, `MemSz: 0xBF6E`, `Flags: ER` (**Execute + Read, Non-Writable**).
  - Segment 1 (`.rodata`): `VAddr: 0xFFFFFFFF8000C000`, `FileSz: 0x0C94`, `MemSz: 0x0C94`, `Flags: R` (**Read-Only, Non-Executable, Non-Writable**).
  - Segment 2 (`.data.rel.ro`): `VAddr: 0xFFFFFFFF8000CC98`, `FileSz: 0x0218`, `MemSz: 0x0218`, `Flags: WR` (**Read + Write, Non-Executable**).
  - Segment 3 (`.data` / `.bss`): `VAddr: 0xFFFFFFFF8000D000`, `FileSz: 0x11E80`, `MemSz: 0x22D40`, `Flags: WR` (**Read + Write, Non-Executable**).
  - Segment 6 (`GNU_STACK`): `Flags: WR` (**No Execute on Stack**).

**Finding:** The linker script and ELF segments strictly comply with the **W^X (Write XOR Execute)** security invariant. No segment is both writable and executable.

---

## 5. Memory Management & Isolation Audit

### Verified Strengths
1. **Paging Hierarchy & Higher-Half Offset Map:** Physical memory is mapped at `0xFFFF_8000_0000_0000`. User address spaces are constructed via `create_user_pml4()`, copying only PML4 entries 256..511 (kernel space) and leaving entries 0..255 private to each task.
2. **Safe Boundary Checks in User Copies:** `copy_from_user` and `copy_to_user` (`kernel/src/paging.rs`):
   - Reject null pointers (`vaddr == 0`).
   - Reject kernel-space addresses (`vaddr >= 0x0000_8000_0000_0000`).
   - Reject non-canonical addresses (bits 47..63 must be identical).
   - Reject arithmetic overflow on `vaddr + len`.
   - Traverse PML4 -> PDPT -> PD -> PT and ensure `UserAccessible` (bit 2) is present at every level.
   - Enforce direct-map bounds on physical frames.

### Security Findings
- **P2 — User Copy Fallback in Unit Tests:** In `kernel/src/ipc.rs` lines 120–129 and 138–147, `copy_user_slice_from` and `copy_user_slice_to` use `#[cfg(test)]` blocks that perform raw `core::ptr::copy` without walking page tables. While disabled in bare-metal production builds, test-harness bypasses can mask page permission bugs in host tests.

---

## 6. Task Management & Scheduler Audit

### Preemption Verification
- The PIT fires at 1000Hz (vector 32). In `exception_handler` (`kernel/src/idt.rs`), the timer interrupt increments `TICK_COUNT`, calls `wake_sleeping_tasks()`, and calls `schedule()`.
- Preemption was verified live: tasks yield CPU slices without explicit cooperative syscalls, and sleeping tasks accurately awaken after their millisecond duration expires.

### Scheduler Context Switch Forensics
- Context switching is performed in assembly (`switch_context`, `kernel/src/task.rs:264`):
  - Callee-saved registers (`r15`, `r14`, `r13`, `r12`, `rbx`, `rbp`, `rsi`, `rdi`) and `rsp` are saved to `prev.context`.
  - Next task's registers and `rsp` are loaded from `next.context`.
  - The return address is popped via `ret`, returning into whichever kernel routine originally yielded or received the interrupt.
  - Page table root (`CR3`) is updated.
  - TSS `RSP0` and `KERNEL_RSP_SCRATCH` are updated to point to `next.kernel_stack_top`.

### Security Findings
- **P1 — Non-Reentrant Syscall Scratch Registers (`kernel/src/syscall.rs:111-113`):**
  ```rust
  pub static mut USER_RSP_SCRATCH: u64 = 0;
  pub static mut KERNEL_RSP_SCRATCH: u64 = 0;
  ```
  `syscall_entry` saves user RSP to `USER_RSP_SCRATCH` and loads kernel stack from `KERNEL_RSP_SCRATCH`. If an interrupt or nested fault occurs before RSP is safely pushed onto the kernel stack, or if multicore (SMP) is introduced, this shared mutable global will cause stack corruption. In production x86_64 OSes, `SWAPGS` is used to load per-CPU scratch areas.

---

## 7. Capability Security Audit

### Verified Protections
1. **CPtr Canonical Formatting:** CPtrs must have bits 63..32 as zero. Reserved bits are strictly checked.
2. **Generation Counter Advancement:** When a capability is deleted or revoked, the object generation advances. Any attempt to use an old handle returns `CapError::InvalidGeneration`.
3. **CDT Linkage:** `revoke` walks the derivation tree iteratively or recursively, unlinking all descendants without corrupting sibling chains. Occupied destination slots are rejected during minting/copying.
4. **Rights Attenuation:** A minted or copied capability can only attenuate rights; any attempt to grant rights not held by the parent is rejected with `CapError::RightsEscalation`.

### Security Findings
- **P1 — Ambient Authority in Filesystem & Console Syscalls:**
  - Filesystem syscalls (`SYS_FS_CREATE`, `SYS_FS_READ`, `SYS_FS_WRITE`, `SYS_FS_DELETE`, `SYS_FS_LIST`) and console syscalls (`SYS_READ_CONSOLE`, `SYS_WRITE_CONSOLE`) do NOT take a `CPtr` parameter.
  - Any user-space task in Ring 3 can manipulate any file or read/write console buffers without possessing a capability. This violates the capability-based security model.

---

## 8. IPC, Notifications & Hardware Interrupts Audit

### Verified Implementation
- Synchronous rendezvous transfers IPC messages between sender and receiver contexts with badge injection.
- Notification objects maintain a 64-bit event bitmask. Multiple signals coalesce via bitwise OR.
- Single-waiter invariant is strictly enforced: if a second task calls `sys_notify_wait` while a waiter is registered, the call is rejected immediately.
- Destroying or revoking a notification object wakes any blocked waiter with error sentinel `!0` to prevent orphaned blocked tasks.

### Security Findings
- **P2 — Double End-Of-Interrupt (EOI) on Hardware IRQs (`kernel/src/idt.rs:391`, `kernel/src/notification.rs:400`):**
  - In `idt.rs:391`, the IDT gate for hardware IRQs (vectors 33..47) executes `crate::pic::end_of_interrupt(irq_num)` immediately before returning from interrupt context.
  - In `notification.rs:400`, `sys_irq_ack(irq_cptr)` also executes `crate::pic::end_of_interrupt(irq)`.
  - When a user-level driver calls `SYS_IRQ_ACK`, a second EOI is sent to the 8259 PIC when no ISR bit is active. In cascaded 8259 PIC configurations, redundant EOIs can erroneously clear in-service bits of lower-priority interrupts.
- **P3 — Unchecked Rights in `sys_irq_ack` (`kernel/src/notification.rs:388-392`):**
  - `sys_irq_ack` verifies `irq_slot.cap_type == CapType::Irq`, but fails to check whether `irq_slot.rights` contains `CapRights::WRITE`. Any task possessing an Irq capability with empty rights can send EOIs.

---

## 9. Storage, VirtIO-Blk & Filesystem Persistence Audit

### Adversarial Runtime Experiment
To test whether JanusFS data survives reboots, an automated test harness (`audit_runtime_test.py`) was executed across two sequential QEMU sessions using the dedicated disk image `test_audit.img`:

#### Session 1 (Initial Boot):
1. Shell booted.
2. Verified initial files: `/welcome.txt` (82 B), `/help.txt` (336 B), `/version.txt` (61 B).
3. Created file: `write /persist.txt PersistenceValue123`.
4. Read file: `cat /persist.txt` returned `PersistenceValue123`.
5. Created temporary file: `write /temp.txt TempData`.
6. Deleted temporary file: `rm /temp.txt`.
7. Verified deletion: `cat /temp.txt` returned `cat: file not found`.
8. Clean shutdown: `exit`.

#### Session 2 (Full Reboot on Same Disk):
1. QEMU restarted afresh with `test_audit.img`.
2. Executed `ls`: Directory listed only `/welcome.txt`, `/help.txt`, `/version.txt`.
3. Executed `cat /persist.txt`: Returned **`cat: file not found`**.

### Root Cause Analysis in Source Code
- In `kernel/src/pci.rs:89-100`, `discover_virtio()` detects the VirtIO-Blk PCI device, reads BAR0, enables Bus Mastering and I/O space in the PCI Command register, and returns `(io_base, irq)`.
- In `kernel/src/main.rs:177`, `fs::init(Some(virtio_io))` passes `io_base` to `fs.rs`.
- In `kernel/src/fs.rs:100`, `static mut VIRTIO_IO_BASE: Option<u16>` stores the base port.
- **`VIRTIO_IO_BASE` is never read or referenced again in `fs.rs`.**
- `fs.rs` reads and writes exclusively to an in-memory array: `static mut BLOCK_CACHE: [[u8; BLOCK_SIZE]; CACHE_BLOCKS]`.
- On every kernel boot, `format()` reinitializes `BLOCK_CACHE` and `INODES` in RAM, overwriting any previous state.
- **Conclusion:** VirtIO-Blk request queues are not implemented. JanusFS is currently an **in-memory RAM filesystem**.

---

## 10. Interactive Ring 3 Shell Audit

### Shell Payload & Execution
- In `kernel/src/main.rs:575-1057`, `task_a_payload` contains the assembly implementation of `janus_shell`.
- It executes entirely in Ring 3:
  - Line reading uses `SYS_READ_CONSOLE` (24) and `SYS_SLEEP` (20ms) polling.
  - Echoing and printing use `SYS_WRITE_CONSOLE` (25).
  - Parsing extracts arguments and invokes appropriate syscalls:
    - `help`: prints built-in documentation.
    - `ps` / `status`: invokes `SYS_TASK_STATUS` (32).
    - `ls`: invokes `SYS_FS_LIST` (30).
    - `cat`: invokes `SYS_FS_READ` (27).
    - `write`: invokes `SYS_FS_WRITE` (28).
    - `rm`: invokes `SYS_FS_DELETE` (29).
    - `time`: invokes `SYS_GET_TIME` (23).
    - `echo`: prints string argument.
    - `test`: invokes capability, IPC, notification, and filesystem test sequence.
    - `clear`: outputs ANSI escape sequence `\x1b[2J\x1b[H`.
    - `exit`: invokes `SYS_EXIT` (22) to terminate task.

### Error Handling Verification
- Empty input (newline only): Shell cleanly prompts `janus> ` again without crashing.
- Unknown command: Correctly reports `Unknown command. Type 'help' for command reference.`.
- Nonexistent file `cat /nonexistent.txt`: Correctly reports `cat: file not found`.
- Nonexistent file `rm /nonexistent.txt`: Correctly reports `rm: file not found or deletion failed`.

---

## 11. Test Quality & Classification (114 Unit Tests)

All 114 tests in `cargo +nightly test -p kernel --lib` were analyzed and categorized:

| Category | Count | Subsystems Covered | Quality Assessment |
|---|---|---|---|
| **Pure Unit Tests** | 35 | CNode layout, CPtr encoding, PS/2 scancode translation, generation exhaustion, 64-bit badge permissions, event bitmask encoding. | **High:** Exercises edge cases, bit bounds, arithmetic, and data-structure invariants. |
| **Kernel Helper Tests** | 18 | `format_u32`, console ring buffer FIFO wrap-around, inode name matching, block allocation, user slice copy validation. | **Medium:** Verifies helper algorithms in isolation. |
| **Syscall Path Tests** | 24 | `sys_notify_create`, `sys_notify_signal`, `sys_notify_poll`, `sys_irq_bind`, `sys_cap_mint`, `sys_cap_revoke`. | **High:** Verifies rights checking, capability lookup failures, and error codes. |
| **Hardware Abstraction Tests** | 22 | Paging permissions, huge-page PAT flags, direct map boundaries, PML4 destruction defenses. | **High:** Rigorous memory safety assertions. |
| **Integration Tests** | 15 | Multi-step capability derivation cascades, JanusFS format & CRUD in RAM, notification wait/signal rendezvous. | **Medium:** Verifies component workflows within a single address space. |
| **Runtime Acceptance Tests** | 0 (in lib) | Verified externally via automated QEMU harness (`audit_runtime_test.py`). | **High:** Validates true bare-metal boot, Ring 3 execution, and serial interactions. |

### Test Suite Limitations
1. **Missing Task Scheduler Tests:** `task.rs` contains zero unit tests; scheduling and sleep queue logic are only tested at runtime.
2. **Missing Block Driver Tests:** `pci.rs` has no unit tests for PCI scanning or BAR decoding.
3. **Missing Filesystem Persistence Tests:** Existing `fs.rs` tests only assert that `BLOCK_CACHE` in memory retains data while the host process lives.

---

## 12. Security & Reliability Findings Inventory

| ID | Severity | File & Line | Summary | Impact & Remediation |
|---|---|---|---|---|
| **SEC-01** | **P1 (High)** | `kernel/src/fs.rs:364-430`, `kernel/src/console.rs:157-190` | Ambient Authority in Filesystem & Console Syscalls | **Impact:** Any Ring 3 task can read/write console and all filesystem files without capability authorization.<br>**Remediation:** Introduce `CapType::File`, `CapType::Directory`, and `CapType::Console` capabilities and require valid handles on syscalls 24–30. |
| **SEC-02** | **P1 (High)** | `kernel/src/syscall.rs:111-113` | Shared Global Mutable Syscall Stack Scratches | **Impact:** `USER_RSP_SCRATCH` and `KERNEL_RSP_SCRATCH` are static mut variables. Reentrant syscalls or multicore execution will corrupt stack pointers.<br>**Remediation:** Store per-task kernel stack in TSS `RSP0` and use `SWAPGS` or per-CPU data structures. |
| **SEC-03** | **P2 (Medium)** | `kernel/src/idt.rs:391`, `kernel/src/notification.rs:400` | Redundant Double EOI on Hardware IRQs | **Impact:** IDT handler sends EOI immediately, and `sys_irq_ack` sends a second EOI. Duplicate EOIs can corrupt 8259 PIC priority arbitration.<br>**Remediation:** Mask the IRQ line in IDT handler and send EOI, then unmask the IRQ line in `sys_irq_ack` (do not send a second EOI). |
| **SEC-04** | **P2 (Medium)** | `kernel/src/fs.rs:97-100`, `kernel/src/pci.rs:89-100` | In-Memory RAM Filesystem Without Block Persistence | **Impact:** VirtIO-Blk block queues are unpopulated; JanusFS data is lost on reboot.<br>**Remediation:** Implement VirtIO virtqueue descriptor rings, available/used index management, and flush `BLOCK_CACHE` sectors to disk. |
| **SEC-05** | **P3 (Low)** | `kernel/src/notification.rs:388-392` | Missing Rights Check in `sys_irq_ack` | **Impact:** Any capability of type `Irq` can acknowledge interrupts regardless of whether `CapRights::WRITE` is present.<br>**Remediation:** Require `irq_slot.rights.contains(CapRights::WRITE)`. |
| **SEC-06** | **P3 (Low)** | `kernel/src/ipc.rs:120-147` | Unsafe Pointer Copy Bypass in Unit Test Build | **Impact:** `copy_user_slice_from/to` uses raw `core::ptr::copy` under `#[cfg(test)]`, bypassing page table permission checks during host testing.<br>**Remediation:** Mock a virtual page table reader in unit tests rather than bypassing checks. |

---

## 13. Exact Remaining Remediation Work (Top 5 Actions)

1. **Implement True VirtIO-Blk Queue Transport:** Allocate and align virtqueue descriptor tables, available rings, and used rings at `virtio_io`. Wire `BLOCK_CACHE` evictions and flushes to VirtIO sector read/write requests so JanusFS data persists across reboots.
2. **Eliminate Ambient Authority with Storage & Console Capabilities:** Define `CapType::File`, `CapType::Directory`, and `CapType::Console`. Update syscalls 24–30 to require valid capability handles with appropriate `READ`/`WRITE` permissions.
3. **Correct IRQ EOI and Masking Protocol:** In hardware IRQ IDT handlers, mask the firing IRQ in the PIC and send a single EOI. In `sys_irq_ack`, verify `CapRights::WRITE` and unmask the IRQ line in the PIC without issuing a second EOI.
4. **Per-Task Kernel Stack Isolation (`SWAPGS`):** Replace global `USER_RSP_SCRATCH` with per-CPU/per-thread GS base or TSS-based stack switching to guarantee re-entrancy and SMP safety.
5. **Add Scheduler & Driver Unit Tests:** Add unit tests for `task.rs` state transitions (sleep, yield, terminate) and `pic.rs` spurious IRQ detection.

---

## 14. Final Release Verdict

**VERDICT: V1 FUNCTIONALLY USABLE — LIMITATIONS DOCUMENTED**

### Justification
- **Why Not "V1 Ready for Release"?** The system does not possess persistent block storage across reboots, and storage/console syscalls violate the capability security model by using ambient authority.
- **Why Not "V1 Not Ready"?** Janus OS genuinely boots through UEFI into an interactive Ring 3 shell, schedules multiple preemptive tasks, enforces capability access control across endpoints and notifications, passes 114 unit tests, and maintains full memory W^X isolation. The operational limitations are well-bounded, documented, and do not prevent interactive usage.

---

## 15. Audit Certification & Git Status

- **Repository:** `medhansh5/Janus`
- **Branch:** `main`
- **HEAD Commit:** `f3130f9a19ab9efc2cc3e5b9a37ca76f36221b6a`
- **Git State:** Local working copy only. Strictly **no commits, no pushes, no branches, and no PRs** were created during this audit.
- **Auditor Signature:** Antigravity Independent Engineering Audit Team
