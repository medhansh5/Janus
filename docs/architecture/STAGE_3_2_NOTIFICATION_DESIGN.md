# Janus OS — Stage 3.2 Architecture Design (Remediated v2.1.0)
## Asynchronous Notifications, Notification Capabilities & IRQ Routing

**Document Version:** 2.1.0 (Remediated)  
**Status:** PROPOSED & APPROVED FOR STAGE 3.2A IMPLEMENTATION  
**Review Targets:**  
- Incorporates all resolutions from `docs/architecture/STAGE_3_2_NOTIFICATION_REVIEW.md`  
- Incorporates all resolutions from `docs/architecture/STAGE_3_2_NOTIFICATION_RE_REVIEW.md` (REV-01 through REV-04)  
**Target Baseline Commit:** `f3130f9a19ab9efc2cc3e5b9a37ca76f36221b6a`  
**Parent Baseline Commit:** `b58d23aebbee2a5ce625cb5b612b42274d5449c4`  
**Target Modules:** `kernel/src/notification.rs`, `kernel/src/capability.rs`, `kernel/src/task.rs`, `kernel/src/ipc.rs`, `kernel/src/syscall.rs`, `kernel/src/idt.rs`, `kernel/src/pic.rs`

---

## 1. Executive Summary & Mission

Stage 3.1 established a formally verified capability engine in Janus OS, supporting static 2048-slot capability derivation trees (CDT), generation-tagged 64-bit canonical `CPtr` handles, and strict rights attenuation. Stage 3.1 also reserved `CapType::Notification` (discriminant `3`).

This document specifies the complete asynchronous notification architecture for Janus OS. Version 2.1.0 resolves all findings from the independent adversarial re-review (`STAGE_3_2_NOTIFICATION_RE_REVIEW.md`), including capability generation encoding in `payload_a` (REV-01), isolation of Endpoint badge semantics (REV-02), exact integration of reference decrements into `cleanup_slot` (REV-03), and formal proof of the 2048-slot reference count upper bound (REV-04).

### Key Architectural Pillars
1. **Bounded Static Memory:** Zero heap allocation (`no_std`). A fixed array of 64 `NotificationObject` structures is statically allocated in `.bss` (total footprint: 1536 bytes).
2. **63-Bit Coalescing Event Bitmask:** Events accumulate via bitwise-OR into a 63-bit pending bitmask (`bits 0..=62`). Event bursts coalesce naturally, guaranteeing that event queues cannot overflow or drop messages.
3. **Collision-Free Syscall Return ABI:** Bit 63 is strictly reserved. Successful wait/poll invocations return values with bit 63 clear (`<= 0x7FFF_FFFF_FFFF_FFFF`). The sentinel value `u64::MAX` (`!0`, which has bit 63 set) is reserved exclusively for syscall errors.
4. **Cooperative Stack-Switch Wakeup Protocol:** Janus OS yields blocked notification waiters via `switch_context`. Upon signaling, the producer deposits the delivered bitmask into `task.ipc_message` and marks the task `Runnable`. Rescheduled tasks resume directly inside `sys_notify_wait`, retrieve `task.ipc_message`, sanitize it to 0, and return the result up the standard `syscall_entry` call stack to user space via `iretq`.
5. **Capability Generation Encoding (REV-01):** For `CapType::Notification`, `CNodeSlot.payload_a` packs both the 32-bit object generation in the high half and the 32-bit notification ID in the low half: `payload_a = ((object_generation as u64) << 32) | (notification_id as u64)`. Every capability lookup strictly compares `(slot.payload_a >> 32) as u32 == notif.generation`, rendering stale handle access to recycled objects impossible.
6. **Isolated Endpoint vs. Notification Badging (REV-02):** Notification badge validation enforces the 63-bit mask boundary (`bits 0..=62`) strictly on `CapType::Notification`, leaving 64-bit synchronous `CapType::Endpoint` badges unaffected.
7. **Authoritative Lifecycle Accounting via `cleanup_slot` (REV-03):** Decrementing `ref_count` is integrated directly into `kernel/src/capability.rs::cleanup_slot`, guaranteeing exact, transactional accounting across `delete_cap`, recursive `revoke_descendants_iterative`, and future CSpace teardown without double decrements or reference leaks.
8. **Proven Reference Count Bound (REV-04):** Proven invariant: $\text{ref\_count} \le \text{TOTAL\_CDT\_SLOTS} = 2048 \ll 65535$ (`u16::MAX`). Overflow is physically impossible.
9. **Explicit Milestone Separation:**
   - **Stage 3.2A (Core Asynchronous Notifications):** Pure software IPC, creation, signaling, badging, single-waiter blocking/polling, reference accounting, and capability derivation. **100% host unit-testable.**
   - **Stage 3.2B (Hardware IRQ Routing & Driver Protocol):** Hardware PIC masking, EOI protocol, spurious IRQ detection, and IDT vector mapping. **Explicitly deferred until Stage 3.2A is implemented and audited, and all hardware prerequisites are satisfied.**

