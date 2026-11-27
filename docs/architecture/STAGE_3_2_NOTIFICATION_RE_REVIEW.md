# Janus OS — Stage 3.2 Notification Design v2.0 — Independent Adversarial Re-Review

**Document Version:** 1.0.0  
**Review Type:** Independent Adversarial Architecture Re-Review  
**Status:** COMPLETE  
**Review Verdict:** **APPROVED WITH CONDITIONS — NOT READY UNTIL CONDITIONS ARE CLOSED**  
**Stage 3.2B Verdict:** **DEFERRED — PREREQUISITES NOT YET VERIFIED**  
**Target Document:** `docs/architecture/STAGE_3_2_NOTIFICATION_DESIGN.md` (Version 2.0.0)  
**Previous Review:** `docs/architecture/STAGE_3_2_NOTIFICATION_REVIEW.md`  
**Repository:** `medhansh5/Janus`  
**Target Branch:** `main`  
**Baseline Commit:** `f3130f9a19ab9efc2cc3e5b9a37ca76f36221b6a`  
**Parent Baseline Commit:** `b58d23aebbee2a5ce625cb5b612b42274d5449c4`  

---

## 1. Baseline and Scope Verification

The repository baseline and document scope were independently inspected and verified:

| Inspection Item | Requirement | Observed State | Status |
|---|---|---|---|
| **Repository Identity** | `medhansh5/Janus` | `https://github.com/medhansh5/Janus.git` | **PASS** |
| **Branch** | `main` | `main` (tracking `origin/main`) | **PASS** |
| **HEAD Commit** | `f3130f9a19ab9efc2cc3e5b9a37ca76f36221b6a` | `f3130f9a19ab9efc2cc3e5b9a37ca76f36221b6a` | **PASS** |
| **Parent Commit** | `b58d23aebbee2a5ce625cb5b612b42274d5449c4` | `b58d23aebbee2a5ce625cb5b612b42274d5449c4` | **PASS** |
| **Working Tree** | Clean tracked files | Exactly 0 modified tracked files, 0 staged files | **PASS** |
| **Untracked Docs** | Intact | All 12 existing architecture documents preserved | **PASS** |
| **Primary Target** | `STAGE_3_2_NOTIFICATION_DESIGN.md` | Version 2.0.0 remediated design verified | **PASS** |

### Relevant Source Code Baseline Audited:
- `kernel/src/task.rs` (`TaskContext`, `Task`, `MAX_TASKS = 16`, `schedule`, `switch_context`)
- `kernel/src/ipc.rs` (`handle_syscall`, `sys_send`, `sys_recv`, `wake_irq`, `sys_cap_*`)
- `kernel/src/syscall.rs` (`syscall_entry`, `syscall_handler`, `SFMask`, SysV ABI conventions)
- `kernel/src/capability.rs` (`CNodeSlot`, `CPtr`, `CapType`, `CapRights`, `mint_cap_internal`, `cleanup_slot`, `revoke_descendants_iterative`, `delete_cap`)
- `kernel/src/idt.rs` (`IdtEntry`, `init`, `exception_handler`, vectors 32, 42..47)
- `kernel/src/pic.rs` (`init`, `mask`, `unmask`, `end_of_interrupt`)

---

## 2. Executive Verdict & Summary

### Verdict: **APPROVED WITH CONDITIONS — NOT READY UNTIL CONDITIONS ARE CLOSED**
### Stage 3.2B Verdict: **DEFERRED — PREREQUISITES NOT YET VERIFIED**

The remediated Stage 3.2 design (Version 2.0.0) represents substantial architectural progress:
1. It successfully eliminates the fatal hallucinated `context.rax` modification (FIND-3.2-01) by establishing a cooperative wakeup path through `task.ipc_message` and standard syscall return unwinding.
2. It resolves syscall return ambiguity (FIND-3.2-02) by bounding event masks to bits 0..=62 and reserving bit 63, ensuring `u64::MAX` (`!0`) remains an unambiguous error sentinel.
3. It accurately corrects the 32-byte `CNodeSlot` layout and `CapRights::GRANT` bitflag definition (FIND-3.2-06, FIND-3.2-08).
4. It formally isolates Stage 3.2A from Stage 3.2B hardware dependencies.

