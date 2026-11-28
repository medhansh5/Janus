# Janus OS Release & Branching Strategy

This document defines the versioning scheme, branching workflow, and milestone history for `medhansh5/Janus`.

## Semantic Versioning

Janus OS follows [Semantic Versioning (SemVer 2.0.0)](https://semver.org/):
- **MAJOR (`vX.0.0`)**: Breaking syscall/capability ABI changes or major architectural shifts (e.g., SMP migration).
- **MINOR (`v1.X.0`)**: Backward-compatible kernel capabilities, drivers, or service additions.
- **PATCH (`v1.X.Y`)**: Hardening, bug fixes, test coverage, and documentation updates.

---

## Git Workflow

- **`main` Branch**: Primary development branch tested by GitHub Actions CI (`.github/workflows/ci.yml`: formatting, host unit tests, Clippy, and bare-metal/UEFI builds).
- **Feature / Fix Branches**: `feature/<description>`, `fix/<description>`, `docs/<topic>` may be used for larger changes; milestone commits have also been landed directly on `main` when authorized.

---

## Release Tagging & Milestones

Milestones are marked with annotated Git tags (`vMAJOR.MINOR.PATCH`). Below is the distinction between **published Git tags** and **completed engineering milestones**:

### Published Git Tags
- **`v1.0.0`**: Initial V1 milestone tag (microkernel core, GDT/IDT/PIC, synchronous IPC, scoped `IOPort` / `Frame` primitives).
- **`v0.1.0`**: Initial prototype tag (Rust microkernel boot, page tables, basic IPC).

### Completed Engineering Milestones (Pending Formal Tag / GitHub Release)
- **`V1.1.1` (Commit `680aa6102dd8f30dab1480716df18c0734116577`, not yet tagged `v1.1.1`):**
  - Hardening milestone adding direct capability negative tests for syscalls 24–30 (122 unit tests passing), transactional JanusFS allocation/write-error rollback, `CapType::Irq` ownership cleanup on task/CSpace destruction, and aligned architecture/limitations documentation.
- **`V1.1` (Local/untagged milestone merged into `680aa61`):**
  - Added legacy VirtIO 0.9.5 block driver, persistent JanusFS across QEMU reboots, `CapType::Console` / `CapType::Storage` gating on syscalls 24–30, single-EOI PIC/IDT protocol, `CapRights::WRITE` check on `sys_irq_ack`, and `USER_RSP_SCRATCH` zeroing in `syscall_entry`.
- **`Stage 3.2A` & `Full Usable V1` (Included in `680aa61`):**
  - Added 64-object `Notification` pool, COM1/PS2 console queue, and Ring 3 interactive shell (`Task A`).
