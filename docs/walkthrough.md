# Walkthrough: Phase 3 - Preemptive Scheduler & IDT

Phase 3 introduces hardware interrupts, an IDT (Interrupt Descriptor Table), the legacy 8259 PIC (Programmable Interrupt Controller), and a preemptive scheduler to Janus v0.

## 1. Unifying Context Switch to `iretq`

We successfully transitioned from `sysretq` to `iretq` for all userspace transitions.
- The `TaskContext` struct was reverted to its pure kernel thread state (callee-saved registers).
- The `syscall_entry` assembly was rewritten to push a full `InterruptFrame` (SS, RSP, RFLAGS, CS, RIP) upon entry, and use `iretq` to pop it upon exit.
- `jump_to_user` now explicitly constructs a 5-word `iretq` frame and jumps to userspace, setting RFLAGS with `IF=1` to enable hardware interrupts.

## 2. Interrupt Descriptor Table (IDT)

We implemented a global IDT with handlers for:
- **#GP (13)**: General Protection Fault
- **#PF (14)**: Page Fault
- **IRQ0 (32)**: Timer Interrupt

The common exception stub pushes the vector number, error code, and all general-purpose registers, passing them as an `ExceptionFrame` to the Rust handler. 
Faults currently trigger a kernel panic and system halt (to ensure we catch capability errors).

## 3. PIC and Timer Interrups

We implemented a basic driver for the legacy 8259 PIC:
- Remapped the PIC IRQs to vectors 32-47 (avoiding CPU exception vectors 0-31).
- Unmasked IRQ0 (the system timer) to fire on vector 32.
- The timer handler in `idt.rs` acknowledges the interrupt (`end_of_interrupt`) and calls `schedule()`.

## 4. Preemptive Scheduling

With the timer ticking and interrupts enabled in userspace (`IF=1`), Janus is now preemptive!
- We observed QEMU successfully firing `INT=0x20` (IRQ0) while `CPL=3` (user mode).
- The timer handler preempts the user task, switches the kernel stack (`KERNEL_RSP_SCRATCH` and TSS `RSP0`), and round-robins to the next `Runnable` task.
- Our 50-iteration IPC stress test successfully runs concurrently with timer ticks, and eventually triggers the capability security fault as intended.

> [!NOTE]
> **Known Gap for v1**: Currently, `schedule()` does not implement an "idle loop" or handle the scenario where the system reaches zero `Runnable` tasks (e.g. if the last running task terminates). For v0, the payload loops infinitely by design. For v1+, an explicitly designated kernel Idle Task or power-state halt (`hlt` in a loop) will be required to handle total system starvation/blocking.

## 5. Security & Cleanup

- **THREAT-MODEL**: Updated to document the known risks of DoS via resource starvation, and explicitly deferred multi-core (APIC) migration to v1+.
- **GDT Initialization**: Resolved static mutation warnings in the GDT setup by switching to `spin::Once`.
- **Init Capabilities**: Verified that the root CNode is seeded with `Untyped` memory capabilities representing all free RAM regions.

The Janus v0 pipeline is now complete. We have successfully proven capability-based synchronous IPC, secure SEC-FAULT termination, and hardware-enforced preemption.