However, an adversarial inspection of the v2.0 specification against the actual capability engine revealed **1 HIGH-severity design gap, 2 MEDIUM-severity inconsistencies, and 1 LOW documentation gap** that must be resolved prior to beginning implementation:
- **REV-01 (HIGH):** Object generation validation is claimed in Section 9.2 but is unspecified in capability layout and syscall dispatch. `CNodeSlot` only stores `payload_a = notification_id`; the object generation is never packed into `payload_a` or validated during syscall dispatch.
- **REV-02 (MEDIUM):** In Section 5.2, `mint_cap_internal` combines `CapType::Endpoint | CapType::Notification` under `MAX_NOTIFICATION_EVENT_MASK`, which improperly restricts 64-bit synchronous Endpoint badges to 63 bits.
- **REV-03 (MEDIUM):** Reference count decrements are described conceptually in a table but are not tied to `kernel/src/capability.rs::cleanup_slot`. If an implementer places decrements directly in `delete_cap`/`revoke_cap`, recursive revocation will either double-decrement or leak references.
- **REV-04 (LOW):** The justification for `ref_count: u16` does not document the formal system bound ($N \le 2048$ total slots in Janus OS).

---

## 3. Adversarial Audit: Blocking, Wakeup, and `ipc_message`

### 3.1 Trace of `Task.ipc_message` Across All Kernel Paths
The field `pub ipc_message: u64` in `Task` (`kernel/src/task.rs:61`) is accessed in `kernel/src/ipc.rs` at:
- `ipc.rs:184`: `target.ipc_message = msg; target.state = TaskState::Runnable;` (inside `sys_send`, gated by `target.state == TaskState::BlockedOnRecv`).
- `ipc.rs:213`: `sender.ipc_message = msg;` (inside `sys_send`, when sender transitions to `TaskState::BlockedOnSend`).
- `ipc.rs:258-259`: `let msg = receiver.ipc_message; receiver.ipc_message = 0;` (inside `sys_recv`, for `CapType::Irq`).
- `ipc.rs:305-306`: `let msg = sender.ipc_message; sender.ipc_message = 0; sender.state = TaskState::Runnable;` (inside `sys_recv`, reading from blocked sender).
- `ipc.rs:337-338`: `let msg = receiver.ipc_message; receiver.ipc_message = 0;` (inside `sys_recv`, reading delivered message upon waking).
- `ipc.rs:389`: `t.ipc_message = 0;` (inside `wake_irq`, gated by `t.state == TaskState::BlockedOnRecv`).
- `task.rs:73`: `ipc_message: 0` (inside `Task::new`).

### 3.2 State Isolation Proof
Can an ordinary synchronous IPC message or interrupt overwrite `task.ipc_message` while a notification waiter is blocked?
1. **Mutual Exclusion of `TaskState`:**  
   When a task blocks in `SYS_NOTIFY_WAIT`, its state is set to `TaskState::BlockedOnNotification { notification_id: id as u16 }`.
2. **`sys_send` Guard (`ipc.rs:169-178`):**  
   `sys_send` specifically inspects `target.state`. It only deposits a message into `target.ipc_message` if `target.state == TaskState::BlockedOnRecv { .. }`. If the target is in `BlockedOnNotification`, `sys_send` treats the target as not ready and blocks the sender in `BlockedOnSend`. It **never** writes to `target.ipc_message`.
3. **`wake_irq` Guard (`ipc.rs:385`):**  
   The prototype `wake_irq` only inspects tasks with `state == TaskState::BlockedOnRecv`.