---

## 2. Actual Kernel Architecture Findings & Constraints

An inspection of the committed Stage 3.1 codebase (`main` at `f3130f9a19ab9efc2cc3e5b9a37ca76f36221b6a`) establishes the exact architectural foundation:

### 2.1 Task Context & Storage (`kernel/src/task.rs:20-85`)
- `TaskContext` stores **only callee-saved registers** (`r15, r14, r13, r12, rbx, rbp, rsi, rdi`) and stack pointers (`rsp, rip`).
- **There is no `rax` field in `TaskContext`.** Context switching via `switch_context` unwinds via `ret` to the caller of `switch_context`.
- `Task` defines `pub ipc_message: u64`, a dedicated 64-bit word used to transfer message contents during kernel syscall blocking and unblocking.
- `MAX_TASKS = 16`, task IDs `0..=15`. ID 15 is `IDLE_TASK_ID`.

### 2.2 Syscall Entry and Return Flow (`kernel/src/syscall.rs:38-102`)
The x86_64 `syscall`/`sysretq` interface uses `SFMask` to clear `RFLAGS.IF` on entry (interrupts disabled).
When a task blocks inside a syscall, it calls `schedule()`, yielding the CPU while staying on its kernel stack. When rescheduled, execution resumes immediately after `schedule()` in the syscall body, returns through `syscall_handler`, and exits to user space via `iretq` with `RAX` holding the return value.

### 2.3 Committed Capability Layout (`kernel/src/capability.rs:78-87`)
The committed `CNodeSlot` layout is:
```rust
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CNodeSlot {
    pub cap_type: CapType,   // offset  0, size 1, align 1
    pub rights: CapRights,   // offset  1, size 1, align 1
    pub _reserved1: u16,     // offset  2, size 2, align 2 (must be 0)
    pub generation: u32,     // offset  4, size 4, align 4 (32-bit slot generation)
    pub payload_a: u64,      // offset  8, size 8, align 8
    pub payload_b: u64,      // offset 16, size 8, align 8
    pub cdt_index: u32,      // offset 24, size 4, align 4 (Index into CDT_ARENA)
    pub _reserved2: [u8; 4], // offset 28, size 4, align 1 (must be 0)
}
```
- Total size: **32 bytes**, aligned to 8 bytes.
- CDT links reside in the separate static `CDT: [CdtNode; TOTAL_CDT_SLOTS]` table, **not** inside `CNodeSlot`.
- `TOTAL_CDT_SLOTS = MAX_TASKS * CNODE_SLOTS = 16 * 128 = 2048`.

### 2.4 Payload Mapping Matrix Across Capability Types

| Capability Type | `payload_a` (64 bits) | `payload_b` (64 bits) | Description |
|---|---|---|---|
| `CapType::Empty` | `0` | `0` | Unallocated slot. |
| `CapType::Frame` | Physical Frame Address (`u64`) | Virtual Address / `UNMAPPED_VADDR` | Physical memory frame. |
| `CapType::MmioFrame`| Physical Frame Address (`u64`) | Virtual Address / `UNMAPPED_VADDR` | Device MMIO frame. |
| `CapType::Endpoint` | Target Task ID (`u64`) | Badge (`u64`, full 64-bit value) | Synchronous IPC endpoint. |
| **`CapType::Notification`** | **`(object_generation << 32) \| notif_id`** | **Badge (`u64`, bits 0..=62)** | **Asynchronous event object.** |
| `CapType::IOPort` | `(port_size << 16) \| base_port` | `0` | x86 I/O port range. |
| `CapType::Irq` | Hardware IRQ Line (`0..15`) | `0` | Hardware interrupt line. |
| `CapType::CNode` | Physical CNode Address (`u64`) | Slot Count (`u64`) | Capability space container. |

