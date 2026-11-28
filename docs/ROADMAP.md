# Janus OS Development Roadmap

This document tracks implemented milestones and planned future engineering phases for Janus OS.

## Milestone Status Overview

```text
[v0.1.0 / v1.0.0] ──> [v1.1 / v1.1.1 Complete] ──> [v2.x Planned] ──> [v3.x+ Deferred]
Core & IPC            Notifications, Ring 3 Shell,   Userspace Drivers  WASM AOT, SMP/x2APIC,
                      Persistent JanusFS, Hardening  & Dynamic Init     Journaling FS, NVMe
```

---

## Implemented Milestones

### v0.1.0 / v1.0.0 (Microkernel Core & Capability Foundations)
- [x] **Rust Microkernel Architecture**: Higher-half ELF64 kernel (`x86_64-unknown-none`), GDT/TSS/IDT setup, W^X page tables, and `syscall`/`iretq` boundary.
- [x] **Capability Enforcement Engine**: 128-slot `CNode`s, CDT iterative revocation, generation-tagged `CPtr`s, and typed capabilities (`Untyped`, `Endpoint`, `CNode`, `Frame`, `MmioFrame`, `IOPort`, `Irq`).
- [x] **Dynamic User Address Spaces**: Isolated per-task PML4 hierarchies, higher-half direct map, and pre-validated `copy_from_user` / `copy_to_user`.
- [x] **Preemptive Scheduler**: 1000 Hz PIT IRQ0 round-robin scheduling with Ring 0 idle task fallback.
- [x] **Contiguous DMA Allocator**: Physical frame allocator supporting contiguous multi-page DMA allocations.
- [x] **UEFI Bootloader Stub**: `x86_64-unknown-uefi` loader (`SimpleFileSystem`, W^X ELF mapping, mock kernel signature check).

### v1.1 & v1.1.1 (Usable System Services, Persistence & Hardening)
- [x] **Asynchronous Notifications**: 64-object static notification pool with 64-bit coalescing event bitmasks (`SYS_NOTIFY_CREATE`, `SIGNAL`, `WAIT`, `POLL`).
- [x] **Hardware IRQ Routing & Lifecycle Cleanup**: Dual 8259 PIC (vectors 32–47), spurious IRQ7/IRQ15 handling, single-EOI masking + `SYS_IRQ_ACK` (`CapType::Irq` + `CapRights::WRITE`), and automatic unbind/unmask on last-owner CSpace destruction.
- [x] **Capability-Gated System Services**: `CapType::Console` and `CapType::Storage` enforcement on syscalls 24–30, backed by negative-test coverage in `ipc::tests`.
- [x] **Legacy VirtIO-Blk & Persistent JanusFS**: In-kernel legacy VirtIO 0.9.5 PCI driver (`0x1AF4:0x1001`, I/O BAR0, queue `<= 256`, synchronous polling) and JanusFS sector persistence across QEMU reboots with transactional allocation/write-error rollback.
- [x] **Interactive Ring 3 Shell**: `Task A` shell supporting `help`, `ps`/`status`, `ls`, `cat`, `write`, `rm`, `time`, `echo`, `test`, `clear`, and `exit`.
- [x] **UART & PS/2 Input**: 16550 COM1 serial (115200 baud, FIFO) and PS/2 Set 1 scancode translation.

---

## Partially Implemented / Deferred Within v1.x
- **Userspace Driver Isolation**: `IOPort` and `Irq` capabilities exist, but in V1.1.1 the active VirtIO-Blk driver and JanusFS execute in Ring 0 rather than as isolated Ring 3 services.
- **Bootloader Signature Verification**: Bootloader currently performs a placeholder `[MOCK PASS]` check rather than cryptographic verification.
- **Complete Ambient-Authority Elimination**: Diagnostic syscalls (`SYS_GET_TIME=23`, `SYS_TASK_STATUS=32`, `SYS_LOG=8`, `SYS_SLEEP=19`) remain un-gated by capabilities.

---

## Planned: v2.x (Userspace Services, Dynamic Init & Driver Recovery)
- [ ] **Manifest-Based `init` Service**: Implement static manifest parsing (`docs/INIT-POLICY.md`) to replace hardcoded bootstrap CNode setup.
- [ ] **Ring 3 VirtIO-Blk & VFS Service**: Move VirtIO-Blk and filesystem services out of Ring 0 into isolated Ring 3 tasks using async IRQ11 completion.
- [ ] **Independent Driver Restartability**: Re-bind hardware IRQs and restart crashed Ring 3 drivers without rebooting the kernel.
- [ ] **Crash-Consistent Filesystem**: Add journaling or copy-on-write for power-loss atomicity and lift the 2,560-byte max file size limit.

---

## Deferred / Long-Term: v3.x+ (WASM, SMP & Hardware Expansion)
- [ ] **WASM Application Runtime**: Sandboxed Ahead-Of-Time (AOT) WebAssembly execution with capability-scoped host bindings.
- [ ] **x2APIC & Multi-Core (SMP)**: Per-CPU `swapgs` syscall stacks, IPIs, per-core runqueues, and synchronized capability/notification locking.
- [ ] **Modern VirtIO 1.x / NVMe & Cryptographic Verified Boot**: Modern PCI capability-list VirtIO, NVMe, and real cryptographic signature / TPM measured boot.
- [ ] **ARM64 / AArch64 Target Support**: Port bootloader and kernel paging to ARM64.