4. **Resumption Path:**  
   When rescheduled, the task resumes in `sys_notify_wait` immediately after `schedule()` (`task.rs:187` unwinds `switch_context`). It reads `let bits = task.ipc_message; task.ipc_message = 0;` and returns `bits`.
5. **Syscall Return Preservation:**  
   `sys_notify_wait` returns `bits as u64` to `syscall_handler` (`syscall.rs:116-124`), which places it in `RAX`. In `syscall_entry` (`syscall.rs:77-101`), the register pop sequence skips the saved stack `RAX` (`add rsp, 8`), and `iretq` restores Ring 3 with `RAX` intact.

### 3.3 State Transition Matrix

| Scenario | State Transitions & Invariants | Bit Retention & Delivery | Lost Wakeup Window? |
|---|---|---|---|
| **1. Signal before Wait** | `pending_bits \|= S`. Task calls `sys_notify_wait`. Checks `pending_bits != 0`. Takes fast path. | Consumes bits, sets `pending_bits = 0`, returns immediately. | **None.** No yield occurs. |
| **2. Wait registers, signal races before block** | Syscall enters with `IF = 0` (`SFMask`). Inspection of `pending_bits` and update of `waiting_task` execute with interrupts disabled. | Serialized by `IF = 0`. Producer cannot execute until consumer calls `schedule()`. | **None.** Physically impossible on UP. |
| **3. Wait blocks, then signal occurs** | Waiter in `BlockedOnNotification`. Producer executes, transfers bits to `waiter.ipc_message`, clears `pending_bits`, clears `waiting_task`, sets waiter `Runnable`. | Delivered via `waiter.ipc_message`. Resumes after `schedule()`, clears field, returns bits. | **None.** Waiter made Runnable. |
| **4. Two signals before waiter runs** | First signal wakes waiter (`ipc_message = bits1`, `waiting_task = None`, `state = Runnable`). Second signal sees `waiting_task == None`; executes `pending_bits \|= bits2`. | `bits1` delivered via `ipc_message`. `bits2` safely retained in `pending_bits` for next wait call. | **None.** Events coalesced and retained. |
| **5. Second signal after waiter Runnable before consumption** | Waiter is `Runnable`. Second signal executes `pending_bits \|= bits2` because `waiting_task` is `None`. Waiter's `ipc_message` is untouched. | Waiter consumes `bits1`. Next `sys_notify_wait` takes fast path on `pending_bits` (`bits2`). | **None.** Strict sequential delivery. |
| **6. Waiter cancelled / terminated** | Capability revoked or task killed. Kernel sets `waiter.ipc_message = !0`, `waiter.state = Runnable`, `waiting_task = None`. | Waiter returns `!0` (failure sentinel), alerting user space. | **None.** Clean error propagation. |
| **7. Object destroyed while waiter blocked** | `ref_count` drops to 0. Teardown clears `waiting_task` and sets `waiter.ipc_message = !0`. | Waiter wakes with `!0`. Object deactivated. | **None.** Deadlock prevented. |
| **8. Multiple producers signal concurrently** | Syscall entry clears `IF`. Signals execute serially under kernel critical section. | All signals bitwise-OR into `pending_bits` or `ipc_message`. | **None.** Strict atomic coalescing. |

**Conclusion on `ipc_message`:** Reusing `task.ipc_message` is completely sound, safe, and verifiable on the current uniprocessor architecture.

---

## 4. Adversarial Audit: Notification Object Lifecycle & Reference Counting

### 4.1 Lifecycle Accounting Architecture
In Janus OS, capability ownership is governed by `CNodeSlot` and the Capability Derivation Tree (`CDT_ARENA`). There is no reverse map from object IDs to slots.

In design v2.0:
- `ref_count: u16`: Tracks live capability references.
- `generation: u32`: Allocation cycle counter.
- `MAX_NOTIFICATIONS = 64`.

### 4.2 Lifecycle Operation Audit