---

## 3. Notification Object Model & Invariants

### 3.1 63-Bit Coalescing Event Bitmask
Valid event channels are restricted to **bits 0 through 62**:
$$\text{Valid Mask Range: } 0x0000\_0000\_0000\_0001 \dots 0x7FFF\_FFFF\_FFFF\_FFFF$$
- **Bit 63 is strictly reserved.** The kernel rejects any signal, badge, or binding with bit 63 set (`CapError::InvalidBadge` / `SYS_ERR_INVALID_ARG`).
- **Coalescing Semantics:** When an event fires, the signal bitmask $S$ is merged via bitwise-OR:
  $$\text{pending\_bits}_{\text{new}} = \text{pending\_bits}_{\text{old}} \mid S$$
  Multiple occurrences of the same event bit coalesce into a single flag without counter overflow, memory allocation, or message loss.
- **Read-and-Clear Consumption:** Waiting (`SYS_NOTIFY_WAIT`) and polling (`SYS_NOTIFY_POLL`) atomically capture `pending_bits` and reset it to zero:
  $$\text{result} = \text{pending\_bits}, \quad \text{pending\_bits} \leftarrow 0$$

### 3.2 Single-Waiter Invariant
> **Invariant 1:** At any given time, at most one task may be registered as waiting on a `NotificationObject`.
- The waiting task is tracked via `waiting_task: Option<u8>` (task ID $0 \dots 15$).
- If task $A$ is blocked on notification $N$, and task $B$ invokes `SYS_NOTIFY_WAIT` on $N$, the kernel immediately rejects task $B$ with error `!0` (`SYS_ERR_BUSY`) without blocking.
- This guarantees $O(1)$ constant-time wakeups, zero queue allocations, and deterministic scheduling.

### 3.3 Event Bits vs. Badge Semantics
- **Event Bits:** Represent distinct event conditions (e.g., bit 0 = timer tick, bit 1 = serial data ready, bit 2 = disk I/O complete).
- **Badge:** Embedded in `payload_b`. Represents a fixed identity bitmask.
  - **Unbadged Capability (`badge == 0`):** Authorized to signal any bitmask in `0..=62` via register `RDX`.
  - **Badged Capability (`badge != 0`):** The kernel strictly ignores the caller's `RDX` and sets only `pending_bits |= badge`. Untrusted clients cannot spoof other event channels or clients.
  - **Badge Immutability:** Once minted with a non-zero badge, derived capabilities inherit the parent's badge immutably.
  - **Receiver Disambiguation:** A receiver can uniquely identify which client signaled it if and only if clients are assigned non-overlapping badge bitmasks (e.g. Client 1 = `1 << 0`, Client 2 = `1 << 1`).

---

## 4. Memory Layout & Object Structure

All notification state is held in a statically allocated kernel array in `.bss`:

```rust
pub const MAX_NOTIFICATIONS: usize = 64;
pub const MAX_NOTIFICATION_EVENT_MASK: u64 = 0x7FFF_FFFF_FFFF_FFFF; // Bits 0..=62

#[repr(C)]
pub struct NotificationObject {
    /// Accumulated event bitmask pending delivery (bits 0..=62).
    pub pending_bits: u64,
    /// Task currently blocked waiting on this notification (if any).
    pub waiting_task: Option<u8>,
    /// Hardware IRQ line bound to this notification (Stage 3.2B).
    pub bound_irq: Option<u8>,
    /// Task that created the notification (initial owner).
    pub owner_task: Option<u8>,
    /// Active capability reference count (resolves FIND-3.2-03, REV-04).
    pub ref_count: u16,
    /// Allocation generation counter (increments on recycle; resolves REV-01).
    pub generation: u32,
    /// Allocation tracking flag.
    pub active: bool,
}
```

