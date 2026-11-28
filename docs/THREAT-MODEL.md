# Janus Threat Model

This document outlines the threat model for Janus OS as of **V1.1.1**, distinguishing **implemented mitigations** from **planned mitigations** and **out-of-scope risks**.

## 1. Threat Categories & Current Mitigations

The core design goal of Janus is to minimize lateral movement by enforcing capability-based isolation between Ring 3 tasks and Ring 0 kernel state.

### 1.1 Unprivileged Userspace Tasks
- **Threat**: A Ring 3 task attempts to read/write kernel memory, access another task's memory, or use console/storage services without authorization.
- **Implemented Mitigation**:
  - Per-task isolated PML4 page tables; `copy_from_user` / `copy_to_user` pre-validate null pointers, kernel-half bounds, canonicality, page presence, and `USER_ACCESSIBLE` flags.
  - Syscalls 24–30 (`SYS_READ_CONSOLE`..`SYS_FS_LIST`) validate `CapType::Console` / `CapType::Storage`, generation-tagged `CPtr`s, and `READ`/`WRITE` rights before accessing user buffers or performing I/O.
  - Invalid/revoked capability handles return `-1` (`!0`).
- **Planned Mitigation**: Sandboxed WASM AOT application execution and isolated Ring 3 VFS/driver processes (not yet implemented in V1.1.1).

### 1.2 Memory Corruption & Code Execution
- **Threat**: Buffer overflows, Use-After-Free (UAF), or code injection in kernel or userspace.
- **Implemented Mitigation**:
  - Written primarily in Rust, which prevents spatial/temporal memory bugs in safe code; however, kernel internals, DMA rings, and assembly entry points rely on `unsafe` blocks that must be audited manually.
  - Strict W^X (`RX` / `R` / `RW`, non-executable `GNU_STACK`) enforced on kernel ELF segments and user `Frame` / `MmioFrame` capability mappings.
- **Not Implemented (Planned)**: Kernel or userspace **ASLR** is **not implemented** in V1.1.1; kernel virtual base (`0xffffffff80000000`) and user task addresses (`0x400000`+) are static.

### 1.3 Unauthorized Kernel Modification (Verified Boot)
- **Threat**: Modifying `kernel.elf` on the EFI System Partition before boot.
- **Current Status (Placeholder)**: `bootloader/src/main.rs` currently logs `[MOCK PASS]` for kernel signature verification. **Cryptographic verified boot is not implemented in V1.1.1**; tampering with `kernel.elf` on disk is not detected at boot.

### 1.4 Device & Interrupt Isolation
- **Threat**: Unauthorized port I/O or interrupt spoofing/storms.
- **Implemented Mitigation**:
  - `CapType::IOPort` validates port base, range, and access width (`1`/`2`/`4` bytes) for `sys_in_port`/`sys_out_port`.
  - Hardware IRQs mask the firing PIC line in `idt.rs` (single EOI) and require `CapType::Irq` + `CapRights::WRITE` in `sys_irq_ack` to unmask; orphaned IRQ lines are unbound/unmasked on last-owner CSpace destruction.
- **Current Limitation**: In V1.1.1, the active legacy VirtIO-Blk driver and JanusFS run inside Ring 0 (kernel TCB) rather than as untrusted Ring 3 processes.

---

## 2. Explicitly Out of Scope / Accepted Risks for V1.1.1

1. **Side-Channel Attacks (Spectre, Meltdown, Cache Timing)**: Not mitigated.
2. **Physical Access & Evil Maid Attacks**: No Full Disk Encryption (FDE), no TPM measured boot, and mock bootloader signature check.
3. **Local Denial of Service (DoS) & Resource Starvation**: No CPU time quotas or IPC rate limits. Synchronous VirtIO-Blk polling uses a bounded `2_000_000`-spin timeout to avoid infinite hangs, but blocks the kernel during I/O.
4. **Power-Loss Crash Consistency**: JanusFS has no journaling or copy-on-write; power loss mid-flush across sectors is not crash-atomic.
5. **Uniprocessor & Syscall Scratch Assumptions**: Single-core x86_64 only. `syscall_entry` uses global `USER_RSP_SCRATCH`/`KERNEL_RSP_SCRATCH` with `SFMask` (`IF=0`), which is unsafe for SMP and not reentrant under NMI/faults in the entry window.
6. **Residual Ambient / Diagnostic Syscalls (Design Limitations, Not Kernel-Memory Exposure)**: Syscalls `8`, `19`, `23`, and `32` do not require capability handles in V1.1.1. None of them allow reading or writing kernel memory, as user pointers are validated via `copy_from_user`/`copy_to_user`:
   - **`SYS_LOG` (8)**: Reads a `u32` from user memory via `copy_user_u32` (validated for null, kernel-half, canonicality, presence, and `USER_ACCESSIBLE`) and writes a hex diagnostic line to COM1 (`0x3F8`) **without** requiring `CapType::Console` (`WRITE`). This is an intentional diagnostic limitation that bypasses console capability isolation for serial output.
   - **`SYS_SLEEP` (19)**: Validates `current_task_mut()` and blocks the calling task until `current_tick + duration_ms` (or yields via `schedule()` when `duration_ms == 0`). It affects only the caller's own scheduling state and cannot block or modify another task; local CPU/scheduling DoS remains out of scope (§2.3).
   - **`SYS_GET_TIME` (23)**: Returns the global monotonic 1 ms PIT tick count (`idt::get_tick_count()`). It is read-only and exposes no kernel addresses, though it provides a coarse 1 ms timing source to any caller (timing side-channels are out of scope per §2.1).
   - **`SYS_TASK_STATUS` (32)**: Formats system uptime and every active/terminated task's PID and `TaskState` (`0..MAX_TASKS`) into a user buffer validated via `copy_user_slice_to`. While it does not expose kernel memory addresses or mutate other tasks, it discloses cross-task existence and scheduling/blocking states to unprivileged callers without a capability.

---

## 3. Component Trust Levels (V1.1.1)

1. **UEFI Bootloader (`bootloader`)**: Trusted Boot Stub (mock signature check).
2. **Microkernel (`kernel`, Ring 0)**: Highly Trusted TCB — includes scheduler, paging, `CNode`/CDT, IPC, notifications, PIC/IDT, COM1/PS2 console, legacy VirtIO-Blk polling driver, and JanusFS.
3. **Userspace Tasks (`Task A`, `Task B`, Ring 3)**: Untrusted/isolated — `Task A` holds `Console`, `Storage`, `Endpoint`, `Frame`, and `Irq` capabilities; `Task B` holds only `Endpoint`, `Frame`, `IOPort`, and `Irq` capabilities (no `Console` or `Storage`).
4. **Supply-Chain Risk**: Minimal `no_std` crate set (`bitflags`, `x86_64`, `spin`, `volatile`, `uefi`, `xmas-elf`) pinned via `Cargo.lock`.