```
                              SYS_NOTIFY_CREATE
                                      │
                                      ▼
                            ref_count = 1, active = true
                                      │
                 ┌────────────────────┴────────────────────┐
                 ▼                                         ▼
             mint_cap                                  copy_cap
                 │                                         │
                 ▼                                         ▼
           ref_count += 1                            ref_count += 1
                 │                                         │
                 └────────────────────┬────────────────────┘
                                      │
                                      ▼
                           delete_cap / revoke_cap
                                      │
                                      ▼
                                cleanup_slot
                                      │
                                      ▼
                               ref_count -= 1
                                      │
                         ┌────────────┴────────────┐
                         ▼                         ▼
                  ref_count > 0              ref_count == 0
                 (Object active)                   │
                                                   ▼
                                            Reset Object:
                                            - Cancel waiter (!0)
                                            - Advance generation
                                            - active = false
```

### 4.3 Detailed Defect Analysis: REV-01 & REV-03

#### Finding REV-01 (HIGH): Unspecified Generation Encoding and Validation
- **Design Claim (Section 9.2):**  
  > "When `ref_count` drops to zero, the slot is deactivated (`active = false`) and its generation counter is incremented: `generation = generation.wrapping_add(1);`. Any stale capability retaining the old index cannot match the incremented generation counter, preventing use-after-free and cross-task aliasing."
- **Actual Code Reality:**  
  `CNodeSlot` (`kernel/src/capability.rs:78-87`) has fields:
  `cap_type: CapType`, `rights: CapRights`, `generation: u32`, `payload_a: u64`, `payload_b: u64`, `cdt_index: u32`.  
  The `generation` field in `CNodeSlot` is the **CNode slot generation**, used exclusively to validate `CPtr.generation() == slot.generation`.  
  In Section 5.1, the design states: `payload_a = notification_id as u64 (0..63)`.  
  **The notification object generation is nowhere stored in the capability slot!**
- **Impact:**  
  If a capability slot does not store `object_generation`, the kernel **cannot** compare the capability against `NOTIFICATIONS[id].generation`. The claim in Section 9.2 is currently unsupported by the specification.
- **Required Remediation:**  
  1. Define `payload_a` encoding for `CapType::Notification` as:
     - Bits 0..31: `notification_id: u32` ($0 \dots 63$).
     - Bits 32..63: `object_generation: u32`.
  2. In `sys_notify_*` and capability lookups: require `notif.generation == (slot.payload_a >> 32) as u32`. Reject with `!0` (`CapLookupError::StaleHandle`) on mismatch.

#### Finding REV-03 (MEDIUM): Unspecified Integration with `cleanup_slot`
- **Design Ambiguity:**  
  Section 5.3 presents a reference accounting table stating `delete_cap` decrements by $1 + \text{descendants}$ and `revoke_cap` decrements by $\text{descendants}$. However, it does not specify *where* in code this occurs.
- **Actual Code Reality:**  
  In `kernel/src/capability.rs:624-665`, `revoke_descendants_iterative` loops through leaf nodes, unlinks them from CDT, and calls `cleanup_slot(leaf_task, leaf_slot)`.  
  In `delete_cap` (`capability.rs:687-713`), it calls `revoke_descendants_iterative`, unlinks the root, and calls `cleanup_slot(task_id, slot)`.
- **Impact:**  
  If an implementer places decrements directly inside `delete_cap` and `revoke_cap`, recursive revocation will either double-decrement or leak references.
- **Required Remediation:**  
  The specification must explicitly state: **`ref_count -= 1` must be implemented inside `cleanup_slot(task_id, slot)`** (`capability.rs:459-473`). Because `cleanup_slot` is the single funnel through which every destroyed or revoked slot passes, decrementing in `cleanup_slot` guarantees exact, transactional accounting without double decrements or leaks.

---

## 5. Adversarial Audit: Notification Semantics and ABI

