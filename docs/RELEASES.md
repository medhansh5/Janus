# Janus OS Release & Branching Strategy

This document defines the versioning scheme, git branching policy, release tagging conventions, and milestone lifecycle for Janus OS.

## Semantic Versioning

Janus OS follows [Semantic Versioning (SemVer 2.0.0)](https://semver.org/):
- **MAJOR (`vX.0.0`)**: Significant architectural shifts (e.g., capability ABI breaking changes, microkernel syscall modification, multi-core architecture transition).
- **MINOR (`v1.X.0`)**: New kernel capabilities, driver implementations, hardware target support, or userland service APIs added in a backward-compatible manner.
- **PATCH (`v1.0.X`)**: Backward-compatible bug fixes, security patches, performance optimizations, or documentation enhancements.

---

## Git Branching Strategy

```text
  main (stable, production-ready trunk)
   │
   ├─── feature/virtio-blk-queue  ─── (PR review + CI checks) ───> Merge to main
   │
   ├─── fix/cnode-depth-check     ─── (PR review + CI checks) ───> Merge to main
   │
   └─── Tagged Milestones (e.g., `v1.0.0`, `v0.1.0`)
```

### Branch Rules
1. **`main` Branch**:
   - The primary development and stable release branch.
   - All commits on `main` must pass automated CI checks (`.github/workflows/ci.yml`), including formatting, clippy lints, host tests, and bare-metal target builds.
   - Direct pushes to `main` are restricted; code changes land via reviewed Pull Requests.
2. **Feature & Fix Branches**:
   - `feature/<description>`: New functionality, capabilities, or drivers.
   - `fix/<issue-number>-<description>`: Bug fixes or security patches.
   - `docs/<topic>`: Documentation, architecture specs, or manual updates.

---

## Release Tagging & Milestones

Milestones and release builds are marked in git using signed annotated tags matching the format `vMAJOR.MINOR.PATCH`:

```bash
# Example: Creating an annotated milestone tag
git tag -a v1.0.0 -m "Janus v1.0 Milestone: Microkernel Core & Virtio-Blk Ring 3 Driver"
```

### Historical Release Milestones
- **`v1.0.0`**: First functional milestone demonstrating Ring 3 Virtio-Blk storage driver execution, `InterruptEndpoint` IRQ delivery, scoped PCI `IOPort` capabilities, preemptive IRQ0 scheduler, and contiguous DMA memory allocation.
- **`v0.1.0`**: Initial prototype establishing Rust microkernel setup, page table management, and synchronous IPC handoff.
