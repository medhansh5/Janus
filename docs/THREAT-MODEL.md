# Janus Threat Model

This document outlines the threat model for the Janus operating system. It defines the attackers we defend against, the specific threats we mitigate, and explicitly states what is out of scope for the v1 release.

## 1. What Janus Defends Against

The core philosophy of Janus is that **compromise is inevitable, but lateral movement should be impossible.**

### 1.1 Malicious Userspace Applications
- **Threat**: An application intentionally attempts to access arbitrary files, sniff network traffic, or escalate privileges.
- **Defense**: Applications run in a WebAssembly sandbox (and native services are capability-isolated). Applications have zero ambient authority. They can only access resources they have explicitly been granted via `CPtr` handles.
- **Mitigation**: A malicious app cannot read `/etc/shadow` because there is no `/etc/shadow` in its namespace, nor does it possess a capability to ask the VFS for it.

### 1.2 Compromised Drivers and Services
- **Threat**: A vulnerability in a complex parser (e.g., the TCP/IP stack) or a hardware driver (e.g., NVMe driver) is exploited by a remote attacker.
- **Defense**: The microkernel architecture ensures that drivers and network stacks run as unprivileged userspace processes. 
- **Mitigation**: If the network stack is compromised via a malformed packet, the attacker gains code execution *only* within the isolated network service. They cannot write to the disk or alter memory belonging to other processes, because the network service does not hold capabilities for those resources.

### 1.3 Memory Corruption Attacks
- **Threat**: Attackers utilize buffer overflows, Use-After-Free (UAF), or ROP chains to execute arbitrary code.
- **Defense**: 
  1. The kernel and core services are written in Rust, eliminating spatial and temporal memory safety bugs by default (except in heavily audited `unsafe` blocks).
  2. Strict W^X (Write XOR Execute) memory page permissions prevent the execution of injected shellcode.
  3. ASLR prevents the reliable chaining of ROP gadgets.

### 1.4 Unauthorized Kernel Modification
- **Threat**: An attacker attempts to modify the kernel on disk to establish a persistent rootkit.
- **Defense**: Verified Boot. The bootloader cryptographically verifies the signature of the kernel before handoff. An unauthorized kernel will simply refuse to boot.

---

## 2. Explicitly Out of Scope for v0 / v1

The following threats are acknowledged but deferred to later iterations of Janus:

### 2.1 Side-Channel Attacks (e.g., Spectre, Meltdown)
- While Janus isolates memory, preventing advanced microarchitectural side-channel attacks is extremely difficult and out of scope for the initial versions. We do not guarantee protection against an application inferring data from another via cache timing.

### 2.2 Physical Access & Evil Maid Attacks
- If an attacker has physical access to the machine, they can modify the UEFI firmware or extract data directly from the unencrypted disk. Full Disk Encryption (FDE) and TPM-backed measured boot are deferred to v2.

### 2.3 Denial of Service (DoS) and Resource Starvation
- As the kernel introduces a preemptive scheduler, resource-starvation attacks (e.g., a malicious task intentionally exhausting its timeslice or flooding IPC channels) become possible. Fine-grained resource quotas, CPU time accounting, and real-time scheduling guarantees are not implemented in v1. Until these defenses are built, local DoS remains an accepted risk.

### 2.4 Multi-Core Support and APIC Migration (Deferred to v1+)
- For the v0 milestone, the kernel targets a uniprocessor environment and uses the legacy 8259 PIC for timer interrupts. The PIC is insufficient for multi-core scaling (as it lacks Inter-Processor Interrupt (IPI) support). Multi-core support is deferred to v1+, which will require migrating the interrupt subsystem to the Advanced Programmable Interrupt Controller (APIC) and implementing concurrent capability management.

### 2.5 Interpreter vs. AOT Tradeoff (v0 Accepted Risk)

> [!WARNING]
> **Accepted Risk for v0:** In the v0 milestone, Janus relies on `wasmi` (a WebAssembly interpreter) to execute applications.
> 
> Running an interpreter over untrusted code inside a privileged runtime environment inherently increases the attack surface compared to Ahead-Of-Time (AOT) compiled native code. A bug in the interpreter could allow an application to break out of the WASM sandbox and execute native code with the privileges of the WASM host process.
> 
> **Resolution Plan:** This is an accepted risk strictly for v0 to prove the end-to-end OS pipeline. Before any untrusted third-party application is loaded outside of a dev/test environment (v1+), the architecture will transition to AOT compilation (validating and compiling WASM to native ELF at install time).

### 2.5 Audit & Provenance (Deferred to v1 GA)
- Capability grant/revoke logging is deferred. While not present in v0, this is acknowledged as a hard requirement for forensic analysis and system auditing, and will be designed and implemented before v1 GA.

## 3. Component Trust Levels

1. **Kernel (TCB)**: Highly Trusted. Responsible only for IPC routing, context switching, capability validation, and memory isolation.
2. **Init Task**: Highly Trusted. The first userspace task is responsible for allocating initial capabilities (parsing `INIT-POLICY.md`) and spawning service tasks.
3. **Drivers (e.g., virtio-blk)**: Untrusted. Hardware drivers run entirely in Ring 3 userspace. They cannot arbitrarily access PCI space or system memory. The kernel discovers PCI devices and mints strictly scoped `IOPort` (for the device's BARs), `Frame` (for DMA), and `InterruptEndpoint` capabilities to the driver. If a driver crashes or goes rogue, it can only affect its assigned device and clients; it cannot breach kernel memory or other devices.
4. **WASM Runtime / Web Applications**: Untrusted. Will run sandboxed within isolated userspace tasks.
5. **System Services**: Moderately Trusted. VFS or Network stack run as isolated tasks and must be robust against malicious inputs from unprivileged tasks, but their internal compromise doesn't necessarily breach the kernel.
6. **Supply-Chain Risk**: We acknowledge the dependency supply-chain risk introduced by third-party packages (crates.io). To mitigate this, Janus commits to using minimal dependencies in kernel/service code, utilizing pinned lockfiles, and running periodic `cargo-audit` checks.
