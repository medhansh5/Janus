# Janus OS Known Limitations

This document lists explicit technical limitations, constraints, and non-goals in the current release of Janus OS.

## System & Execution Constraints

### 1. Uniprocessor (Single-Core Execution) & Syscall Entry
- **Constraint**: Janus executes on a single x86_64 CPU core using the 8259 PIC and 8254 PIT (IRQ0 at 1000 Hz) for preemptive scheduling.
- **Syscall Scratch Assumption**: `syscall_entry` (`kernel/src/syscall.rs`) uses global `USER_RSP_SCRATCH` and `KERNEL_RSP_SCRATCH` variables with `SFMask` clearing `IF=0` on entry and zeroing `USER_RSP_SCRATCH` immediately after pushing the frame. This is safe under single-core execution with maskable interrupts disabled, but is **not SMP-safe** and **not reentrant** under an NMI or fault during the entry window.
- **Impact**: Multi-core SMP (x2APIC, IPIs, per-CPU `swapgs` kernel stacks) is deferred to a future milestone.

### 2. VirtIO Block Driver Limitations
- **Transport**: Only legacy VirtIO 0.9.5 PCI (`vendor 0x1AF4`, `device 0x1001`) with an I/O-space `BAR0` and queue size `<= 256` is supported. Modern VirtIO 1.x MMIO/PCI capability-list transport and NVMe are not implemented.
- **Synchronous Polling**: `VirtioBlockDevice::read_sector` and `write_sector` poll `used.idx` synchronously with a bounded spin limit (`2_000_000` iterations) rather than using asynchronous IRQ11 completion.

### 3. JanusFS Storage & Crash Consistency Limitations
- **Capacity & Layout**: Fixed 512-byte sectors (`CACHE_BLOCKS = 64`, `MAX_INODES = 64`, `MAX_DIRECT_BLOCKS = 5`, `MAX_FILE_SIZE = 2560` bytes, 16-byte filenames).
- **Persistence vs. Power-Loss Atomicity**: Filesystem metadata and data persist across normal QEMU shutdown/reboot via VirtIO sector writes, and disk-full or synchronous I/O errors roll back cleanly in memory. However, JanusFS has **no journaling or copy-on-write**, so power loss mid-flush across multiple sectors is not crash-atomic.

### 4. Userland Application Interface
- **Constraint**: High-level C/Rust standard library support (`std`) is not available in Ring 3 userspace tasks.
- **Impact**: Current userspace tasks execute using custom `no_std` entrypoints and direct `syscall` ABI invocations. High-level application support (WASM AOT) is planned for future releases.

### 5. Driver Fault Recovery
- **Constraint**: If a Ring 3 userspace driver process triggers an unhandled memory fault or exits, the kernel isolates the task, cleans its `CSpace`, and unmasks/unbinds any exclusively owned IRQ lines.
- **Impact**: Automatic driver restartability is not yet implemented.