### 5.1 Bit 63 Reservation & Sentinel Proof
- **Failure Sentinel:** All existing syscalls return `u64::MAX` (`!0 = 0xFFFF_FFFF_FFFF_FFFF`) on error (`ipc.rs:76, 120, 492, 503, 514, 525`).
- **Bit 63 Reservation:** In v2.0, valid event masks are bounded to bits 0..=62:
  `MAX_NOTIFICATION_EVENT_MASK = 0x7FFF_FFFF_FFFF_FFFF`.
- **Validation Entry Points:**
  - `SYS_NOTIFY_SIGNAL`: `if (bits & !MAX_NOTIFICATION_EVENT_MASK) != 0 { return !0; }`
  - `mint_cap_internal`: `if (new_badge & !MAX_NOTIFICATION_EVENT_MASK) != 0 { return Err(CapError::InvalidBadge); }`
- **Result:** Any return value where `(val & (1 << 63)) == 0` is unambiguously a successful bitmask ($0 \dots 2^{63}-1$). `!0` has bit 63 set and represents failure.

### 5.2 Zero-Mask Behavior
- `SYS_NOTIFY_SIGNAL` with `bits == 0` on an unbadged capability: Evaluates to `mask = 0`. Returns `0` (success) as a no-op handle probe. Does not wake waiting tasks.
- `SYS_NOTIFY_POLL` with no events pending: Returns `0`. Callers distinguish no-events (`0`) from failure (`!0`).

### 5.3 Syscall Table Inspection
Audit of `kernel/src/ipc.rs:4-16` confirms:
- Syscall 9: **Free** (assigned to `SYS_NOTIFY_CREATE`)
- Syscall 10: **Free** (assigned to `SYS_NOTIFY_SIGNAL`)
- Syscall 15: **Free** (assigned to `SYS_NOTIFY_WAIT`)
- Syscall 16: **Free** (assigned to `SYS_NOTIFY_POLL`)
- Syscalls 11, 12: Reserved for Stage 3.3 Virtual Memory.
- Syscalls 17, 18: Reserved for Stage 3.2B Hardware IRQ.

---

## 6. Adversarial Audit: Capability Badge Semantics

### 6.1 Defect Analysis: REV-02 (MEDIUM)
In Section 5.2 of design v2.0:
```rust
match src_slot_ref.cap_type {
    ...
    CapType::Endpoint | CapType::Notification => {
        // Enforce valid bitmask boundary
        if (new_badge & !MAX_NOTIFICATION_EVENT_MASK) != 0 {
            return Err(CapError::InvalidBadge);
        }
        ...
    }
}
```
- **Finding:** `CapType::Endpoint` is for synchronous IPC. In Stage 3.1, synchronous endpoints permit arbitrary 64-bit badges (e.g., client identity addresses or full 64-bit tags). Applying `MAX_NOTIFICATION_EVENT_MASK` to `CapType::Endpoint` unintentionally restricts Endpoint badges to 63 bits.
- **Required Remediation:** Separate the match arms:
  ```rust
  CapType::Endpoint => {
      if src_slot_ref.payload_b != 0 && new_badge != src_slot_ref.payload_b {
          return Err(CapError::InvalidBadge);
      }
  }
  CapType::Notification => {
      if (new_badge & !MAX_NOTIFICATION_EVENT_MASK) != 0 {
          return Err(CapError::InvalidBadge);
      }
      if src_slot_ref.payload_b != 0 && new_badge != src_slot_ref.payload_b {
          return Err(CapError::InvalidBadge);
      }
  }
  ```

---

## 7. Deferred IRQ Design Boundaries (Stage 3.2B)

Stage 3.2A strictly excludes hardware IRQ routing. The design v2.0 successfully defers Stage 3.2B and isolates all hardware concerns.

### Validation of Stage 3.2B Prerequisites:
1. **Spurious IRQ 7 / 15 Handling:**
   - In `kernel/src/pic.rs`, an In-Service Register (ISR) read function must be added (`inb(0x20)` with OCW3 `0x0B`).
   - If IRQ 7 is spurious, do not send EOI.
   - If IRQ 15 is spurious, send EOI only to Master PIC (`0x20`).