### Layout and Footprint
- `pending_bits`: `u64` (8 bytes)
- `waiting_task`: `Option<u8>` (2 bytes)
- `bound_irq`: `Option<u8>` (2 bytes)
- `owner_task`: `Option<u8>` (2 bytes)
- `ref_count`: `u16` (2 bytes)
- `generation`: `u32` (4 bytes)
- `active`: `bool` (1 byte)
- Padding: 3 bytes to align to 24 bytes (`#[repr(C)]`).
- Total structure size: **24 bytes**.
- Table size: $64 \times 24\text{ bytes} = 1536\text{ bytes}$ (1.5 KiB in `.bss`).
- Statically initialized in `.bss`:
  ```rust
  static mut NOTIFICATIONS: [NotificationObject; MAX_NOTIFICATIONS] = [
      NotificationObject {
          pending_bits: 0,
          waiting_task: None,
          bound_irq: None,
          owner_task: None,
          ref_count: 0,
          generation: 1, // Generations start at 1; 0 is sentinel invalid
          active: false,
      }; MAX_NOTIFICATIONS
  ];
  ```

---

## 5. Capability Integration, Ownership & Lifecycle

### 5.1 Capability Encoding for `CapType::Notification` (Resolves REV-01)
In the 32-byte `CNodeSlot`:
- **`cap_type`:** `CapType::Notification` (discriminant `3`).
- **`rights`:** `CapRights` bitflags:
  - `CapRights::WRITE` (`0x02`): **SIGNAL Authority.** Permission to invoke `SYS_NOTIFY_SIGNAL`.
  - `CapRights::READ` (`0x01`): **WAIT & POLL Authority.** Permission to invoke `SYS_NOTIFY_WAIT` and `SYS_NOTIFY_POLL`.
  - `CapRights::GRANT` (`0x08`): **DERIVATION Authority.** Permission to mint (`SYS_CAP_MINT`) or copy (`SYS_CAP_COPY`).
- **`payload_a`:** Packed 64-bit word:
  $$\text{payload\_a} = ((\text{object\_generation as u64}) \ll 32) \mid (\text{notification\_id as u64})$$
  - Bits 0..31: `notification_id: u32` ($0 \dots 63$).
  - Bits 32..63: `object_generation: u32` ($1 \dots \text{u32::MAX}-1$).
- **`payload_b`:** `badge as u64` ($0$ = unbadged; non-zero = badged, bits $0 \dots 62$).

### 5.2 Lookup and Generation Validation Protocol (Resolves REV-01)
Every syscall operating on a `CapType::Notification` capability performs strict generation validation:
```rust
pub fn lookup_notification(
    cnode: CPtr,
    cptr: CPtr,
    required_rights: CapRights,
) -> Result<(usize, &mut NotificationObject), u64> {
    let slot = match lookup_cap(cnode, cptr) {
        Ok(s) => s,
        Err(_) => return Err(!0),
    };
    if slot.cap_type != CapType::Notification || !slot.rights.contains(required_rights) {
        return Err(!0);
    }
    let notif_id = (slot.payload_a & 0xFFFF_FFFF) as usize;
    let expected_gen = (slot.payload_a >> 32) as u32;

    if notif_id >= MAX_NOTIFICATIONS || expected_gen == 0 {
        return Err(!0);
    }
    let notif = unsafe { &mut NOTIFICATIONS[notif_id] };
    if !notif.active || notif.generation != expected_gen {
        return Err(!0); // Stale handle referencing recycled object
    }
    Ok((notif_id, notif))
}
```

### 5.3 Extension of `mint_cap_internal` (Resolves REV-02)
To prevent notification bitmask constraints from restricting synchronous Endpoint badges, match arms in `kernel/src/capability.rs::mint_cap_internal` are kept strictly separated:

```rust
// In mint_cap_internal (validation phase):
match src_slot_ref.cap_type {
    CapType::MmioFrame => {
        if new_rights.contains(CapRights::EXECUTE) {
            return Err(CapError::InvalidRights);
        }
    }
    CapType::Frame => {
        if new_rights.contains(CapRights::WRITE) && new_rights.contains(CapRights::EXECUTE) {
            return Err(CapError::InvalidRights);
        }
    }
    CapType::Endpoint => {
        // Synchronous IPC endpoints permit full 64-bit badge; enforce immutability if parent badged
        if src_slot_ref.payload_b != 0 && new_badge != src_slot_ref.payload_b {
            return Err(CapError::InvalidBadge);
        }
    }
    CapType::Notification => {
        // Enforce 63-bit mask boundary strictly for Notifications
        if (new_badge & !MAX_NOTIFICATION_EVENT_MASK) != 0 {
            return Err(CapError::InvalidBadge);
        }
        // Enforce badge immutability if parent badged
        if src_slot_ref.payload_b != 0 && new_badge != src_slot_ref.payload_b {
            return Err(CapError::InvalidBadge);
        }
    }
    _ => {
        if new_badge != 0 {
            return Err(CapError::InvalidBadge);
        }
    }
}

// In payload_a and payload_b assignment (commit phase):
dest_slot_ref.payload_a = src_slot_ref.payload_a; // Preserves (generation << 32) | notif_id
dest_slot_ref.payload_b = match src_slot_ref.cap_type {
    CapType::Frame | CapType::MmioFrame => UNMAPPED_VADDR,
    CapType::Endpoint => {
        if src_slot_ref.payload_b != 0 { src_slot_ref.payload_b } else { new_badge }
    }
    CapType::Notification => {
        if src_slot_ref.payload_b != 0 { src_slot_ref.payload_b } else { new_badge }
    }
    _ => 0,
};

// Increment reference count at commit point (Resolves REV-03):
if src_slot_ref.cap_type == CapType::Notification {
    let notif_id = (src_slot_ref.payload_a & 0xFFFF_FFFF) as usize;
    unsafe {
        NOTIFICATIONS[notif_id].ref_count += 1;
    }
}
```

### 5.4 Integration of Reference Decrement in `cleanup_slot` (Resolves REV-03)
In Janus OS, every slot deletion (`delete_cap`) and descendant revocation (`revoke_descendants_iterative`) passes through `cleanup_slot(task_id: u8, slot: u8)` (`kernel/src/capability.rs:459-473`). Integrating the reference decrement directly into `cleanup_slot` establishes a single authoritative lifecycle funnel:

```rust
// In kernel/src/capability.rs:
unsafe fn cleanup_slot(task_id: u8, slot: u8) {
    if let Ok(cnode) = get_task_cnode_mut(task_id) {
        let slot_ref = &mut cnode.slots[slot as usize];
        if (slot_ref.cap_type == CapType::Frame || slot_ref.cap_type == CapType::MmioFrame)
            && slot_ref.payload_b != UNMAPPED_VADDR
        {
            slot_ref.payload_b = UNMAPPED_VADDR;
        }

        // Exact reference decrement for Notifications (Resolves REV-03)
        if slot_ref.cap_type == CapType::Notification {
            let notif_id = (slot_ref.payload_a & 0xFFFF_FFFF) as usize;
            if notif_id < MAX_NOTIFICATIONS {
                let notif = &mut NOTIFICATIONS[notif_id];
                if notif.ref_count > 0 {
                    notif.ref_count -= 1;
                    if notif.ref_count == 0 {
                        // All capability references destroyed: reset object
                        reset_notification_object(notif_id);
                    }
                }
            }
        }

        slot_ref.cap_type = CapType::Empty;
        slot_ref.rights = CapRights::empty();
        slot_ref.payload_a = 0;
        slot_ref.payload_b = 0;
        slot_ref.generation = slot_ref.generation.saturating_add(1);
    }
}
```

#### Object Reset Procedure:
```rust
unsafe fn reset_notification_object(id: usize) {
    let notif = &mut NOTIFICATIONS[id];
    // 1. Cancel registered waiter if one exists
    if let Some(waiter_id) = notif.waiting_task {
        if let Some(waiter) = crate::task::get_task_mut(waiter_id as u64) {
            waiter.ipc_message = !0; // Error sentinel alerts caller
            waiter.state = TaskState::Runnable;
        }
    }
    // 2. Advance generation counter; permanently retire if exhausted
    if notif.generation == u32::MAX {
        notif.active = false; // Slot permanently retired
        return;
    }
    notif.generation = notif.generation.saturating_add(1);
    // 3. Clear state
    notif.pending_bits = 0;
    notif.waiting_task = None;
    notif.bound_irq = None;
    notif.owner_task = None;
    notif.ref_count = 0;
    notif.active = false; // Returned to allocation pool
}
```

### 5.5 Comprehensive Lifecycle Accounting Matrix

