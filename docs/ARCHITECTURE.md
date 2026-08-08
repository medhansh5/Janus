# Janus OS Architecture

Janus is a microkernel operating system built around the principle of least privilege. Its primary design constraint is security, prioritizing robustness over feature completeness or POSIX compatibility.

## High-Level Design

The system is strictly divided into three layers:
1. **The Microkernel**: A minimal, highly trusted layer written in Rust. It manages only scheduling, memory, Inter-Process Communication (IPC), capabilities, and IRQ delivery.
2. **Userspace Services**: Drivers, virtual filesystems (VFS), and network stacks run as isolated, native Rust applications.
3. **WASM Runtime & Apps**: Third-party applications run as WebAssembly modules within a capability-scoped runtime.

## 1. Boot Flow

Janus relies on a verified boot chain to establish a root of trust before the kernel even starts.

1. **UEFI Firmware**: Initializes hardware and loads the Janus custom UEFI bootloader.
2. **Bootloader (UEFI Stub)**: 
   - Validates the cryptographic signature of the kernel ELF binary. If verification fails, the system halts.
   - Collects the memory map and platform information.
   - Maps the kernel into memory (enforcing strict W^X permissions on the kernel's own pages).
   - Calls `ExitBootServices` and hands off execution to the kernel entry point.
3. **Kernel Initialization**:
   - Initializes the architecture-specific physical memory allocator.
   - Sets up the root Page Table.
   - Initializes the root capability node (CNode).
   - Spawns the initial userspace service (the `init` process) and grants it the necessary starting capabilities.

## 2. Capability Model

Janus uses a strict **kernel-managed capability model** inspired by seL4.
- **No Ambient Authority**: Janus leverages hardware-enforced capabilities for all resource access. Instead of the kernel implicitly authorizing actions based on user IDs, tasks must explicitly invoke a capability slot in their CNode.

- **Endpoint**: Enables synchronous, unbuffered IPC between two specific tasks.
- **Untyped**: Represents raw, unallocated physical memory. It can be minted into typed memory (like CNode or Frame) or further split.
- **CNode**: Represents a capability table, mapping slots to other capabilities.
- **Frame**: Represents a typed 4KB page of physical memory that can be mapped into a task's address space for data or DMA.
- **IOPort**: (x86 specific) Grants access to a specific range of I/O ports. Crucial for hardware driver isolation (e.g., scoping PCI access strictly to a device's Base Address Registers).
- **InterruptEndpoint**: A specialized endpoint that hardware IRQs signal. A driver task waits on this endpoint via `sys_recv`, and the kernel wakes the task when the corresponding hardware interrupt fires.

## Capability Usage

1. **Memory Isolation**: To allocate memory, a task must possess an `Untyped` capability, invoke it to mint a `Frame`, and then instruct the kernel to map that `Frame` into its address space.
2. **Device Drivers**: Drivers run strictly in unprivileged Ring 3 userspace. The kernel (or highly trusted `init` task) enumerates the PCI bus, discovers devices, and grants the driver task `IOPort` capabilities scoped exclusively to the device's BARs, `Frame` capabilities for DMA buffers, and an `InterruptEndpoint` for device IRQs.
3. **IPC**: To communicate, tasks must be explicitly granted an `Endpoint` capability pointing to each other.

### Revocation Semantics
Capability revocation in Janus is instantaneous and cascading.
- **Memory Capabilities**: If a shared memory capability is revoked, the kernel immediately unmaps those physical pages from the page table of any process currently holding that derived capability. A fault will occur if the process attempts to access it mid-operation.
- **IPC Endpoints**: If an IPC endpoint capability is revoked, any pending blocked sends or receives on that endpoint immediately fail and return a `Revoked` error code to the caller.
- **Cascading Deletion**: Revoking a parent capability automatically searches the capability derivation tree and destroys all children capabilities. To prevent kernel-side Denial of Service (DoS) or stack overflows, this cascading revocation is implemented as an **iterative, depth-bounded operation** rather than unbounded recursion. If a derivation chain exceeds the maximum depth, derivation is blocked at creation time.

## 3. IPC Design

Inter-Process Communication is the sole method for isolated services to interact.
- **Synchronous IPC**: Used for blocking RPC-style calls. The scheduler performs a direct thread handoff from the sender to the receiver, minimizing context switch overhead.
- **Capability Passing**: IPC messages can contain capability pointers (`CPtr`), allowing one process to explicitly grant authority to another process dynamically.
- **Endpoints**: Processes communicate over unforgeable "Endpoint" capabilities. A process can only send a message to a service if it holds a valid Endpoint capability pointing to that service.

### Bulk Data Transfer
Because synchronous IPC relies on passing small amounts of data in CPU registers (typically a few machine words), it is unsuitable for bulk data (e.g., a large file read from the VFS). 
For bulk transfers, Janus uses **Shared Memory Capabilities**:
- The sender derives a `SharedMemory` capability mapping a specific physical frame.
- The sender passes this capability to the receiver via IPC.
- Both processes can now read/write the memory region directly. Once the transfer is complete, the capability can be revoked by the sender.

### Interrupt & IRQ Handling
Hardware interrupts are translated by the kernel into **Asynchronous IPC Messages**.
- Drivers do not register interrupt handlers in the kernel.
- Instead, a driver process blocks (waits) on a dedicated `InterruptEndpoint` capability.
- When the hardware raises an IRQ, the kernel intercepts it, acknowledges the hardware controller, and sends an empty, non-blocking IPC message to that specific `InterruptEndpoint`.
- The driver process is unblocked, handles the device state, and loops back to wait on the endpoint again.

## 4. Memory Model

- **W^X Enforcement**: Every memory page is strictly marked as either Writable (W) or Executable (X), but never both. This applies to both kernel and userspace memory, fundamentally mitigating code injection attacks.
- **ASLR**: Address Space Layout Randomization is enforced for the kernel itself (KASLR) and for all userspace services.
- **Isolated Page Tables**: Every userspace process has a distinct, isolated page table. The kernel maps only what is absolutely necessary into each address space.

## 5. Application Model & WASM

Third-party applications do not run as native binaries. They are distributed as WebAssembly (WASM) modules.
- **v0/v1 Approach**: Applications run inside `wasmi` (a WebAssembly interpreter).
- **v1+ Approach (AOT)**: To reduce runtime attack surface and eliminate the interpreter overhead, WASM modules will be validated and compiled Ahead-Of-Time (AOT) into native code at install time. 

> [!TIP]
> The use of `wasmi` in v0 is an accepted intermediate step to prove the capability model and end-to-end boot flow. Long-term, AOT compilation is the intended design for Janus.

## 6. Trust Boundaries

1. **Hardware / UEFI**: Implicitly trusted to boot the bootloader.
2. **Bootloader**: Trusted to verify the kernel signature.
3. **Microkernel**: Fully trusted. The sole arbiter of security and isolation.
4. **Init Process**: **Highly Trusted**. Because the `init` process holds the root capabilities (memory, hardware access) and decides how to partition and grant them to the VFS, Drivers, and WASM Runtime, a compromise of `init` is equivalent to a full system compromise. Its logic must be kept minimal and rigorously audited.
   - *Open Design Question (Pre-v1)*: What mechanism decides which capabilities `init` grants to a given WASM app? Will it be a static manifest file, an interactive user prompt, or a policy engine? This requires a dedicated design pass before v1, though v0 will hardcode a fixed grant for test apps.
5. **Core Services (VFS, Drivers)**: Trusted to perform their specific function correctly, but *isolated* from each other. If a network driver is compromised, it cannot write to the disk because it lacks the disk capability.
6. **WASM Apps**: Fully **untrusted**. Must be explicitly granted fine-grained capabilities by the user or the `init` system.
