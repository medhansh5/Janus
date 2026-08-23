# Janus OS Development Roadmap

This document outlines completed milestones and future engineering phases for Janus OS.

## Milestone Status Overview

```text
[v1.0 Complete]  ──>  [v2.0 Near-Term]  ──>  [v3.0 Medium-Term]  ──>  [v4.0 Long-Term]
Microkernel Core      Cap Trees & Fault     WASM Execution &      SMP & Multi-Arch
Virtio-Blk Driver     Driver Recovery       VFS Storage Service   x2APIC / ARM64
```

---

## Completed Milestone: v1.0 (Microkernel Core & Ring 3 Virtio Proof)

- [x] **Rust Microkernel Architecture**: Kernel entry, GDT/IDT setup, page table management, and `syscall`/`sysret` boundary.
- [x] **Capability Enforcement Engine**: `CSpace` node management, CPtr lookup, typed memory capabilities (`Untyped`, `Frame`, `CNode`).
- [x] **Ring 3 Virtio-Blk Storage Driver**: Unprivileged storage driver communicating exclusively via capabilities.
- [x] **Scoped PCI I/O Isolation**: `IOPort` capability bitmask restricting driver port access strictly to device BARs.
- [x] **Hardware Interrupt Delivery**: Device IRQ routing via `InterruptEndpoint` capabilities.
- [x] **Preemptive Scheduler**: Hardware IRQ0 timer-driven preemptive task scheduling and idle task handle.
- [x] **Contiguous DMA Memory Allocator**: Physical frame provision for hardware DMA memory.
- [x] **UEFI Verified Bootloader Stub**: UEFI handover, kernel verification, and initial RAM filesystem preparation.

---

## Phase 2: v2.0 (Dynamic Capability Delegation & Fault Recovery)

- [ ] **Dynamic Capability Delegation Tree**: Implement static manifest parsing in `init` process (`INIT-POLICY.md`) to dynamically partition root capabilities to services.
- [ ] **Independent Driver Restartability**: Detect crashed Ring 3 driver tasks, reclaim capabilities, re-initialize device queues, and restart service without restarting the microkernel.
- [ ] **Asynchronous Event Notifications**: Non-blocking capability event queues alongside synchronous IPC.
- [ ] **Iterative Capability Revocation Hardening**: Deep derivation tree testing and memory frame cleanup validation under stress conditions.

---

## Phase 3: v3.0 (WASM Execution Engine & System Services)

- [ ] **Ahead-Of-Time (AOT) WASM Compilation**: Transition from WebAssembly interpreter (`wasmi`) to AOT compiled WebAssembly execution modules.
- [ ] **Capability-Scoped WASM Syscall Bridge**: Expose explicit capability handles (`CPtr`) to sandboxed WASM applications.
- [ ] **Virtual File System (VFS) Service**: Isolated Ring 3 VFS process handling file capabilities, mount points, and storage IPC.
- [ ] **Userland Console & Interactive Shell**: Ring 3 terminal driver and interactive command-line interface.

---

## Phase 4: v4.0 (SMP Multi-Core & Hardware Expansion)

- [ ] **x2APIC Migration & Multi-Core (SMP)**: Inter-Processor Interrupts (IPI), per-core runqueues, and lock-free kernel capability table sync.
- [ ] **ARM64 / AArch64 Target Support**: Port bootloader and microkernel page table management to ARM64 architecture.
- [ ] **Measured Boot & TPM Integration**: TPM-backed measured boot and full disk encryption keys.
