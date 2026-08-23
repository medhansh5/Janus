# Contributing to Janus OS

Thank you for your interest in contributing to Janus OS. Janus is a security-first microkernel operating system written in Rust, built around kernel-enforced capability isolation and zero ambient authority.

## Guidelines and Philosophy

1. **Zero Ambient Authority**: All resource access in userspace requires an explicit, unforgeable capability (`CPtr`). Code contributions must preserve capability isolation.
2. **Minimal Trusted Computing Base (TCB)**: Keep kernel space strictly limited to task scheduling, physical frame mapping, IPC routing, and IRQ delivery. All drivers and system services run in Ring 3 userspace.
3. **Safety and Soundness**: Any use of `unsafe` Rust must be strictly justified with an explicit `// SAFETY:` comment documenting memory and hardware invariants.

## Getting Started

### Prerequisites
- **Rust Toolchain**: Nightly toolchain specified in `rust-toolchain.toml` with `rust-src`, `llvm-tools-preview`, `rustfmt`, and `clippy`.
- **Target Triples**:
  - `x86_64-unknown-none` (Kernel target)
  - `x86_64-unknown-uefi` (Bootloader target)
- **QEMU & OVMF Firmware**:
  - `qemu-system-x86_64`
  - OVMF EFI firmware (`/usr/share/ovmf/OVMF.fd` on Debian/Ubuntu, or specified via `OVMF` variable in Makefile)

### Local Build and Verification

Before submitting a pull request, run the following verification steps:

```bash
# Check code formatting
cargo fmt --all -- --check

# Run host unit tests
cargo test -p boot-info
cargo test -p kernel --lib

# Build kernel and bootloader targets
make all

# Run system in QEMU
make run
```

## Pull Request Workflow

1. **Branch Naming**: Use descriptive branch names:
   - `feature/<short-description>` for new capabilities or kernel features.
   - `fix/<issue-number>-<short-description>` for bug fixes.
   - `docs/<topic>` for documentation updates.
2. **Commit Messages**: Write clear, imperative commit titles:
   - `kernel: implement iterative CNode capability revocation`
   - `bootloader: parse memory map descriptors for reserved regions`
3. **Continuous Integration**: Every PR must pass automated CI checks in `.github/workflows/ci.yml`.

## Unsafe Code Standards

When introducing an `unsafe` block in the kernel or bootloader:
- Keep the block as small as possible.
- Provide a `// SAFETY:` header directly above the block.
- Verify pointer alignment, lifetime invariants, and volatile memory access rules.

```rust
// SAFETY: The physical memory region `paddr` is guaranteed to be a valid, unmapped
// 4KB frame returned by the physical memory allocator and aligned to 4096 bytes.
unsafe {
    core::ptr::write_bytes(vaddr.as_mut_ptr(), 0, 4096);
}
```

## Licensing

All contributions to Janus OS are submitted under the project's dual MIT and Apache-2.0 license terms (see [LICENSE](LICENSE)).
