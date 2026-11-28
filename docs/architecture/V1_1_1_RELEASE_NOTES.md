# Janus OS V1.1.1

**Suggested Tag:** `v1.1.1`  
**Target Commit:** `680aa6102dd8f30dab1480716df18c0734116577`

## Overview

Janus OS V1.1.1 is a security-hardened, uniprocessor x86_64 microkernel release written in Rust (`x86_64-unknown-none`) with a custom UEFI bootloader (`x86_64-unknown-uefi`). It boots via OVMF in QEMU into an interactive Ring 3 shell backed by kernel-enforced object capabilities, higher-half W^X paging, asynchronous notifications, single-EOI 8259 PIC interrupt routing, and persistent JanusFS storage over a legacy VirtIO block driver.

This release closes the hardening items identified in the V1.1 release evidence audit: capability syscall negative testing, transactional filesystem allocation rollback, IRQ ownership cleanup on task/CSpace destruction, and accurate hardware/architecture documentation.

## Major Capabilities

- **UEFI Bootloader & Higher-Half Kernel:** Loads the ELF64 kernel via UEFI `SimpleFileSystem`, maps segments with strict W^X flags (`RX`, `R`, `RW`), and transitions to higher-half virtual memory (`0xFFFF_8000_0000_0000` direct physical map). *(Bootloader signature check is currently a mock pass.)*
- **Virtual Memory & Ring 3 Execution:** Bitmap physical frame allocator (with contiguous DMA frame allocation), isolated per-task user PML4s, pre-validated `copy_from_user`/`copy_to_user`, and Ring 3 transitions via `syscall` / `iretq`.
- **Task Scheduling & Lifecycle:** Preemptive round-robin scheduler driven by 1000 Hz PIT IRQ0, sleep deadlines (`SYS_SLEEP`), idle task fallback, and full `CSpace` teardown on `SYS_EXIT`.
- **Capability-Based Access Control:** 128-slot `CNode`s, Capability Derivation Tree (CDT) with iterative revocation, generation-checked `CPtr` handles, rights attenuation, and typed capabilities (`Untyped`, `Endpoint`, `Notification`, `CNode`, `Frame`, `MmioFrame`, `IOPort`, `Irq`, `Console`, `Storage`). Syscalls 24–30 enforce `Console` and `Storage` capabilities and rights prior to buffer access or I/O.
- **Synchronous IPC & Asynchronous Notifications:** Blocking rendezvous IPC (`SYS_SEND`/`SYS_RECV`) and a 64-object static notification pool supporting 64-bit coalescing event bitmasks (`SYS_NOTIFY_CREATE`/`SIGNAL`/`WAIT`/`POLL`).
- **Hardware Interrupts & PIT Timekeeping:** Dual 8259 PIC (vectors 32–47), spurious IRQ7/IRQ15 handling, IDT masking + single EOI per real IRQ, `CapType::Irq` + `CapRights::WRITE` acknowledgement (`SYS_IRQ_ACK`), and monotonic millisecond uptime (`SYS_GET_TIME`).
- **UART & PS/2 Input:** 16550 COM1 serial (115200 baud, 8N1, FIFO enabled) and PS/2 Set 1 keyboard scancode translation feeding a 512-byte circular input buffer.
- **Legacy VirtIO Block Storage & Persistent JanusFS:** Legacy VirtIO 0.9.5 PCI block driver (`0x1AF4:0x1001`, I/O BAR0, 256-entry VirtQueue) backing JanusFS (512-byte sectors, 64 inodes, 64 cached blocks, 2,560-byte max file size) with cross-reboot disk restoration.
- **Interactive Ring 3 Shell:** Supports `help`, `ps`/`status`, `ls`, `cat`, `write`, `rm`, `time`, `echo`, `test` (Tests 1–5), `clear`, and `exit`.

## V1.1.1 Hardening

- **Capability Negative-Test Coverage:** Added `ipc::tests::test_syscalls_24_30_capability_negative_and_positive` exercising `handle_syscall` across syscalls 24–30 for non-canonical `CPtr`s, empty slots, stale generations, wrong capability types, missing `READ`/`WRITE` rights, unauthorized tasks, and null pointers, verifying zero side effects on denied calls.
- **Transactional Filesystem Allocation:** Updated `fs::write_file`, `fs::delete_file`, and `fs::create_file` to validate bounds and pre-allocate blocks before mutating metadata, propagate `flush_block_to_disk` / `sync` errors, and roll back `INODES` and `BLOCK_CACHE` via static `.bss` `ROLLBACK_BLOCKS` on disk-full or synchronous write failures.
- **IRQ Ownership Lifecycle Cleanup:** Added `notification::cleanup_irq_on_cap_removal` invoked by `capability::cleanup_slot` on `CapType::Irq` deletion, revocation, or task CSpace destruction—unbinding and unmasking orphaned IRQ lines only when zero owners remain, while preserving lines still owned by other tasks. Also fixed `reset_notification_object` / `sys_irq_bind` so destroying an old notification cannot clear a reassigned IRQ binding.
- **Documentation Alignment:** Updated `README.md`, `docs/ARCHITECTURE.md`, and `docs/LIMITATIONS.md` to match the implemented syscall ABI, capability types, and platform constraints.

## Validation

All automated quality gates and builds passed on commit `680aa6102dd8f30dab1480716df18c0734116577`:

- `cargo +nightly fmt --all -- --check`: **PASS** (exit `0`)
- `cargo +nightly test -p kernel --lib`: **PASS** (**122 passed, 0 failed, 0 ignored**)
- `cargo +nightly clippy -p kernel --lib --tests -- -D warnings`: **PASS** (exit `0`, 0 warnings)
- `cargo +nightly build -p kernel --target x86_64-unknown-none --release "-Zbuild-std=core,compiler_builtins"`: **PASS** (exit `0`)
- `cargo +nightly build -p bootloader --target x86_64-unknown-uefi --release "-Zbuild-std=core,alloc,compiler_builtins"`: **PASS** (exit `0`)
- **GitHub Actions CI (Run `46000266477`):** **PASS** (`conclusion: success`)
- **QEMU Runtime & Persistence Validation:** Verified via isolated 3-boot OVMF UEFI QEMU testing (Ring 3 shell commands, Tests 1–5 including capability rejection, file creation/retention across Boot 1 → Boot 2, and persistent deletion across Boot 2 → Boot 3 on the same VirtIO disk image).

## Known Limitations

- **Uniprocessor Only (No SMP):** Targets single-core x86_64 only. `syscall_entry` relies on global `USER_RSP_SCRATCH` and `KERNEL_RSP_SCRATCH` with `SFMask` clearing `IF=0`; it is not SMP-safe and not reentrant under NMI/fault during the entry window.
- **Legacy VirtIO-Blk & Synchronous Polling:** Supports only legacy VirtIO 0.9.5 PCI (`0x1AF4:0x1001`) with an I/O-space `BAR0` and queue size `<= 256`, using bounded synchronous polling (`2_000_000` spins) rather than async IRQ11 completion.
- **JanusFS Capacity & Crash Consistency:** Limited to 64 inodes, 64 cached 512-byte blocks, 2,560-byte max file size, and 16-byte filenames. Provides in-memory and synchronous write-error rollback, but **no journaling or copy-on-write** for power-loss atomicity across multi-sector flushes.
- **No `std` in Userspace & No Bare-Metal Compatibility Claim:** Ring 3 tasks use `no_std` assembly `syscall` wrappers; runtime validation is established on QEMU x86_64 + OVMF only.