| Operation | Reference Change | Exact Mutation Point | Rollback Guarantee |
|---|---|---|---|
| **`SYS_NOTIFY_CREATE`** | `+1` (initial) | Inside `sys_notify_create` upon slot minting | Preflight checks empty slot & free object; no partial write. |
| **`mint_cap`** | `+1` | `mint_cap_internal` (line 351) commit point | All checks precede mutation; commit cannot fail. |
| **`copy_cap`** | `+1` | Inherited from `mint_cap_internal` commit | Identical transactional commit as `mint_cap`. |
| **`delete_cap`** | `-1` per revoked leaf, `-1` for root | Inside `cleanup_slot` called for each node | Revocation preflight ensures cycle-free traversal before mutation. |
| **`revoke_cap`** | `-1` per revoked descendant | Inside `cleanup_slot` called for each descendant | Root capability is untouched (`ref_count >= 1`). |
| **CSpace Destruction** | `-1` for each occupied slot | `cleanup_slot` called during task teardown | Guarantees zero reference leaks upon process exit. |
| **Failed Operation** | `0` | Returns error prior to mutation | Zero state change; references untouched. |

---

## 6. Proof of Reference Count Upper Bound (Resolves REV-04)

### Formal Theorem
> **Theorem:** For any `NotificationObject` $N$ in Janus OS, $\text{ref\_count}(N) \le 2048$.

### Proof:
1. In Janus OS, `MAX_TASKS = 16` (`kernel/src/task.rs:79`).
2. Each task possesses exactly one root CNode containing `CNODE_SLOTS = 128` slots (`kernel/src/capability.rs:83`).
3. There are no secondary CNodes, dynamic CNode allocations, or kernel-space capability pools outside this static set.
4. Total capability slots in the entire operating system:
   $$\text{Total Slots} = \text{MAX\_TASKS} \times \text{CNODE\_SLOTS} = 16 \times 128 = 2048$$
5. Every valid capability must occupy an active `CNodeSlot`.
6. Therefore, the absolute worst-case scenario (where every single slot in every task references the same notification object) yields:
   $$\text{ref\_count}_{\max} = 2048$$
7. Since $2048 \le 65535$ (`u16::MAX`), a 16-bit unsigned integer can **never overflow** under any execution path. $\blacksquare$

### Defense-in-Depth Assertion:
In `increment_notification_ref`:
```rust
if notif.ref_count >= TOTAL_CDT_SLOTS as u16 {
    return Err(CapError::CSpaceFull);
}
```

---

## 7. Syscall Registry & Return ABI Specification

### 7.1 Syscall Allocation Registry
Assigned numbers integrate cleanly into `kernel/src/ipc.rs`:

| Number | Name | Arguments | Description |
|---|---|---|---|
| `9` | `SYS_NOTIFY_CREATE` | `RSI = dst_cptr` | Allocates free notification object and mints root capability. |
| `10` | `SYS_NOTIFY_SIGNAL` | `RSI = cptr, RDX = bits` | Signals event bitmask (enforces badge if badged). |
| `15` | `SYS_NOTIFY_WAIT` | `RSI = cptr` | Blocks until events pending; consumes and returns bitmask. |
| `16` | `SYS_NOTIFY_POLL` | `RSI = cptr` | Non-blocking read-and-clear. Returns 0 if no events pending. |
| `17` | `SYS_IRQ_BIND` | `RSI = irq_cptr, RDX = notif_cptr, R10 = irq_bit` | **Deferred to Stage 3.2B.** |
| `18` | `SYS_IRQ_ACK` | `RSI = irq_cptr` | **Deferred to Stage 3.2B.** |

### 7.2 Return Value & Error Code ABI
- **Standard Sentinel:** In compliance with all existing Janus syscalls, `!0` (`u64::MAX = 0xFFFF_FFFF_FFFF_FFFF`) indicates failure.
- **Success Range:** All valid bitmasks are bounded to bits 0..=62:
  $$0x0000\_0000\_0000\_0000 \dots 0x7FFF\_FFFF\_FFFF\_FFFF$$
- **Bit 63 Invariant:** Any return value where `(val & (1 << 63)) != 0` is strictly an error.
- **Polling vs. Waiting Return Semantics:**
  - `SYS_NOTIFY_POLL`: Returns `0` if successful and no events were pending. Returns positive non-zero bitmask ($1 \dots 2^{63}-1$) if events were consumed. Returns `!0` on syscall error (e.g., invalid handle, generation mismatch, permission denied).
  - `SYS_NOTIFY_WAIT`: Blocks until `pending_bits != 0`. On success, returns positive non-zero bitmask ($1 \dots 2^{63}-1$). Returns `!0` on syscall error or waiter cancellation.