2. **Missing IDT Vectors:**
   - `kernel/src/idt.rs` currently registers only vectors 32, 42, 43, 46, and 47. Vectors 33..41 (including COM1 / IRQ 4 at vector 36) must be registered with dedicated assembly stubs before hardware testing.
3. **Driver FIFO Drain Contract:**
   - Edge-triggered interrupts coalesce in the 8259 PIC IRR register while masked. Drivers must loop until the device data register/FIFO is empty before invoking `SYS_IRQ_ACK`.

---

## 8. Original Review Finding Closure Verification

Verification of the nine findings from `docs/architecture/STAGE_3_2_NOTIFICATION_REVIEW.md`:

| Original Finding | Original Severity | Remediated Status in Design v2.0 | Re-Review Verification Finding | Status |
|---|---|---|---|---|
| **FIND-3.2-01** | CRITICAL | Addressed via `task.ipc_message` & `switch_context` unwinding | Verified sound, non-preemptible, and leak-free | **CLOSED** |
| **FIND-3.2-02** | HIGH | Addressed via bits 0..=62 mask & bit 63 reservation | Verified `!0` is unambiguous failure sentinel | **CLOSED** |
| **FIND-3.2-03** | HIGH | Addressed via `ref_count: u16` & `generation: u32` | Partially closed; needs generation packing in `payload_a` (REV-01) and `cleanup_slot` integration (REV-03) | **CONDITIONALLY CLOSED** |
| **FIND-3.2-04** | HIGH | Addressed via `mint_cap_internal` badging specification | Needs separation from Endpoint badge masking (REV-02) | **CONDITIONALLY CLOSED** |
| **FIND-3.2-05** | HIGH | Addressed via explicit Stage 3.2B deferral & prerequisites | Verified deferred; prerequisites technically complete | **CLOSED** |
| **FIND-3.2-06** | MEDIUM | Addressed via correcting `CNodeSlot` to 32B layout | Verified aligned with `capability.rs:78-87` | **CLOSED** |
| **FIND-3.2-07** | MEDIUM | Addressed via identifying missing IDT vectors in prerequisites | Verified deferred to Stage 3.2B | **CLOSED** |
| **FIND-3.2-08** | MEDIUM | Addressed via correcting `CapRights::GRANT` to `0x08` | Verified aligned with `capability.rs:52` | **CLOSED** |
| **FIND-3.2-09** | LOW | Addressed via documenting driver drain-until-empty loop | Verified documented in Section 11 & 12 | **CLOSED** |

---

## 9. Comprehensive Findings & Required Remediations

### Finding REV-01 (HIGH): Missing Generation Encoding in Capability `payload_a`
- **Location:** `STAGE_3_2_NOTIFICATION_DESIGN.md` Section 5.1, Section 9.2.
- **Evidence:** Section 9.2 states stale capabilities cannot match incremented object generations, but Section 5.1 defines `payload_a` as only storing `notification_id: u32`.
- **Corrective Action:**  
  Specify that `CNodeSlot.payload_a` for `CapType::Notification` encodes:
  $$\text{payload\_a} = ((\text{object\_generation as u64}) \ll 32) \mid (\text{notification\_id as u64})$$
  In `sys_notify_*` dispatch, verify:
  `if (slot.payload_a >> 32) as u32 != notif.generation { return !0; }`

### Finding REV-02 (MEDIUM): Endpoint Badges Restricted by Notification Mask in `mint_cap_internal`
- **Location:** `STAGE_3_2_NOTIFICATION_DESIGN.md` Section 5.2.
- **Evidence:** `CapType::Endpoint` and `CapType::Notification` are merged under `if (new_badge & !MAX_NOTIFICATION_EVENT_MASK) != 0`, unintentionally constraining Endpoint badges.
- **Corrective Action:**  
  Split match arms in `mint_cap_internal` so that `MAX_NOTIFICATION_EVENT_MASK` applies strictly to `CapType::Notification`.

