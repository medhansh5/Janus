# Task List: Virtio-Blk Driver

> **Historical Document Notice:** This checklist records early v0/v1 prototype planning for a userspace VirtIO-Blk driver (`InterruptEndpoint` / `SYS_LOG` Sector 0 proof). It does **not** reflect the current V1.1.1 implementation, where legacy VirtIO-Blk and JanusFS run in Ring 0 and IRQs use `CapType::Irq` + `Notification`. For current documentation, see [README.md](../README.md), [ARCHITECTURE.md](ARCHITECTURE.md), [LIMITATIONS.md](LIMITATIONS.md), and [V1_1_1_RELEASE_READINESS.md](architecture/V1_1_1_RELEASE_READINESS.md).

## Foundation
- `[x]` **Design Virtio Capabilities**: Map the required capabilities (IOPort, Frame, InterruptEndpoint) to isolate driver access.
- `[x]` **Capability Provisioning**: Kernel explicitly hands scoped capabilities (like specific IOPort ranges for PCI) to the untrusted driver process.
- `[x]` **Syscall Additions**:
    - `[x]` Implement `SYS_CAP_INFO` to resolve `Frame` capabilities to physical addresses.
    - `[x]` Implement `SYS_IN_PORT` and `SYS_OUT_PORT` to enforce PCI access.
    - `[x]` Add `SYS_LOG` for output testing.
- `[x]` **Security Enforcement Check**: Write a test where the driver intentionally requests an unowned `Frame` CPtr via `SYS_CAP_INFO` and ensure the kernel faults it securely.

## Kernel Initialization (init stand-in)
- `[x]` **Driver Initialization (Userspace)**:
    - `[x]` Read device registers via scoped `IOPort` capability.
    - `[x]` Negotiate virtio features and setup Virtqueues mapped by the kernel into userspace memory.

## Driver & Client Payloads
- `[x]` **Sector Read Validation**: 
    - `[x]` Successfully send read command.
    - `[x]` Wait on `InterruptEndpoint` capability for IRQ.
    - `[x]` Print completion and magic value retrieved from Sector 0 to prove success.

## Testing & Verification
- `[x]` Create `test.img` with a known signature in Sector 0.
- `[x]` Update `Makefile` to launch QEMU with the virtio drive.
- `[x]` End-to-End Test: verify `SYS_LOG` outputs the exact Sector 0 signature.
