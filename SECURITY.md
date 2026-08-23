# Security Policy & Security Model

## Supported Versions

| Version | Supported | Security Maintenance |
| ------- | --------- | -------------------- |
| 1.0.x   | Yes       | Active maintenance   |
| 0.1.x   | No        | End of life          |

## Security Model & Architecture Guarantees

Janus OS is built around the principle that system compromise must be contained locally, preventing lateral movement across processes or kernel memory.

### 1. Capability-Based Access Control
- **Zero Ambient Authority**: Processes possess no implicit permissions based on process ID or user ownership.
- **Kernel-Managed Handles**: Capabilities are stored exclusively in kernel-managed CNode tables. Userspace references capabilities via opaque integer handles (`CPtr`).
- **Granular Privilege Delegation**: Processes can only perform operations (memory mapping, IPC, port I/O, IRQ handling) for which they hold an explicit `CPtr`.

### 2. Memory Isolation & Hardware Enforcement
- **Page Table Isolation**: Each userspace task executes in an independent page table hierarchy.
- **W^X Enforcement**: Pages are strictly configured as either Writable (W) or Executable (X), but never both.
- **Contiguous DMA Scoping**: Hardware drivers receive DMA buffers as typed `Frame` capabilities. Drivers cannot perform DMA outside explicitly allocated physical frames.

### 3. Drivers and I/O Boundaries
- **Ring 3 Execution**: Hardware drivers (such as Virtio-Blk) execute in unprivileged Ring 3 userspace.
- **Scoped Port I/O**: `IOPort` capabilities restrict driver port access strictly to assigned PCI Base Address Registers (BARs).
- **Interrupt Binding**: Hardware interrupts are transformed into asynchronous messages sent to specific `InterruptEndpoint` capabilities. Unassigned IRQs are masked at the interrupt controller level.

---

## Reporting Security Vulnerabilities

We take the security of Janus OS seriously. If you discover a security flaw, vulnerability, or capability bypass, please notify the security team before public disclosure.

### How to Report
- **Email**: Send security reports to `security@ashcrest.org`.
- **GPG Key**: Available upon request for encrypted communications.

### What to Include in Your Report
- Summary of the vulnerability and its potential impact.
- Step-by-step reproduction instructions or code demonstration.
- Relevant component (e.g., Microkernel capability table, syscall dispatcher, bootloader signature verification).

### Response Expectations
- **Initial Acknowledgment**: Within 48 hours of receipt.
- **Triage & Severity Assessment**: Within 5 business days.
- **Patch & Disclosure Timeline**: Coordinate disclosure within 90 days of initial confirmation, or sooner upon release of a patched version.