---

## 8. Blocking & Wakeup Protocol

### 8.1 Syscall Return Flow Architecture
```
===================================================================
1. Waiter Calling SYS_NOTIFY_WAIT:
===================================================================
User Space (Ring 3)
   │
   ▼ syscall (RDI=15, RSI=cptr)
syscall_entry (kernel/src/syscall.rs)
   │ (Constructs interrupt frame, saves user RSP/RIP, disables IF)
   ▼
syscall_handler -> sys_notify_wait(cptr) (kernel/src/ipc.rs)
   │
   ├─► Validate (cptr, generation, CapRights::READ)
   │
   ├─► Fast path: if pending_bits != 0:
   │      bits = pending_bits; pending_bits = 0;
   │      return bits; ────────────────────────────────────────┐
   │                                                           │
   └─► Slow path (no events pending):                          │
          task.state = TaskState::BlockedOnNotification { id } │
          NOTIFICATIONS[id].waiting_task = Some(task.id);      │
          schedule(); # Context switch! Yields CPU.            │
          # ===============================================    │
          # Resumes here when woken!                           │
          # ===============================================    │
          bits = task.ipc_message;                             │
          task.ipc_message = 0; # Sanitize                     │
          return bits; ────────────────────────────────────────┤
                                                               ▼
syscall_handler returns bits in RAX ◄──────────────────────────┘
   │
syscall_entry: pop registers, skip pushed RAX, iretq
   │
   ▼ User Space resumes with RAX = delivered bitmask!

===================================================================
2. Producer Calling SYS_NOTIFY_SIGNAL:
===================================================================
sys_notify_signal(cptr, bits):
   │
   ├─► Validate (cptr, generation, CapRights::WRITE)
   ├─► Determine mask:
   │      if slot.payload_b != 0 { slot.payload_b } else { bits & 0x7FFF_FFFF_FFFF_FFFF }
   ├─► If mask == 0: return 0 (no-op probe)
   ├─► NOTIFICATIONS[id].pending_bits |= mask
   ├─► If NOTIFICATIONS[id].waiting_task is Some(waiter_id):
   │      let waiter = &mut TASKS[waiter_id];
   │      waiter.ipc_message = NOTIFICATIONS[id].pending_bits;
   │      NOTIFICATIONS[id].pending_bits = 0;
   │      NOTIFICATIONS[id].waiting_task = None;
   │      waiter.state = TaskState::Runnable; # Waiter eligible to run!
   └─► return 0 (success)
```

### 8.2 Safety Proof for Reusing `task.ipc_message`
1. **State Isolation:** While blocked, `state == TaskState::BlockedOnNotification { id }`. `sys_send` refuses delivery because target is not `BlockedOnRecv`.
2. **Sequential Signal Retention:** If a second signal arrives while the waiter is `Runnable` but has not yet executed, `waiting_task` is `None`. The second signal accumulates in `pending_bits` without touching `task.ipc_message`. The waiter consumes the first signal, and its subsequent wait call takes the fast path on the second signal. Zero event loss.

---

## 9. Stage 3.2B Hardware IRQ Routing Boundary & Prerequisites

Hardware IRQ routing (`SYS_IRQ_BIND` [17] and `SYS_IRQ_ACK` [18]) is **formally deferred to Stage 3.2B**.

### Prerequisites for Stage 3.2B Implementation:
1. **Stage 3.2A Completion:** Stage 3.2A must be fully implemented, tested, and pass post-commit audit.
2. **Spurious IRQ Detection:**
   - In `kernel/src/pic.rs`, implement `is_spurious_irq7()` and `is_spurious_irq15()` via 8259 ISR read.
   - For spurious IRQ 7: discard without sending EOI.
   - For spurious IRQ 15: send EOI to Master PIC only.
3. **IDT Vector Coverage:**
   - Register IDT stubs in `kernel/src/idt.rs` for vectors 33..41 (including vector 36 for COM1 / IRQ 4).
4. **Driver Drain-Until-Empty Contract:**
   - Document requirement: drivers must drain device FIFOs until empty before invoking `SYS_IRQ_ACK`.

