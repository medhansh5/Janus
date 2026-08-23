# Janus OS Known Limitations

This document lists explicit technical limitations, constraints, and non-goals in the current release of Janus OS.

## System & Execution Constraints

### 1. Uniprocessor (Single-Core Execution)
- **Constraint**: Janus currently executes on a single x86_64 CPU core using the 8259 PIC / APIC single-core timer for IRQ0 preemptive scheduling.
- **Impact**: Multi-threading across multiple physical CPU cores is not supported in the v1 release. Multi-core execution via x2APIC and IPI scheduling is scheduled for v4.

### 2. Userland Application Interface
- **Constraint**: High-level C/Rust standard library support (`std`) is not available in Ring 3 userspace tasks.
- **Impact**: Current userspace tasks (including storage driver tests) execute using custom `no_std` entrypoints and direct assembly register invocations (`syscall` ABI). High-level application support relies on WebAssembly AOT modules (planned for v3).

### 3. Driver Fault Recovery
- **Constraint**: If a Ring 3 userspace driver process triggers an unhandled memory fault or panic, the kernel safely isolates and terminates the task.
- **Impact**: The driver process is shut down securely, but the system does not yet automatically restart the driver or re-bind its hardware IRQ endpoint. Dynamic driver restartability will be implemented in v2.

### 4. Build Environment & OVMF EFI Path
- **Constraint**: The default `Makefile` specifies the OVMF UEFI firmware path for Debian/Ubuntu systems (`/usr/share/ovmf/OVMF.fd`).
- **Impact**: On non-Debian distributions (Fedora, Arch) or macOS, developers must specify the OVMF path explicitly when invoking make:
  ```bash
  make run OVMF=/path/to/OVMF.fd
  ```

### 5. Synchronous IPC Only
- **Constraint**: IPC communication is currently strictly synchronous and blocking.
- **Impact**: Senders block until receivers execute a corresponding `SYS_RECV` call. Asynchronous notification queues and unbuffered message channels are planned for v2.
