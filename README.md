# Janus OS

[![CI-badge](https://github.com/medhansh5/Janus/actions/workflows/ci.yml/badge.svg)](https://github.com/medhansh5/Janus/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT_or_Apache--2.0-blue.svg)](LICENSE)

**Janus** is an experimental, security-focused x86_64 microkernel operating system written in Rust, built around kernel-managed object capabilities (`CPtr` handles), W^X page isolation, and Ring 3 execution.

> **Scope Notice:** Janus V1.1.1 is an early-stage research/systems-engineering microkernel targeting **uniprocessor QEMU x86_64** with OVMF UEFI firmware. It is **not** production-ready, **not** POSIX-compatible, and does **not** claim general-purpose bare-metal hardware compatibility.

---

## Architecture Overview

```text
+-----------------------------------------------------------------------+
|                             USERSPACE (Ring 3)                         |
|                                                                       |
|  +----------------------------------+   +--------------------------+  |
|  | Task A: Interactive Shell        |   | Task B: IPC Responder    |  |
|  | (`help`, `ls`, `cat`, `write`,   |   | (Synchronous rendezvous  |  |
|  |  `rm`, `ps`, `time`, `test`, ...) |   |  IPC via Endpoint caps)  |  |
|  +----------------+-----------------+   +------------+-------------+  |
|                   |                                  |                |
|                   | `syscall` / `iretq` (CPtr in `rsi`)                |
|                   v                                  v                |
|=======================================================================|
|                   SYSTEM CALL & CAPABILITY BOUNDARY                   |
|=======================================================================|
|                             MICROKERNEL (Ring 0)                      |
|                                                                       |
|  +------------------+  +-------------------+  +--------------------+  |
|  | CSpace & CDT     |  | Preemptive IRQ0   |  | Physical Frame &   |  |
|  | Validation       |  | Scheduler & IPC   |  | Higher-Half Paging |  |
|  +------------------+  +-------------------+  +--------------------+  |
|  +------------------+  +-------------------+  +--------------------+  |
|  | Console (COM1 /  |  | JanusFS & Legacy  |  | 8259 PIC & Async   |  |
|  | PS/2) Service    |  | VirtIO-Blk Driver |  | Notification Pool  |  |
|  +------------------+  +-------------------+  +--------------------+  |
+-----------------------------------------------------------------------+
```

---

## Feature Maturity Classification

### Implemented & Validated Functionality (v1.1.1)
- **UEFI Bootloader & Higher-Half x86_64 Kernel**: Custom `x86_64-unknown-uefi` loader maps ELF segments with W^X permissions (`RX` / `R` / `RW`) and jumps to the higher-half kernel. *(Note: kernel signature verification in the bootloader is currently a mock check.)*
- **Virtual Memory & Ring 3 Isolation**: Bitmap physical frame allocator, direct physical map (`0xFFFF_8000_...`), isolated user PML4s, and pre-validated `copy_from_user` / `copy_to_user`.
- **Capability-Based Access Control**: 128-slot per-task `CNode`s, Capability Derivation Tree (CDT) with iterative revocation, generation-tagged `CPtr` handles, and rights attenuation across `Untyped`, `Endpoint`, `Notification`, `CNode`, `Frame`, `MmioFrame`, `IOPort`, `Irq`, `Console`, and `Storage`.
- **Capability-Gated System Services**: Console I/O (syscalls 24–25) and JanusFS storage (syscalls 26–30) require explicit `CapType::Console` / `CapType::Storage` capabilities with `READ` / `WRITE` rights. *(Diagnostic syscalls such as `SYS_GET_TIME=23` and `SYS_TASK_STATUS=32` remain un-gated.)*
- **Synchronous IPC & Asynchronous Notifications**: Blocking rendezvous IPC (`SYS_SEND`/`SYS_RECV`) and 64-object coalescing event bitmask `Notification`s (`SYS_NOTIFY_CREATE`/`SIGNAL`/`WAIT`/`POLL`).
- **Hardware Interrupts & PIT Timekeeping**: Dual 8259 PIC (vectors 32–47), 1000 Hz PIT timer preemption, spurious IRQ7/IRQ15 filtering, single-EOI masking/unmasking (`CapType::Irq` + `CapRights::WRITE`), and automatic IRQ unbind/unmask cleanup on last-owner destruction.
- **Persistent JanusFS on Legacy VirtIO-Blk**: Kernel legacy VirtIO 0.9.5 PCI driver (`0x1AF4:0x1001`, I/O BAR0, 256-entry VirtQueue, synchronous polling with bounded spin timeout) backing JanusFS sector persistence across QEMU reboots, with transactional allocation rollback on disk-full or synchronous write errors.
- **Interactive Ring 3 Shell**: `help`, `ps`/`status`, `ls`, `cat`, `write`, `rm`, `time`, `echo`, `test`, `clear`, and `exit`.

### Planned / Future Work
- **Multi-Core Support (SMP)**: x2APIC migration, IPIs, and per-CPU `swapgs` syscall stacks.
- **Asynchronous IRQ-Driven Block I/O & Modern VirtIO / NVMe**.
- **Crash-Consistent Journaling / Copy-on-Write Filesystem**.
- **Userspace Driver Isolation & WASM Application Runtime**.

---

## Target Specifications & Tested Environment

- **Kernel Target:** `x86_64-unknown-none`
- **Bootloader Target:** `x86_64-unknown-uefi`
- **Verified Emulator Platform:** QEMU x86_64 (`qemu-system-x86_64`) with OVMF UEFI firmware, legacy VirtIO-Blk (`-drive file=...,format=raw,if=virtio`), COM1 serial, PS/2 keyboard, and 8254 PIT / 8259 PIC.
- **Unsupported:** 32-bit x86, Legacy BIOS/CSM, ARM64/RISC-V, SMP, and unverified general-purpose bare-metal hardware.

---

## Build, Test & QEMU Execution

### Prerequisites
- Rust `nightly` toolchain with `rust-src`, `llvm-tools-preview`, `rustfmt`, `clippy`, and targets `x86_64-unknown-none` and `x86_64-unknown-uefi`.
- `qemu-system-x86_64` and OVMF UEFI firmware (`OVMF.fd` / `edk2-x86_64-code.fd`).

### Build & Quality Gates
```bash
# Formatting, unit tests (122 tests), and Clippy
cargo +nightly fmt --all -- --check
cargo +nightly test -p kernel --lib
cargo +nightly clippy -p kernel --lib --tests -- -D warnings

# Release builds
cargo +nightly build -p kernel --target x86_64-unknown-none --release "-Zbuild-std=core,compiler_builtins"
cargo +nightly build -p bootloader --target x86_64-unknown-uefi --release "-Zbuild-std=core,alloc,compiler_builtins"
```

### Running in QEMU
Stage `esp/EFI/BOOT/BOOTX64.EFI` (from `bootloader.efi`) and `esp/kernel.elf` (from `kernel`), create a raw disk image `disk.img` (1 MiB zeroed), and launch QEMU:
```bash
qemu-system-x86_64 \
  -drive if=pflash,format=raw,readonly=on,file=/path/to/OVMF_CODE.fd \
  -drive format=raw,file=fat:rw:esp \
  -drive file=disk.img,format=raw,if=virtio \
  -serial stdio \
  -display none \
  -no-reboot
```
*(A `Makefile` is also provided for Linux debug builds via `make all` / `make run OVMF=...`; note that `make run` recreates `test.img` on each invocation.)*

---

## Known Limitations

- **Uniprocessor Only (No SMP):** Single-core x86_64 only; `syscall_entry` uses global `USER_RSP_SCRATCH` / `KERNEL_RSP_SCRATCH` with `SFMask` clearing `IF=0` (not SMP-safe and not NMI-reentrant).
- **Legacy VirtIO-Blk & Synchronous Polling:** Requires legacy VirtIO 0.9.5 PCI (`0x1AF4:0x1001`) with I/O `BAR0` and queue size `<= 256`; I/O uses bounded synchronous polling rather than async IRQ11 completion.
- **JanusFS Limits & Crash Consistency:** Fixed 512-byte sectors (`64` cached blocks, `64` inodes, `2560` B max file size, 16-byte filenames). Persists across clean QEMU reboots with transactional rollback on disk-full or write errors, but has **no journaling or copy-on-write** for power-loss atomicity.
- **No `std` in Userspace:** Ring 3 tasks use `no_std` assembly `syscall` wrappers (`rdi`=syscall number, `rsi`=arg1/`CPtr`, `rdx`=arg2, `r10`=arg3, `r8`=arg4, `r9`=arg5, returning via `iretq`).

For complete details, see [docs/LIMITATIONS.md](docs/LIMITATIONS.md), [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md), and [docs/architecture/V1_1_1_HARDENING_REPORT.md](docs/architecture/V1_1_1_HARDENING_REPORT.md).

---

## License

Janus is dual-licensed under either the [MIT License](LICENSE-MIT) or the [Apache License, Version 2.0](LICENSE-APACHE) at your option. See [LICENSE](LICENSE) for details.