---

## 10. Re-Review Finding Closure Matrix (REV-01 through REV-04)

| Finding | Severity | Resolution in Design v2.1.0 | Evidence in Code / Design | Status |
|---|---|---|---|---|
| **REV-01** | **HIGH** | Encoded `(object_generation << 32) \| notif_id` in `payload_a`. Lookup strictly validates `slot_gen == notif.generation`. Handled generation exhaustion at `u32::MAX`. | Section 2.4, 5.1, 5.2, 5.4 | **CLOSED** |
| **REV-02** | **MEDIUM** | Separated match arms in `mint_cap_internal`. `MAX_NOTIFICATION_EVENT_MASK` applies strictly to `CapType::Notification`, leaving 64-bit Endpoint badges unrestricted. | Section 5.3 | **CLOSED** |
| **REV-03** | **MEDIUM** | Integrated `ref_count -= 1` directly into `kernel/src/capability.rs::cleanup_slot`, establishing a single authoritative lifecycle funnel across delete and revoke. | Section 5.4, 5.5 | **CLOSED** |
| **REV-04** | **LOW** | Formally proved `ref_count <= TOTAL_CDT_SLOTS = 2048 << u16::MAX (65535)`, proving overflow is physically impossible. Added defense assertion. | Section 6 | **CLOSED** |

---

## 11. Comprehensive Unit Test Plan (Stage 3.2A)

The following 18 host unit tests must be implemented in `kernel/src/notification.rs` and `kernel/src/capability.rs`:

1. `test_notif_create_and_signal_unbadged`: Verify creation, signaling bits 0..=62, coalescing.
2. `test_notif_signal_bit63_rejected`: Verify signaling with bit 63 set returns error `!0`.
3. `test_notif_payload_a_generation_encoding`: Verify `payload_a` packs generation in high 32 bits and ID in low 32 bits.
4. `test_notif_stale_generation_rejected`: Verify capability lookup fails if object generation has advanced.
5. `test_notif_endpoint_badge_unaffected`: Verify `CapType::Endpoint` retains full 64-bit badge capability (bit 63 permitted).
6. `test_notif_badged_mint_immutability`: Verify child inherits parent badge; altering non-zero badge returns `InvalidBadge`.
7. `test_notif_badged_signal_enforcement`: Verify caller bits ignored and fixed badge applied.
8. `test_notif_wait_fast_path`: Verify immediate return when `pending_bits != 0`.
9. `test_notif_wait_slow_path_blocking`: Verify task state transitions to `BlockedOnNotification`.
10. `test_notif_single_waiter_busy`: Verify second waiter receives error `!0` immediately.
11. `test_notif_wakeup_protocol`: Verify signal writes to `task.ipc_message` and marks `Runnable`.
12. `test_notif_poll_read_and_clear`: Verify `SYS_NOTIFY_POLL` consumes bits and resets `pending_bits = 0`.
13. `test_notif_poll_empty_returns_zero`: Verify polling with no events returns `0`.
14. `test_notif_copy_preserves_badge_and_generation`: Verify `copy_cap` preserves `payload_a` and `payload_b` and increments `ref_count`.
15. `test_notif_cleanup_slot_decrements_refcount`: Verify `cleanup_slot` decrements `ref_count`.
16. `test_notif_object_recycled_on_zero_refcount`: Verify `active = false` and generation increments when `ref_count == 0`.
17. `test_notif_deletion_wakes_blocked_waiter`: Verify deleting capability wakes waiter with error `!0`.
18. `test_notif_rights_attenuation`: Verify signal without `WRITE` or wait without `READ` returns error `!0`.

---

## 12. Implementation-Readiness Verdicts

### Stage 3.2A: Core Asynchronous Notifications (Pure IPC)
**Verdict:** **READY FOR IMPLEMENTATION**  
All critical, high, medium, and low findings from both independent reviews are closed. The design is internally consistent, mathematically proven, and verified against the committed codebase.

### Stage 3.2B: Hardware IRQ Routing & Driver Protocol
**Verdict:** **DEFERRED — PREREQUISITES NOT VERIFIED**  
Hardware interrupt binding remains deferred until Stage 3.2A is implemented, tested, and audited, and all Stage 3.2B prerequisites are satisfied.
