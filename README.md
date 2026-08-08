# Janus OS

Janus is a security-first microkernel operating system built around kernel-enforced capabilities and (planned) sandboxed WebAssembly applications. Designed from the ground up in Rust, it strictly adheres to the principle of least privilege: there is zero ambient authority, and every action requires explicit, unforgeable capabilities managed entirely by the kernel.

## Current Status: v1 Milestone Complete

Janus has successfully completed its v1 milestone, proving that real hardware drivers can run safely as unprivileged userspace tasks. The following core features are implemented and validated:
- **Userspace Drivers (Virtio-Blk)**: The first hardware driver runs entirely in Ring 3, communicating with the kernel via capabilities, proving the microkernel boundary.
- **Hardware Interrupt Delivery**: Device IRQs are routed securely to userspace tasks via the `InterruptEndpoint` capability.
- **Contiguous DMA Allocator**: The kernel can securely provision contiguous physical memory frames for hardware DMA buffers.
- **Scoped PCI Isolation**: `IOPort` capabilities are explicitly scoped so drivers can only touch their own device's Base Address Registers.
- **Idle Task Mechanism**: A dedicated Ring 0 idle task cleanly handles empty runqueues.
- **Capability-Enforced Memory**: The physical frame allocator respects reserved bootloader regions and vends memory via strict capability typing.
- **Preemptive Scheduling**: A hardware timer (IRQ0)-driven scheduler that preempts execution seamlessly in userspace.
- **Synchronous IPC**: Fast, blocking Inter-Process Communication with capability passing.
- **Security Boundary Proven**: The capability enforcement holds up under stress-testing and arbitrary preemption. Unauthorized operations (e.g., a process attempting to use an invalid Endpoint capability or unowned physical Frame) correctly trigger a kernel-level security fault and terminate the offending task.

### What is *NOT* in Janus (Yet)
To set expectations clearly, Janus is still in its infancy. As of v1, there are:
- **No filesystem** (VFS)
- **No WASM runtime integration** (test tasks are written in raw assembly)
- **No independent driver restartability** (a crashed driver terminates securely, but the system doesn't yet automatically recover it)
- **No UI or shell**
- **Single-core support only** (multi-core APIC migration deferred to future milestones)

## Architecture Overview

Janus relies on a strict microkernel architecture inspired by seL4. The kernel handles only scheduling, memory, IPC, capabilities, and IRQ delivery. All services and drivers run as unprivileged userspace processes. Capabilities are never stored in userspace memory; instead, applications reference capabilities via opaque handles (`CPtr`). If a capability is revoked, access is instantaneously and cascadingly cut off across the entire system.

For a deep dive into the design and threat model, see:
- [ARCHITECTURE.md](docs/ARCHITECTURE.md)
- [THREAT-MODEL.md](docs/THREAT-MODEL.md)
- [INIT-POLICY.md](docs/INIT-POLICY.md)
- [Development Walkthrough](docs/walkthrough.md) (a great read on the debugging history!)

## Build & Run Instructions

To build and run Janus locally in QEMU, you will need a Rust nightly toolchain (handled automatically by `rust-toolchain.toml`), QEMU, and OVMF (for UEFI support).

### Prerequisites
- **QEMU**: `sudo apt install qemu-system-x86` (Ubuntu/Debian) or `brew install qemu` (macOS)
- **OVMF**: `sudo apt install ovmf` (Ubuntu/Debian). *Note: On macOS, QEMU typically bundles EFI firmware, or you may need to source the `OVMF.fd` binary manually depending on your Homebrew setup.*

### Running
1. Clone the repository: `git clone https://github.com/ashcresthq/janus`
2. Navigate to the project root: `cd janus`
3. Build and launch in QEMU: 
```bash
make run
```
You will see the UEFI bootloader execute, jump to the kernel, and spawn the initial IPC stress-test tasks.

## Design Philosophy

- **No Ambient Authority**: If you do not have a capability for it, it does not exist in your namespace.
- **Kernel-Managed Capabilities**: The kernel retains sole authority to mint, grant, and revoke privileges.
- **Rust for Memory Safety**: The entire system is built in Rust to eliminate spatial and temporal memory safety bugs by default.
- **Microkernel Isolation**: A compromised driver or parser cannot laterally move to other components because it lacks the capabilities to do so.

## Contributing

Janus is currently in its highly experimental v0/v1 planning phase. **While the repository is public for visibility and discussion, we are not actively accepting code PRs for new features at this exact moment** until the v1 architecture (specifically the capability initialization tree and WASM integration) is fully specified.

If you are interested in the design, please feel free to open Issues for discussion, architecture suggestions, or identifying threat model gaps!

### Security Vulnerability Reporting
If you believe you've found a security-relevant flaw in the capability model or kernel implementation, feel free to open a public GitHub issue. Because this project is pre-production and not yet handling untrusted workloads, there is no formal responsible-disclosure process or embargo needed at this stage.

## License

This project is dual-licensed under the MIT and Apache-2.0 licenses. See the `LICENSE` file for details.
