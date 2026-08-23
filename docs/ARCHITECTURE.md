# Janus OS Architecture

Janus is a microkernel operating system built around kernel-enforced capabilities and zero ambient authority. Its primary design constraint is security, prioritizing robustness and isolation over legacy POSIX compatibility.

## System Architecture Diagram

```text
+-----------------------------------------------------------------------+
|                             USERSPACE (Ring 3)                         |
|                                                                       |
|  +--------------------+   +-------------------+   +----------------+  |
|  | WASM Application   |   | Virtio-Blk Driver |   |   Init Task    |  |
|  | (Sandboxed App)    |   | (Storage Service) |   | (Cap Root/Boot)|  |
|  +---------+----------+   +---------+---------+   +-------+--------+  |
|            |                        |                     |           |
|            | CPtr Handle            | CPtr / IOPort       | Manifest  |
|            v                        v                     v           |
|=======================================================================|
|                   SYSTEM CALL & CAPABILITY BOUNDARY                   |
|                        `syscall` / `sysret` ABI                        |
|=======================================================================|
|                             MICROKERNEL (Ring 0)                      |
|                                                                       |
|  +------------------+  +-------------------+  +--------------------+  |
|  | CSpace & Cap     |  | Preemptive        |  |  Physical Frame    |  |
|  | Table Validation |  | Scheduler & IPC   |  |  & Page Table Alloc|  |
|  +------------------+  +-------------------+  +--------------------+  |
|                                                                       |
|  +-----------------------------------------------------------------+  |
|  | Hardware IRQ Routing & InterruptEndpoint Dispatch (IRQ0 Timer)  |  |
|  +-----------------------------------------------------------------+  |
+-----------------------------------------------------------------------+
```

## High-Level Design

The system is strictly divided into three layers:
1. **The Microkernel**: A minimal, highly trusted layer written in Rust. It manages only scheduling, page tables, capability validation, Inter-Process Communication (IPC), and hardware IRQ delivery.
2. **Userspace Services**: Drivers (e.g., Virtio-Blk), virtual filesystems (VFS), and network stacks run as isolated userspace processes in Ring 3.
3. **WASM Runtime & Applications**: Third-party applications execute within capability-scoped WebAssembly runtimes.

---

## 1. Kernel / Userspace System Call Boundary

Janus leverages x86_64 hardware system call instructions (`syscall` / `sysret`) for fast ring transitions between Ring 3 userspace and Ring 0 microkernel mode.

### Syscall Register ABI

| Register | Direction | Description |
| -------- | --------- | ----------- |
| `rax`    | Input     | System Call Identifier (`SYS_SEND`, `SYS_RECV`, `SYS_CALL`, `SYS_YIELD`) |
| `rdi`    | Input     | Target Capability Pointer handle (`CPtr`) |
| `rsi`    | Input     | IPC Message payload word 0 / transfer flag |
| `rdx`    | Input     | IPC Message payload word 1 |
| `r10`    | Input     | IPC Message payload word 2 |
| `r8`     | Input     | Capability grant handle (optional `CPtr` to pass) |
| `rax`    | Output    | System call result status / error code |
| `rcx`    | Preserved | Saved user instruction pointer (`rip` restored by `sysret`) |
| `r11`    | Preserved | Saved CPU flags (`rflags` restored by `sysret`) |

### Isolation Rules
- **No Direct Memory Sharing**: Userspace cannot access kernel data structures directly. Memory addresses passed across syscall boundaries must be backed by a valid, mapped `Frame` capability.
- **CPtr Validation**: Every syscall operation checks the calling task's capability table (`CSpace`). If the specified `CPtr` is invalid, unowned, or lacks permissions for the requested operation, the kernel immediately aborts the syscall, returning a `CapabilityFault` status or terminating the process if unsafe.

---

## 2. Boot Flow

Janus relies on a verified boot chain to establish a root of trust before the kernel starts.

1. **UEFI Firmware**: Initializes hardware platforms and hands control to the Janus custom UEFI bootloader stub.
2. **Bootloader (UEFI Stub)**: 
   - Validates the cryptographic signature of the kernel ELF binary. If verification fails, execution halts.
   - Obtains EFI physical memory map descriptors and platform metadata.
   - Maps kernel sections into memory with W^X page protection (code pages read-only executable, data pages read-write non-executable).
   - Invokes `ExitBootServices` and jumps to the kernel entry point.
3. **Kernel Initialization**:
   - Initializes physical frame memory allocators while respecting reserved bootloader memory regions.
   - Constructs root Page Table structures and architectural Interrupt Descriptor Tables (IDT).
   - Instantiates the initial capability root node (`CNode`).
   - Spawns the initial userspace service (`init`) in Ring 3 and grants starting root capabilities.

---

## 3. Capability Model

Janus implements a kernel-managed **object-capability model** inspired by seL4.
- **No Ambient Authority**: System resources (memory frames, IPC endpoints, I/O ports, interrupt lines) require an explicit capability handle in the process's `CSpace`.
- **Capability Types**:
  - `Endpoint`: Synchronous, unbuffered IPC communication channel.
  - `Untyped`: Unallocated physical memory frame range, mintable into typed capabilities.
  - `CNode`: Capability table storing slot mappings for a process.
  - `Frame`: Physical 4KB memory page for address space mapping or hardware DMA.
  - `IOPort`: x86 I/O port range access bitmask (scoped strictly to device BARs).
  - `InterruptEndpoint`: Specialized endpoint signaled directly by hardware device IRQs.

### Capability Revocation Semantics
Revocation in Janus is instantaneous and cascading:
- **Memory Capabilities**: Revoking a `Frame` capability immediately unmaps corresponding physical pages from all recipient process address space page tables.
- **IPC Endpoints**: Revoking an `Endpoint` capability immediately unblocks waiting processes on that endpoint with a `Revoked` status code.
- **Depth-Bounded Deletion**: Capability derivation trees are traversed iteratively during deletion to prevent kernel stack overflow.

---

## 4. Drivers and IRQ Routing

Hardware drivers execute entirely in unprivileged Ring 3 userspace:
- **PCI Isolation**: Drivers receive `IOPort` capabilities limited strictly to their device's Base Address Registers (BARs).
- **Contiguous DMA**: The kernel provisions contiguous physical memory frames for DMA buffers via typed `Frame` capabilities.
- **Interrupt Handling**: When a hardware interrupt fires (e.g., IRQ0 timer or Virtio-Blk controller), the kernel acknowledges the interrupt controller and dispatches an asynchronous event message to the registered `InterruptEndpoint`. The driver task waiting in userspace receives the signal and processes the I/O request queue.