### Finding REV-03 (MEDIUM): Unspecified Integration with `cleanup_slot`
- **Location:** `STAGE_3_2_NOTIFICATION_DESIGN.md` Section 5.3.
- **Evidence:** Reference decrements are described at syscall level rather than identifying `capability.rs::cleanup_slot`.
- **Corrective Action:**  
  Explicitly specify that `NotificationObject.ref_count` decrement and zero-check occur inside `cleanup_slot(task_id, slot)`.

### Finding REV-04 (LOW): Missing Formal Proof of `ref_count: u16` Upper Bound
- **Location:** `STAGE_3_2_NOTIFICATION_DESIGN.md` Section 4.
- **Evidence:** `ref_count` is defined as `u16` without documenting the physical capacity limit.
- **Corrective Action:**  
  Document the invariant: $\text{ref\_count} \le \text{TOTAL\_CDT\_SLOTS} = 2048 < 65535$, proving overflow is impossible.

---

## 10. Required Unit Test Matrix for Stage 3.2A Implementation

The following 16 host-executable unit tests must be implemented in `kernel/src/notification.rs` / `kernel/src/capability.rs`:

1. `test_notif_create_and_signal_unbadged`: Verify root creation, signaling bits 0..=62, coalescing.
2. `test_notif_signal_bit63_rejected`: Verify signaling with bit 63 set returns error `!0`.
3. `test_notif_badged_mint_immutability`: Verify child inherits parent badge; altering non-zero badge returns `InvalidBadge`.
4. `test_notif_badged_signal_enforcement`: Verify caller bits ignored and fixed badge applied.
5. `test_notif_wait_fast_path`: Verify immediate return when `pending_bits != 0`.
6. `test_notif_wait_slow_path_blocking`: Verify task state transitions to `BlockedOnNotification`.
7. `test_notif_single_waiter_busy`: Verify second waiter receives error `!0` immediately.
8. `test_notif_wakeup_protocol`: Verify signal writes to `task.ipc_message` and marks `Runnable`.
9. `test_notif_poll_read_and_clear`: Verify `SYS_NOTIFY_POLL` consumes bits and resets `pending_bits = 0`.
10. `test_notif_poll_empty_returns_zero`: Verify polling with no events returns `0`.
11. `test_notif_copy_preserves_badge_and_increments_refcount`: Verify `copy_cap` increments `ref_count`.
12. `test_notif_cleanup_slot_decrements_refcount`: Verify `cleanup_slot` decrements `ref_count`.
13. `test_notif_object_recycled_on_zero_refcount`: Verify `active = false` and generation increments when `ref_count == 0`.
14. `test_notif_stale_generation_rejected`: Verify handle with stale generation fails lookup.
15. `test_notif_deletion_wakes_blocked_waiter`: Verify deleting capability wakes waiter with error `!0`.
16. `test_notif_rights_attenuation`: Verify signal without `WRITE` or wait without `READ` returns error `!0`.

---

## 11. Final Readiness Verdicts

### Stage 3.2A (Core Asynchronous Notifications)
**Verdict:** **APPROVED WITH CONDITIONS — NOT READY UNTIL CONDITIONS ARE CLOSED**  
Implementation may commence once:
1. REV-01 is closed: `payload_a` is defined to encode `(generation << 32) | id` and generation checks are specified.
2. REV-02 is closed: `mint_cap_internal` separates Endpoint badges from Notification badge mask checks.
3. REV-03 is closed: `cleanup_slot` is formally identified as the locus of `ref_count` decrements.
4. REV-04 is closed: The 2048-slot upper-bound invariant for `ref_count: u16` is documented.

### Stage 3.2B (Hardware IRQ Routing & Driver Protocol)
**Verdict:** **DEFERRED — PREREQUISITES NOT YET VERIFIED**  
Hardware interrupt binding remains deferred until Stage 3.2A is implemented, audited, and committed, and all Stage 3.2B hardware prerequisites are satisfied.
