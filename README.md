# Janus OS

[![CI-badge](https://github.com/ashcresthq/janus/actions/workflows/ci.yml/badge.svg)](https://github.com/ashcresthq/janus/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT_or_Apache--2.0-blue.svg)](LICENSE)

**Janus** is a security-first microkernel operating system written in Rust, built around kernel-enforced capabilities and zero ambient authority. All resource access requires explicit, unforgeable capability handles managed strictly by the microkernel.

> **GitHub Topics**: `rust` • `osdev` • `operating-system` • `microkernel` • `kernel` • `systems-programming` • `security` • `x86-64`

---

## Architecture Diagram

```text
+-----------------------------------------------------------------------+
|                             USERSPACE (Ring 3)                         |
|                                                                       |
|  +--------------------+   +-------------------+   +----------------+  |
|  | WASM Application   |   | Virtio-Blk Driver |   |   Init Task    |  |
|  | (Sandboxed App)    |   | (Storage Service) |   | (Cap Root/Boot)|  |
|  +---------+----------+   +---------+---------+   +-------+--------+  |
|            |                        |                     |           |
|            | CPtr Handle            | CPtr / IOPort       | Manifest  |
|            v                        v                     v           |
|=======================================================================|
|                   SYSTEM CALL & CAPABILITY BOUNDARY                   |
|                        `syscall` / `sysret` ABI                        |
|=======================================================================|
|                             MICROKERNEL (Ring 0)                      |
|                                                                       |
|  +------------------+  +-------------------+  +--------------------+  |
|  | CSpace & Cap     |  | Preemptive        |  |  Physical Frame    |  |
|  | Table Validation |  | Scheduler & IPC   |  |  & Page Table Alloc|  |
|  +------------------+  +-------------------+  +--------------------+  |
|                                                                       |
|  +-----------------------------------------------------------------+  |
|  | Hardware IRQ Routing & InterruptEndpoint Dispatch (IRQ0 Timer)  |  |
|  +-----------------------------------------------------------------+  |
+-----------------------------------------------------------------------+
```

---

## Feature Maturity Classification

### Implemented & Validated Functionality (v1.1.1)
- **Capability-Gated System Services**: Console I/O (syscalls 24–25) and JanusFS storage (syscalls 26–30) require explicit `CapType::Console` / `CapType::Storage` capabilities with `READ`/`WRITE` rights.
- **Persistent JanusFS on Legacy VirtIO-Blk**: Kernel legacy VirtIO 0.9.5 block driver (`0x1AF4:0x1001`, I/O BAR0, 256-entry VirtQueue, synchronous polling with bounded timeout) backing JanusFS sector persistence across reboots with transactional allocation rollback on disk-full or I/O errors.
- **Hardware IRQ Routing & Single-EOI Protocol**: Dual 8259 PIC vectors 32–47 with spurious IRQ7/IRQ15 handling, IDT masking + single EOI, `CapType::Irq` + `CapRights::WRITE` acknowledgement, asynchronous `Notification` delivery, and automatic IRQ unbind/unmask cleanup on last-owner destruction.
- **Contiguous DMA & W^X Memory Safety**: Higher-half paging, isolated user PML4s, and `copy_from_user`/`copy_to_user` pre-validation.
- **Preemptive IRQ0 Scheduler & Ring 3 Interactive Shell**: 1000 Hz PIT timer scheduler, Ring 3 shell (`help`, `ps`, `ls`, `cat`, `write`, `rm`, `time`, `echo`, `test`, `clear`, `exit`).

### Planned Functionality
- **WASM Application Runtime**: Sandboxed Ahead-Of-Time (AOT) WebAssembly application execution.
- **Asynchronous IRQ-Driven Block I/O & Modern VirtIO / NVMe**.
- **Crash-Consistent Journaling / CoW Filesystem**.
- **Multi-Core Support (SMP)**: x2APIC migration, per-CPU `swapgs` stacks, IPIs, and multi-core scheduling.

---

## Supported Hardware & Target Specifications

### Target Triples
- **Kernel Target**: `x86_64-unknown-none`
- **Bootloader Target**: `x86_64-unknown-uefi`

### Supported Hardware & VM Platforms
- **Primary Emulator**: QEMU x86_64 (`qemu-system-x86_64`) with OVMF UEFI firmware, Virtio-Blk disk controller (`-drive file=...,if=virtio`), and 8254 PIT / APIC timer interrupts.
- **Bare-Metal Hardware**: 64-bit x86_64 CPUs with UEFI firmware support and PCI bus controllers.

### Unsupported Targets (Out of Scope for v1)
- 32-bit x86 architecture.
- Legacy BIOS / CSM boot modes.
- ARM64 / AArch64 and RISC-V architectures (planned for future major releases).

---

## Clean-Machine Build & Boot Instructions

Follow these instructions to install dependencies, build the kernel, and launch Janus in QEMU from a fresh operating system installation.

### 1. Install Prerequisites

#### Ubuntu / Debian
```bash
sudo apt update
sudo apt install -y build-essential git qemu-system-x86 ovmf
```

#### Fedora
```bash
sudo dnf install -y make git qemu-system-x86 edk2-ovmf
```

#### Arch Linux
```bash
sudo pacman -S --needed base-devel git qemu-system-x86_64 edk2-ovmf
```

#### macOS (Homebrew)
```bash
brew install make git qemu
```

### 2. Install Rust Nightly Toolchain

```bash
# Install rustup if not present
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source $HOME/.cargo/env

# Add nightly targets and components
rustup toolchain install nightly
rustup target add x86_64-unknown-none x86_64-unknown-uefi --toolchain nightly
rustup component add rust-src llvm-tools-preview rustfmt clippy --toolchain nightly
```

### 3. Clone and Build

```bash
git clone https://github.com/ashcresthq/janus.git
cd janus

# Build bootloader and kernel binaries
make all
```

### 4. Run in QEMU

```bash
# On Ubuntu/Debian (default OVMF path: /usr/share/ovmf/OVMF.fd)
make run

# On Fedora / Arch / macOS (override OVMF path as needed):
make run OVMF=/usr/share/edk2/x64/OVMF.fd
```

---

## Kernel / Userspace Boundary

Janus relies on x86_64 hardware system call instructions (`syscall` / `sysret`) to separate Ring 0 microkernel space from Ring 3 userspace tasks.

- **System Call ABI**: Userspace passes system call IDs in `rax`, capability handles (`CPtr`) in `rdi`, and payload registers in `rsi`, `rdx`, `r10`, and `r8`.
- **Memory Boundary**: Kernel memory structures cannot be accessed by Ring 3 tasks. Virtual memory mappings are managed via isolated page tables per process.
- **Capability Lookup**: Every syscall validates that the caller holds a valid `CPtr` in its `CSpace` table.

---

## Security Model

- **Zero Ambient Authority**: Processes possess no implicit permissions based on process ID or user credentials.
- **Kernel-Managed Capabilities**: Capabilities exist only inside kernel memory. Userspace holds opaque `CPtr` handles.
- **W^X Enforcement**: Pages are configured strictly as Writable (W) or Executable (X), but never both.
- **I/O & Interrupt Isolation**: Hardware drivers execute in Ring 3 with port I/O restricted to assigned PCI BAR ranges and IRQs delivered over `InterruptEndpoint` handles.

For complete details, see [SECURITY.md](SECURITY.md) and [THREAT-MODEL.md](docs/THREAT-MODEL.md).

---

## Known Limitations

- **Single Core (Uniprocessor)**: The v1 release targets uniprocessor x86_64 execution. Multi-core SMP support is deferred to v4.
- **Raw System Call ABI**: Higher-level Rust standard library support (`std`) is not available in Ring 3 tasks. Driver tests invoke syscalls via `no_std` assembly.
- **Firmware Path**: Default `Makefile` uses `/usr/share/ovmf/OVMF.fd`. Non-Debian platforms require passing `OVMF=/path/to/OVMF.fd`.

For full details, see [LIMITATIONS.md](docs/LIMITATIONS.md).

---

## Repository Governance & Branch Strategy

- **`main` Branch**: Production-ready, stable development branch. All PRs must pass automated CI checks (`.github/workflows/ci.yml`).
- **Feature Branches**: `feature/<name>` for new features, `fix/<issue-number>` for fixes, `docs/<topic>` for documentation.
- **Releases & Tags**: Milestone releases are tagged with semantic versioning (e.g., `v1.0.0`, `v0.1.0`). See [RELEASES.md](docs/RELEASES.md).

---

## Documentation Index

- [ARCHITECTURE.md](docs/ARCHITECTURE.md) - System architecture, system call ABI, and memory model.
- [THREAT-MODEL.md](docs/THREAT-MODEL.md) - Threat analysis, attack surface, and security boundaries.
- [INIT-POLICY.md](docs/INIT-POLICY.md) - Init process capability grant design.
- [ROADMAP.md](docs/ROADMAP.md) - Development roadmap and milestone timeline.
- [LIMITATIONS.md](docs/LIMITATIONS.md) - Technical constraints and non-goals.
- [RELEASES.md](docs/RELEASES.md) - Versioning, tagging, and branch strategy.
- [CONTRIBUTING.md](CONTRIBUTING.md) - Code style, testing, and PR submission rules.
- [SECURITY.md](SECURITY.md) - Security guarantees and vulnerability reporting policy.
- [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) - Community guidelines.

---

## License

Janus is dual-licensed under either the [MIT License](LICENSE-MIT) or the [Apache License, Version 2.0](LICENSE-APACHE) at your option. See [LICENSE](LICENSE) for details.
