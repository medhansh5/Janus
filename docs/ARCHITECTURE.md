# Janus OS Architecture

Janus is a microkernel operating system built around kernel-enforced capabilities and zero ambient authority. Its primary design constraint is security, prioritizing robustness and isolation over legacy POSIX compatibility.

## System Architecture Diagram

```text
+-----------------------------------------------------------------------+
|                             USERSPACE (Ring 3)                         |
|                                                                       |
|  +----------------------------------+   +--------------------------+  |
|  | Task A: Interactive Shell        |   | Task B: IPC Responder    |  |
|  | (`help`, `ls`, `cat`, `write`,   |   | (Synchronous rendezvous  |  |
|  |  `rm`, `ps`, `time`, `test`, ...) |   |  IPC via Endpoint caps)  |  |
|  +----------------+-----------------+   +------------+-------------+  |
|                   |                                  |                |
|                   | `syscall` / `iretq` (CPtr in `rsi`)                |
|                   v                                  v                |
|=======================================================================|
|                   SYSTEM CALL & CAPABILITY BOUNDARY                   |
|=======================================================================|
|                             MICROKERNEL (Ring 0)                      |
|                                                                       |
|  +------------------+  +-------------------+  +--------------------+  |
|  | CSpace & CDT     |  | Preemptive IRQ0   |  | Physical Frame &   |  |
|  | Validation       |  | Scheduler & IPC   |  | Higher-Half Paging |  |
|  +------------------+  +-------------------+  +--------------------+  |
|  +------------------+  +-------------------+  +--------------------+  |
|  | Console (COM1 /  |  | JanusFS & Legacy  |  | 8259 PIC & Async   |  |
|  | PS/2) Service    |  | VirtIO-Blk Driver |  | Notification Pool  |  |
|  +------------------+  +-------------------+  +--------------------+  |
+-----------------------------------------------------------------------+
```

## High-Level Design

The current V1.1.1 implementation is structured into two active privilege rings (with isolated Ring 3 service drivers and WASM runtimes planned for future milestones):
1. **The Microkernel (Ring 0)**: Manages physical frame allocation, higher-half W^X paging, `CNode`/CDT capability validation, preemptive scheduling, synchronous IPC, asynchronous notifications, 8259 PIC IRQ routing, COM1/PS2 console I/O, the legacy VirtIO-Blk polling driver, and JanusFS.
2. **Userspace Tasks (Ring 3)**: Unprivileged tasks (`Task A` interactive shell, `Task B` IPC responder) isolated in per-task user address spaces and communicating via `syscall` / `iretq` using opaque `CPtr` handles.

---

## 1. Kernel / Userspace System Call Boundary

Janus leverages x86_64 hardware system call instructions (`syscall` / `sysret`) for fast ring transitions between Ring 3 userspace and Ring 0 microkernel mode.

### Syscall Register ABI (Actual V1.1 Implementation)

| Register | Direction | Description |
| -------- | --------- | ----------- |
| `rdi`    | Input     | System Call Number (`SYS_SEND=1`, ..., `SYS_READ_CONSOLE=24`..`SYS_FS_LIST=30`, `SYS_TASK_STATUS=32`) |
| `rsi`    | Input     | Argument 1 (Capability Pointer `CPtr` for capability-gated syscalls) |
| `rdx`    | Input     | Argument 2 |
| `r10`    | Input     | Argument 3 (moved to `rcx` by `syscall_entry` for `sysv64` ABI) |
| `r8`     | Input     | Argument 4 |
| `r9`     | Input     | Argument 5 |
| `rax`    | Output    | System call result or `!0` (`-1`) on error |
| `rcx`    | Clobbered | Hardware-saved user `RIP` on `syscall` entry |
| `r11`    | Clobbered | Hardware-saved user `RFLAGS` on `syscall` entry |

### Isolation Rules
- **No Direct Memory Sharing**: Userspace cannot access kernel data structures directly. Memory addresses passed across syscall boundaries must be backed by a valid, mapped `Frame` capability.
- **CPtr Validation**: Every syscall operation checks the calling task's capability table (`CSpace`). If the specified `CPtr` is invalid, unowned, or lacks permissions for the requested operation, the kernel immediately aborts the syscall, returning a `CapabilityFault` status or terminating the process if unsafe.

---

## 2. Boot Flow

Janus relies on a verified boot chain to establish a root of trust before the kernel starts.

1. **UEFI Firmware**: Initializes hardware platforms and hands control to the Janus custom UEFI bootloader stub.
2. **Bootloader (UEFI Stub)**: 
   - Loads `kernel.elf` via UEFI `SimpleFileSystem` (currently uses a mock kernel signature check).
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
  - `Notification`: Asynchronous 64-bit coalescing event bitmask object.
  - `Untyped`: Unallocated physical memory frame range.
  - `CNode`: Capability table storing 128 slots per task.
  - `Frame` / `MmioFrame`: Physical 4KB memory/MMIO page with W^X enforcement.
  - `IOPort`: x86 I/O port range access bitmask (scoped strictly to device BARs).
  - `Irq`: Hardware IRQ line (0..15) capability for binding and single-EOI acknowledgement.
  - `Console`: Capability gating COM1/PS2 console reads (`READ`) and writes (`WRITE`).
  - `Storage`: Capability gating JanusFS operations (`READ` / `WRITE`).

### Capability Revocation Semantics
Revocation in Janus is instantaneous and cascading:
- **Memory Capabilities**: Revoking a `Frame` capability immediately unmaps corresponding physical pages from all recipient process address space page tables.
- **IPC Endpoints**: Revoking an `Endpoint` capability immediately unblocks waiting processes on that endpoint with a `Revoked` status code.
- **Depth-Bounded Deletion**: Capability derivation trees are traversed iteratively during deletion to prevent kernel stack overflow.

---

## 4. Drivers and IRQ Routing

- **Legacy VirtIO-Blk & JanusFS**: In V1.1.1, the legacy VirtIO 0.9.5 block driver (`kernel/src/virtio_blk.rs`) and JanusFS (`kernel/src/fs.rs`) run in Ring 0 using synchronous polling over a 3-page contiguous VirtQueue (`q_size <= 256`).
- **PCI & I/O Port Primitives**: `CapType::IOPort` capabilities enforce port range and width bounds for `sys_in_port`/`sys_out_port`.
- **Interrupt Handling**: Hardware IRQs (vectors 33..47) filter spurious IRQ7/IRQ15, deliver events to bound `Notification` objects (`handle_hardware_irq`) or blocked `Irq` receivers (`wake_irq`), mask the firing line in the 8259 PIC, and issue a single EOI. Authorized drivers holding `CapType::Irq` with `CapRights::WRITE` unmask the line via `sys_irq_ack`. On last-owner destruction or revocation, `cleanup_irq_on_cap_removal` unbinds and unmasks orphaned lines.
