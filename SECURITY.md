# Security Policy & Security Model

## Supported Versions

| Version / Branch | Status | Security Maintenance |
| ---------------- | ------ | -------------------- |
| `main` (`V1.1.1` hardening baseline `680aa61`) | Active development | Current hardening target (untagged/unpublished release candidate) |
| `v1.0.0` | Historical tag | Superseded by V1.1.1 |
| `v0.1.0` | Historical tag | End of life |

> **Note:** Janus OS V1.1.1 is an experimental research microkernel targeting uniprocessor QEMU x86_64. It is not production-hardened and has not undergone cryptographic boot or SMP hardening.

---

## Security Model & Implemented Guarantees (V1.1.1)

### 1. Capability-Based Access Control
- **Kernel-Managed Handles**: Capabilities reside in kernel-managed 128-slot `CNode`s. Userspace references them via generation-tagged `CPtr` handles.
- **Capability-Gated Services**: Syscalls 24–30 (`SYS_READ_CONSOLE`..`SYS_FS_LIST`) enforce `CapType::Console` / `CapType::Storage` and `READ`/`WRITE` rights prior to user-buffer access or I/O.
- **Residual Diagnostic Syscalls**: `SYS_GET_TIME` (23), `SYS_TASK_STATUS` (32), `SYS_LOG` (8), and `SYS_SLEEP` (19) remain un-gated by capabilities.

### 2. Memory Isolation & Hardware Enforcement
- **Page Table Isolation**: Each userspace task runs in an isolated PML4 address space; `copy_from_user` / `copy_to_user` pre-validate bounds, canonicality, and `USER_ACCESSIBLE` flags.
- **W^X Enforcement**: Kernel ELF segments and user `Frame` / `MmioFrame` mappings forbid simultaneous `WRITE` and `EXECUTE` permissions. *(ASLR is not implemented.)*

### 3. I/O & Interrupt Boundaries
- **In-Kernel Storage & Console Services**: In V1.1.1, the legacy VirtIO 0.9.5 block driver and JanusFS execute in Ring 0; `CapType::IOPort` and `CapType::Irq` primitives are enforced at the syscall boundary for capability-bearing tasks.
- **Single-EOI IRQ Protocol**: Hardware IRQs mask the firing line in the 8259 PIC and issue one EOI; `SYS_IRQ_ACK` requires `CapType::Irq` with `CapRights::WRITE` to unmask, and orphaned IRQ lines are cleaned up when the last owner's CSpace is destroyed.

---

## Reporting Security Vulnerabilities

If you discover a security flaw, capability bypass, or kernel memory safety issue in `medhansh5/Janus`, please report it privately via **GitHub Security Advisories** on `https://github.com/medhansh5/Janus/security` (or by opening a minimal issue requesting a private contact channel if advisories are unavailable) before public disclosure.

### What to Include
- Summary of the vulnerability and affected subsystem (e.g., `capability.rs`, `paging.rs`, `syscall.rs`, `fs.rs`, `notification.rs`).
- Step-by-step reproduction steps or minimal unit/QEMU test case.
