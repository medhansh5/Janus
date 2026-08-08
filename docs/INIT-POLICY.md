# Init Capability-Grant Policy Design

## Overview
In Janus, `init` is the first userspace process spawned by the microkernel. It is Highly Trusted and receives the "root" capabilities for all physical memory, hardware IO, and IRQs. The primary responsibility of `init` is to divide these root capabilities, derive lesser privileges from them, and spawn the core system services (VFS, drivers, WASM runtime) with exactly the capabilities they need.

This document outlines the design for how `init` determines *which* capabilities to grant to each spawned process.

## Evaluated Options

### 1. Hardcoded Table
- **Description**: The `init` process contains a hardcoded array in Rust defining every service and its required capabilities.
- **Pros**: Extremely simple, no parser required, zero runtime overhead.
- **Cons**: Rigid, requires recompiling `init` to add or change a service, completely unscalable for third-party WASM applications.
- **Verdict**: Viable only for v0/v1 prototypes; fundamentally unsuited for a general-purpose OS.

### 2. Policy Engine (e.g., rule-based logic)
- **Description**: A dynamic rules engine (similar to SELinux or AppArmor) runs inside `init` evaluating complex policies based on context (e.g., "allow X if Y holds").
- **Pros**: Highly flexible, supports dynamic and context-aware capability granting.
- **Cons**: Substantially increases the complexity of `init`. A bug in the policy engine could lead to unauthorized capability escalation. Increases the Trusted Computing Base (TCB) significantly.
- **Verdict**: Overkill for the rigid microkernel isolation model we are building, and too risky for the TCB.

### 3. Static Manifest
- **Description**: A declarative manifest (e.g., JSON, TOML, or a simple custom binary format) packaged with the boot image. `init` parses this manifest at startup. The manifest explicitly lists the binary to load and the exact list of required capabilities.
- **Pros**: Deterministic, easily auditable by reviewing a static file, and keeps `init`'s logic straightforward (parse manifest -> spawn process -> grant caps). 
- **Cons**: Requires a parser inside `init`, which introduces parsing vulnerabilities into the TCB.
- **Verdict**: **Recommended**. It provides the necessary flexibility for a general-purpose OS while keeping the trust boundary rigid and reviewable.

## Recommendation for v1: Static Manifest

For v1, we will adopt the **Static Manifest** approach. The manifest acts as the definitive source of truth for the system's security topology. 

### Manifest Format & Loading
To minimize parser complexity and eliminate dependencies like `serde` or `toml` from the highly trusted `init` binary, the manifest should use a simple, flat structure. 

Initially, the manifest will be a plaintext line-based format or a flat binary structure embedded directly into the initial RAM disk (initrd/initfs) alongside the service binaries.

**Example Conceptual Structure (Text representation):**
```text
[service: vfs]
binary = /boot/vfs.elf
cap = MEMORY: 0x10000000 - 0x20000000, RW
cap = IPC_ENDPOINT: block_driver_ep, SEND

[service: block_driver]
binary = /boot/nvme.elf
cap = IO_PORT: 0x1F0 - 0x1F7
cap = IRQ: 14
cap = IPC_ENDPOINT: block_driver_ep, RECV
```

### Capability Referencing
Capabilities in the manifest will be referenced by symbolic names (e.g., `block_driver_ep`) during parsing. `init` will maintain a temporary lookup table to link these symbolic names to actual generated `CPtr` values, ensuring that when `vfs` receives a SEND capability and `block_driver` receives a RECV capability, they are securely minted from the same underlying kernel endpoint object.

## Security Implications & Threat Model Updates

Because `init` relies on this manifest to distribute ambient authority across the entire system, the manifest and its parser are critical security boundaries.

1. **Manifest Integrity**: The manifest must be cryptographically verified alongside the kernel and `init` binary by the UEFI bootloader during Verified Boot. A tampered manifest allows full system compromise.
   > **Note**: As of v0/v1, this protection rides on the mock/placeholder signature verification from the bootloader. True cryptographic signing is deferred to a future milestone.
2. **Parser Vulnerabilities**: The manifest parser executes inside the Highly Trusted `init` process. Memory corruption (e.g., buffer overflows) or logic bugs during parsing could allow a maliciously crafted manifest (if the boot chain was bypassed) to escalate privileges. 
3. **Mitigation**: The parser will be written in safe Rust, intentionally avoiding complex deserialization crates. It will enforce strict bounds checking and fail-closed semantics on any malformed input. This parser explicitly enters the Trusted Computing Base (TCB).
