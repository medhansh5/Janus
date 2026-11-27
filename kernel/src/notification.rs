use crate::capability::{CPtr, CapError, CapRights, CapType};

pub const MAX_NOTIFICATIONS: usize = 64;
pub const MAX_NOTIFICATION_EVENT_MASK: u64 = 0x7FFF_FFFF_FFFF_FFFF; // Bits 0..=62

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotificationObject {
    pub pending_bits: u64,
    pub waiting_task: Option<u8>,
    pub bound_irq: Option<u8>,
    pub owner_task: Option<u8>,
    pub ref_count: u16,
    pub generation: u32,
    pub active: bool,
}

impl NotificationObject {
    pub const fn empty() -> Self {
        Self {
            pending_bits: 0,
            waiting_task: None,
            bound_irq: None,
            owner_task: None,
            ref_count: 0,
            generation: 1, // Generations start at 1; 0 is sentinel invalid
            active: false,
        }
    }
}

pub static mut NOTIFICATIONS: [NotificationObject; MAX_NOTIFICATIONS] =
    [NotificationObject::empty(); MAX_NOTIFICATIONS];

/// Hardware IRQ line to Notification object bindings: IRQ 0..15 -> Option<(notification_id, irq_bit)>
pub static mut IRQ_BINDINGS: [Option<(u16, u64)>; 16] = [None; 16];

/// Tracks whether each hardware IRQ line (0..15) is currently masked awaiting driver acknowledgement.
pub static mut IRQ_MASKED: [bool; 16] = [false; 16];

/// Records that an IRQ line was masked in the PIC upon interrupt delivery.
///
/// # Safety
/// Must be called from interrupt context or with interrupts disabled.
pub unsafe fn mark_irq_masked(irq: u8) {
    if (irq as usize) < 16 {
        IRQ_MASKED[irq as usize] = true;
    }
}

/// Records that an IRQ line was unmasked in the PIC.
///
/// # Safety
/// Must be called with interrupts disabled.
pub unsafe fn mark_irq_unmasked(irq: u8) {
    if (irq as usize) < 16 {
        IRQ_MASKED[irq as usize] = false;
    }
}

/// Cleans up IRQ ownership state after a `CapType::Irq` slot is removed or revoked.
///
/// Scans all tasks' CNodes to verify whether any other `CapType::Irq` capability for `irq`
/// still exists. Only when zero owners remain does it clear `IRQ_BINDINGS[irq]` and unmask
/// the line if it was left masked awaiting acknowledgement.
///
/// # Safety
/// Caller must ensure single-threaded execution in kernel mode and that the removed slot
/// has already been cleared.
pub unsafe fn cleanup_irq_on_cap_removal(irq: u8) {
    if (irq as usize) >= 16 {
        return;
    }
    for task_id in 0..crate::capability::MAX_TASKS as u8 {
        if let Ok(cnode) = crate::capability::get_task_cnode_mut(task_id) {
            for slot in cnode.slots.iter() {
                if slot.cap_type == CapType::Irq && (slot.payload_a & 0xFF) as u8 == irq {
                    return; // Another live IRQ capability still owns/controls this line
                }
            }
        }
    }
    if let Some((notif_id, _)) = IRQ_BINDINGS[irq as usize].take() {
        if (notif_id as usize) < MAX_NOTIFICATIONS {
            NOTIFICATIONS[notif_id as usize].bound_irq = None;
        }
    }
    if IRQ_MASKED[irq as usize] {
        IRQ_MASKED[irq as usize] = false;
        #[cfg(not(test))]
        crate::pic::unmask(irq);
    }
}

/// Allocates an inactive notification object from the fixed static pool.
///
/// # Safety
/// Caller must ensure single-threaded execution (interrupts disabled in kernel mode).
#[allow(clippy::needless_range_loop)]
pub unsafe fn allocate_notification(owner_task: u8) -> Result<(usize, u32), CapError> {
    for i in 0..MAX_NOTIFICATIONS {
        let notif = &mut NOTIFICATIONS[i];
        if !notif.active && notif.generation < u32::MAX {
            notif.active = true;
            notif.owner_task = Some(owner_task);
            notif.ref_count = 1;
            notif.pending_bits = 0;
            notif.waiting_task = None;
            notif.bound_irq = None;
            let gen = notif.generation;
            return Ok((i, gen));
        }
    }
    Err(CapError::SlotExhausted)
}

/// Resets a notification object when its last capability reference is deleted.
///
/// # Safety
/// Caller must ensure single-threaded execution (interrupts disabled in kernel mode).
pub unsafe fn reset_notification_object(id: usize) {
    if id >= MAX_NOTIFICATIONS {
        return;
    }
    let notif = &mut NOTIFICATIONS[id];

    // Cancel registered waiter if one exists
    if let Some(waiter_id) = notif.waiting_task.take() {
        if let Some(waiter) = crate::task::get_task_mut(waiter_id as u64) {
            waiter.ipc_message = !0; // Error sentinel alerts caller of handle cancellation
            waiter.state = crate::task::TaskState::Runnable;
        }
    }

    // Clean up hardware IRQ binding if this notification is still the active binding
    if let Some(irq) = notif.bound_irq.take() {
        if (irq as usize) < 16 {
            if let Some((bound_id, _)) = IRQ_BINDINGS[irq as usize] {
                if bound_id as usize == id {
                    IRQ_BINDINGS[irq as usize] = None;
                    #[cfg(not(test))]
                    crate::pic::mask(irq);
                }
            }
        }
    }

    // Advance generation; permanently retire if exhausted
    if notif.generation == u32::MAX {
        notif.active = false;
        notif.ref_count = 0;
        notif.pending_bits = 0;
        notif.waiting_task = None;
        notif.bound_irq = None;
        notif.owner_task = None;
        return;
    }

    notif.generation = notif.generation.saturating_add(1);
    notif.active = false;
    notif.ref_count = 0;
    notif.pending_bits = 0;
    notif.waiting_task = None;
    notif.bound_irq = None;
    notif.owner_task = None;
}

/// Increments the reference count of an active notification object.
///
/// # Safety
/// Caller must ensure single-threaded execution in kernel mode.
pub unsafe fn increment_notification_ref(id: usize) -> Result<(), CapError> {
    if id >= MAX_NOTIFICATIONS {
        return Err(CapError::OutOfBounds);
    }
    let notif = &mut NOTIFICATIONS[id];
    if notif.ref_count >= crate::capability::TOTAL_CDT_SLOTS as u16 {
        return Err(CapError::SlotExhausted);
    }
    notif.ref_count += 1;
    Ok(())
}

/// Decrements the reference count of an active notification object and triggers reset on zero.
///
/// # Safety
/// Caller must ensure single-threaded execution in kernel mode.
pub unsafe fn decrement_notification_ref(id: usize) {
    if id >= MAX_NOTIFICATIONS {
        return;
    }
    let notif = &mut NOTIFICATIONS[id];
    if notif.ref_count > 0 {
        notif.ref_count -= 1;
        if notif.ref_count == 0 {
            reset_notification_object(id);
        }
    }
}

/// Helper for unit tests to reset the notification pool to clean initial state.
///
/// # Safety
/// Test-only harness function.
#[cfg(test)]
pub unsafe fn reset_notifications_for_test() {
    NOTIFICATIONS = [NotificationObject::empty(); MAX_NOTIFICATIONS];
    IRQ_BINDINGS = [None; 16];
    IRQ_MASKED = [false; 16];
}

/// Syscall: Allocates a new notification object and mints root capability into caller's dest_slot.
pub fn sys_notify_create(dest_slot: usize) -> u64 {
    let slot_idx = if dest_slot < crate::capability::CNODE_SLOTS {
        dest_slot
    } else {
        CPtr(dest_slot as u64).slot()
    };
    let task_id = match crate::task::current_task_id() {
        Some(id) => id as u8,
        None => return !0,
    };
    let (notif_id, gen) = match unsafe { allocate_notification(task_id) } {
        Ok(res) => res,
        Err(_) => return !0,
    };
    match unsafe { crate::capability::install_notification_root(task_id, slot_idx, notif_id, gen) }
    {
        Ok(_) => 0,
        Err(_) => {
            unsafe { reset_notification_object(notif_id) };
            !0
        }
    }
}

/// Syscall: Signals an asynchronous event bitmask to the target notification object.
pub fn sys_notify_signal(cptr: CPtr, bits: u64) -> u64 {
    if (bits & (1u64 << 63)) != 0 {
        return !0; // Bit 63 strictly reserved
    }
    let task_id = match crate::task::current_task_id() {
        Some(id) => id as u8,
        None => return !0,
    };
    let slot = match unsafe { crate::capability::lookup_slot(task_id, cptr) } {
        Ok(s) => s,
        Err(_) => return !0,
    };
    if slot.cap_type != CapType::Notification {
        return !0;
    }
    if !slot.rights.contains(CapRights::WRITE) {
        return !0;
    }
    let notif_id = (slot.payload_a & 0xFFFF_FFFF) as usize;
    let expected_gen = (slot.payload_a >> 32) as u32;
    if notif_id >= MAX_NOTIFICATIONS || expected_gen == 0 {
        return !0;
    }
    let notif = unsafe { &mut NOTIFICATIONS[notif_id] };
    if !notif.active || notif.generation != expected_gen {
        return !0;
    }

    let mask = if slot.payload_b != 0 {
        slot.payload_b
    } else {
        bits & MAX_NOTIFICATION_EVENT_MASK
    };
    if mask == 0 {
        return 0; // No-op probe succeeds without mutating state
    }

    notif.pending_bits |= mask;

    if let Some(waiter_id) = notif.waiting_task.take() {
        if let Some(waiter) = unsafe { crate::task::get_task_mut(waiter_id as u64) } {
            waiter.ipc_message = notif.pending_bits;
            notif.pending_bits = 0;
            waiter.state = crate::task::TaskState::Runnable;
        }
    }
    0
}

/// Syscall: Blocks calling task until events arrive; consumes and returns pending bitmask.
pub fn sys_notify_wait(cptr: CPtr) -> u64 {
    let task_id = match crate::task::current_task_id() {
        Some(id) => id as u8,
        None => return !0,
    };
    let slot = match unsafe { crate::capability::lookup_slot(task_id, cptr) } {
        Ok(s) => s,
        Err(_) => return !0,
    };
    if slot.cap_type != CapType::Notification {
        return !0;
    }
    if !slot.rights.contains(CapRights::READ) {
        return !0;
    }
    let notif_id = (slot.payload_a & 0xFFFF_FFFF) as usize;
    let expected_gen = (slot.payload_a >> 32) as u32;
    if notif_id >= MAX_NOTIFICATIONS || expected_gen == 0 {
        return !0;
    }
    let notif = unsafe { &mut NOTIFICATIONS[notif_id] };
    if !notif.active || notif.generation != expected_gen {
        return !0;
    }

    // Fast path: events already pending
    if notif.pending_bits != 0 {
        let bits = notif.pending_bits;
        notif.pending_bits = 0;
        return bits;
    }

    // Slow path: register waiter and yield
    if notif.waiting_task.is_some() {
        return !0; // Single-waiter invariant: busy rejection
    }
    notif.waiting_task = Some(task_id);
    {
        let current_task = match unsafe { crate::task::current_task_mut() } {
            Some(t) => t,
            None => {
                notif.waiting_task = None;
                return !0;
            }
        };
        current_task.state = crate::task::TaskState::BlockedOnNotification {
            notification_id: notif_id as u16,
        };
    }

    unsafe { crate::task::schedule() };

    // Resumes here when awakened
    let current_task = match unsafe { crate::task::current_task_mut() } {
        Some(t) => t,
        None => return !0,
    };
    let bits = current_task.ipc_message;
    current_task.ipc_message = 0; // Sanitize
    bits
}

/// Syscall: Non-blocking read-and-clear of pending events.
pub fn sys_notify_poll(cptr: CPtr) -> u64 {
    let task_id = match crate::task::current_task_id() {
        Some(id) => id as u8,
        None => return !0,
    };
    let slot = match unsafe { crate::capability::lookup_slot(task_id, cptr) } {
        Ok(s) => s,
        Err(_) => return !0,
    };
    if slot.cap_type != CapType::Notification {
        return !0;
    }
    if !slot.rights.contains(CapRights::READ) {
        return !0;
    }
    let notif_id = (slot.payload_a & 0xFFFF_FFFF) as usize;
    let expected_gen = (slot.payload_a >> 32) as u32;
    if notif_id >= MAX_NOTIFICATIONS || expected_gen == 0 {
        return !0;
    }
    let notif = unsafe { &mut NOTIFICATIONS[notif_id] };
    if !notif.active || notif.generation != expected_gen {
        return !0;
    }

    let bits = notif.pending_bits;
    notif.pending_bits = 0;
    bits
}

/// Syscall 17: Binds a hardware IRQ line to a notification object.
///
/// Parameters:
/// - `irq_cptr`: Capability of type `Irq` with `WRITE` rights.
/// - `notif_cptr`: Capability of type `Notification` with `WRITE` rights.
/// - `irq_bit`: Bitmask to signal when the IRQ fires (bits 0..=62, non-zero).
#[allow(unused_unsafe)]
pub fn sys_irq_bind(irq_cptr: CPtr, notif_cptr: CPtr, irq_bit: u64) -> u64 {
    let task_id = match crate::task::current_task_id() {
        Some(id) => id as u8,
        None => return !0,
    };

    if irq_bit == 0 || (irq_bit & !MAX_NOTIFICATION_EVENT_MASK) != 0 {
        return !0;
    }

    let irq_slot = match unsafe { crate::capability::lookup_slot(task_id, irq_cptr) } {
        Ok(s) => s,
        Err(_) => return !0,
    };
    if irq_slot.cap_type != CapType::Irq || !irq_slot.rights.contains(CapRights::WRITE) {
        return !0;
    }
    let irq = (irq_slot.payload_a & 0xFF) as u8;
    if irq >= 16 {
        return !0;
    }

    let notif_slot = match unsafe { crate::capability::lookup_slot(task_id, notif_cptr) } {
        Ok(s) => s,
        Err(_) => return !0,
    };
    if notif_slot.cap_type != CapType::Notification || !notif_slot.rights.contains(CapRights::WRITE)
    {
        return !0;
    }
    let notif_id = (notif_slot.payload_a & 0xFFFF_FFFF) as usize;
    let expected_gen = (notif_slot.payload_a >> 32) as u32;
    if notif_id >= MAX_NOTIFICATIONS || expected_gen == 0 {
        return !0;
    }

    unsafe {
        let notif = &mut NOTIFICATIONS[notif_id];
        if !notif.active || notif.generation != expected_gen {
            return !0;
        }

        if let Some((prev_id, _)) = IRQ_BINDINGS[irq as usize] {
            if (prev_id as usize) < MAX_NOTIFICATIONS && prev_id as usize != notif_id {
                NOTIFICATIONS[prev_id as usize].bound_irq = None;
            }
        }
        notif.bound_irq = Some(irq);
        IRQ_BINDINGS[irq as usize] = Some((notif_id as u16, irq_bit));

        #[cfg(not(test))]
        crate::pic::unmask(irq);
    }

    0
}

/// Syscall 18: Acknowledges a hardware IRQ line and unmasks it (single EOI model).
#[allow(unused_unsafe)]
pub fn sys_irq_ack(irq_cptr: CPtr) -> u64 {
    let task_id = match crate::task::current_task_id() {
        Some(id) => id as u8,
        None => return !0,
    };

    let irq_slot = match unsafe { crate::capability::lookup_slot(task_id, irq_cptr) } {
        Ok(s) => s,
        Err(_) => return !0,
    };
    if irq_slot.cap_type != CapType::Irq || !irq_slot.rights.contains(CapRights::WRITE) {
        return !0;
    }
    let irq = (irq_slot.payload_a & 0xFF) as u8;
    if irq >= 16 {
        return !0;
    }

    unsafe {
        #[cfg(not(test))]
        crate::pic::unmask(irq);
        mark_irq_unmasked(irq);
    }

    0
}

/// Handles a hardware IRQ by signalling any bound notification object.
///
/// # Safety
/// Called from interrupt handler with interrupts disabled.
pub unsafe fn handle_hardware_irq(irq: u8) {
    if irq >= 16 {
        return;
    }
    if let Some((notif_id, irq_bit)) = IRQ_BINDINGS[irq as usize] {
        let notif_idx = notif_id as usize;
        if notif_idx < MAX_NOTIFICATIONS {
            let notif = &mut NOTIFICATIONS[notif_idx];
            if notif.active {
                notif.pending_bits |= irq_bit;
                if let Some(waiter_id) = notif.waiting_task.take() {
                    if let Some(waiter) = crate::task::get_task_mut(waiter_id as u64) {
                        waiter.ipc_message = notif.pending_bits;
                        notif.pending_bits = 0;
                        waiter.state = crate::task::TaskState::Runnable;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::{
        copy_cap, delete_cap, mint_cap, revoke_cap, CNode, CNodeSlot, CPtr, CapError, CapRights,
        CapType, CdtNode, CDT, MAX_TASKS, TEST_CNODES, TEST_LOCK, TOTAL_CDT_SLOTS,
    };
    use crate::task::{get_task_mut, register_task, set_current_task, Task, TaskState};
    use x86_64::PhysAddr;

    unsafe fn setup_test_harness() {
        TEST_CNODES = Some([CNode::empty(); MAX_TASKS]);
        CDT = [CdtNode::empty(); TOTAL_CDT_SLOTS];
        reset_notifications_for_test();

        let task0 = Task::new(0, PhysAddr::new(0), CPtr(0), 0);
        let task1 = Task::new(1, PhysAddr::new(0), CPtr(0), 0);
        register_task(task0);
        register_task(task1);
        set_current_task(0);
    }

    #[test]
    fn test_notif_payload_a_generation_encoding() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_harness();
            assert_eq!(sys_notify_create(1), 0);

            let slot = crate::capability::lookup_slot(0, CPtr::new(1, 1)).unwrap();
            assert_eq!(slot.cap_type, CapType::Notification);
            let notif_id = (slot.payload_a & 0xFFFF_FFFF) as usize;
            let expected_gen = (slot.payload_a >> 32) as u32;

            assert_eq!(notif_id, 0);
            assert_eq!(expected_gen, 1);
            assert_eq!(NOTIFICATIONS[0].generation, 1);
            assert_eq!(NOTIFICATIONS[0].ref_count, 1);
            assert!(NOTIFICATIONS[0].active);
        }
    }

    #[test]
    fn test_notif_invalid_id_rejected() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_harness();
            let cnode = crate::capability::get_task_cnode_mut(0).unwrap();
            // Inject a capability referencing out-of-bounds notification ID 64
            cnode.slots[1] = CNodeSlot {
                cap_type: CapType::Notification,
                rights: CapRights::READ | CapRights::WRITE | CapRights::GRANT,
                _reserved1: 0,
                generation: 1,
                payload_a: ((1u64) << 32) | 64u64,
                payload_b: 0,
                cdt_index: 1,
                _reserved2: [0; 4],
            };

            let cptr = CPtr::new(1, 1);
            assert_eq!(sys_notify_signal(cptr, 1), !0);
            assert_eq!(sys_notify_poll(cptr), !0);
            assert_eq!(sys_notify_wait(cptr), !0);
        }
    }

    #[test]
    fn test_notif_stale_generation_rejected() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_harness();
            assert_eq!(sys_notify_create(1), 0);

            let cnode = crate::capability::get_task_cnode_mut(0).unwrap();
            // Mutate payload_a to expect generation 99 instead of 1
            cnode.slots[1].payload_a = (99u64) << 32;

            let cptr = CPtr::new(1, 1);
            assert_eq!(sys_notify_signal(cptr, 1), !0);
            assert_eq!(sys_notify_poll(cptr), !0);
            assert_eq!(sys_notify_wait(cptr), !0);
        }
    }

    #[test]
    fn test_notif_recycled_object_stale_cap_rejected() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_harness();
            assert_eq!(sys_notify_create(1), 0);
            let root_cptr = CPtr::new(1, 1);

            // Copy to slot 2 so we have two handles
            let copy_cptr = copy_cap(0, root_cptr, 2).unwrap();
            assert_eq!(NOTIFICATIONS[0].ref_count, 2);

            // Delete copy cap -> ref_count drops to 1
            assert_eq!(delete_cap(0, copy_cptr), Ok(()));
            assert_eq!(NOTIFICATIONS[0].ref_count, 1);
            assert!(NOTIFICATIONS[0].active);

            // Delete root cap -> ref_count drops to 0, object resets and advances generation to 2
            assert_eq!(delete_cap(0, root_cptr), Ok(()));
            assert_eq!(NOTIFICATIONS[0].ref_count, 0);
            assert!(!NOTIFICATIONS[0].active);
            assert_eq!(NOTIFICATIONS[0].generation, 2);

            // Now reallocate notification slot 0 for a new object
            assert_eq!(sys_notify_create(3), 0);
            assert_eq!(NOTIFICATIONS[0].generation, 2);
            assert_eq!(NOTIFICATIONS[0].ref_count, 1);

            // Construct an old capability pointing to slot 0 with old generation 1
            let cnode = crate::capability::get_task_cnode_mut(0).unwrap();
            cnode.slots[4] = CNodeSlot {
                cap_type: CapType::Notification,
                rights: CapRights::READ | CapRights::WRITE,
                _reserved1: 0,
                generation: 1,
                payload_a: (1u64) << 32, // Old generation 1!
                payload_b: 0,
                cdt_index: 4,
                _reserved2: [0; 4],
            };

            let stale_cptr = CPtr::new(1, 4);
            assert_eq!(sys_notify_signal(stale_cptr, 1), !0);
            assert_eq!(sys_notify_poll(stale_cptr), !0);
            assert_eq!(sys_notify_wait(stale_cptr), !0);
        }
    }

    #[test]
    fn test_notif_generation_exhaustion_permanently_retires() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_harness();
            // Fast-forward generation of slot 0 to u32::MAX - 1
            NOTIFICATIONS[0].generation = u32::MAX - 1;
            assert_eq!(allocate_notification(0), Ok((0, u32::MAX - 1)));
            assert_eq!(NOTIFICATIONS[0].generation, u32::MAX - 1);

            // Reset object -> generation should advance to u32::MAX, active = false, permanently retired
            reset_notification_object(0);
            assert_eq!(NOTIFICATIONS[0].generation, u32::MAX);
            assert!(!NOTIFICATIONS[0].active);

            // Attempting to allocate slot 0 again should skip it because generation == u32::MAX
            assert_eq!(allocate_notification(0), Ok((1, 1))); // Skips 0, allocates 1!
        }
    }

    #[test]
    fn test_notif_copy_mint_identity_preservation() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_harness();
            assert_eq!(sys_notify_create(1), 0);
            let root_cptr = CPtr::new(1, 1);

            // Mint to slot 2 with attenuated rights and badge
            let mint_cptr =
                mint_cap(0, root_cptr, 2, CapRights::WRITE | CapRights::GRANT, 0x42).unwrap();

            // Copy to slot 3
            let _copy_cptr = copy_cap(0, mint_cptr, 3).unwrap();

            let cnode = crate::capability::get_task_cnode_mut(0).unwrap();
            let root_slot = cnode.slots[1];
            let mint_slot = cnode.slots[2];
            let copy_slot = cnode.slots[3];

            // payload_a (generation and ID) must be identical across all three
            assert_eq!(root_slot.payload_a, mint_slot.payload_a);
            assert_eq!(mint_slot.payload_a, copy_slot.payload_a);

            // Badges
            assert_eq!(root_slot.payload_b, 0);
            assert_eq!(mint_slot.payload_b, 0x42);
            assert_eq!(copy_slot.payload_b, 0x42);

            // Ref count must be 3
            assert_eq!(NOTIFICATIONS[0].ref_count, 3);
        }
    }

    #[test]
    fn test_notif_create_initializes_refcount_one() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_harness();
            assert_eq!(sys_notify_create(5), 0);
            assert_eq!(NOTIFICATIONS[0].ref_count, 1);
            assert_eq!(NOTIFICATIONS[0].owner_task, Some(0));
            assert!(NOTIFICATIONS[0].active);
        }
    }

    #[test]
    fn test_notif_mint_increments_refcount() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_harness();
            assert_eq!(sys_notify_create(1), 0);
            assert_eq!(NOTIFICATIONS[0].ref_count, 1);

            let root = CPtr::new(1, 1);
            let child = mint_cap(0, root, 2, CapRights::WRITE | CapRights::GRANT, 0).unwrap();
            assert_eq!(NOTIFICATIONS[0].ref_count, 2);

            let _grandchild = mint_cap(0, child, 3, CapRights::WRITE, 0).unwrap();
            assert_eq!(NOTIFICATIONS[0].ref_count, 3);
        }
    }

    #[test]
    fn test_notif_copy_increments_refcount() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_harness();
            assert_eq!(sys_notify_create(1), 0);
            let root = CPtr::new(1, 1);

            let copy1 = copy_cap(0, root, 2).unwrap();
            assert_eq!(NOTIFICATIONS[0].ref_count, 2);

            let _copy2 = copy_cap(0, copy1, 3).unwrap();
            assert_eq!(NOTIFICATIONS[0].ref_count, 3);
        }
    }

    #[test]
    fn test_notif_delete_decrements_refcount() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_harness();
            assert_eq!(sys_notify_create(1), 0);
            let root = CPtr::new(1, 1);
            let child = copy_cap(0, root, 2).unwrap();
            assert_eq!(NOTIFICATIONS[0].ref_count, 2);

            assert_eq!(delete_cap(0, child), Ok(()));
            assert_eq!(NOTIFICATIONS[0].ref_count, 1);

            assert_eq!(delete_cap(0, root), Ok(()));
            assert_eq!(NOTIFICATIONS[0].ref_count, 0);
            assert!(!NOTIFICATIONS[0].active);
        }
    }

    #[test]
    fn test_notif_recursive_revoke_decrements_descendants() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_harness();
            assert_eq!(sys_notify_create(1), 0);
            let root = CPtr::new(1, 1);

            // Build a derivation tree: root -> child1 -> grandchild
            //                              -> child2
            let c1 = mint_cap(0, root, 2, CapRights::WRITE | CapRights::GRANT, 0).unwrap();
            let _gc = mint_cap(0, c1, 3, CapRights::WRITE, 0).unwrap();
            let _c2 = mint_cap(0, root, 4, CapRights::WRITE, 0).unwrap();

            assert_eq!(NOTIFICATIONS[0].ref_count, 4);

            // Revoking root must decrement all 3 descendants exactly once
            assert_eq!(revoke_cap(0, root), Ok(()));
            assert_eq!(NOTIFICATIONS[0].ref_count, 1); // Only root remains!
            assert!(NOTIFICATIONS[0].active);
        }
    }

    #[test]
    fn test_notif_empty_slot_cleanup_does_not_decrement() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_harness();
            assert_eq!(sys_notify_create(1), 0);
            assert_eq!(NOTIFICATIONS[0].ref_count, 1);

            // Call cleanup_slot on empty slot 2
            crate::capability::cleanup_slot(0, 2);
            assert_eq!(NOTIFICATIONS[0].ref_count, 1);
        }
    }

    #[test]
    fn test_notif_failed_mint_leaves_refcount_unchanged() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_harness();
            assert_eq!(sys_notify_create(1), 0);
            let root = CPtr::new(1, 1);
            assert_eq!(NOTIFICATIONS[0].ref_count, 1);

            // Attempt mint with bit 63 set -> should fail
            assert_eq!(
                mint_cap(0, root, 2, CapRights::WRITE, 1u64 << 63),
                Err(CapError::InvalidBadge)
            );
            assert_eq!(NOTIFICATIONS[0].ref_count, 1);

            // Attempt mint to occupied slot 1 -> should fail
            assert_eq!(
                mint_cap(0, root, 1, CapRights::WRITE, 0),
                Err(CapError::SelfMint)
            );
            assert_eq!(NOTIFICATIONS[0].ref_count, 1);
        }
    }

    #[test]
    fn test_notif_last_reference_removal_cleans_object() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_harness();
            assert_eq!(sys_notify_create(1), 0);
            let root = CPtr::new(1, 1);

            NOTIFICATIONS[0].pending_bits = 0x1234;
            assert_eq!(delete_cap(0, root), Ok(()));

            assert_eq!(NOTIFICATIONS[0].ref_count, 0);
            assert_eq!(NOTIFICATIONS[0].pending_bits, 0);
            assert!(!NOTIFICATIONS[0].active);
            assert_eq!(NOTIFICATIONS[0].generation, 2);
        }
    }

    #[test]
    fn test_notif_refcount_overflow_defense() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_harness();
            assert_eq!(sys_notify_create(1), 0);
            let root = CPtr::new(1, 1);

            // Artificially saturate ref_count to TOTAL_CDT_SLOTS
            NOTIFICATIONS[0].ref_count = TOTAL_CDT_SLOTS as u16;

            assert_eq!(
                mint_cap(0, root, 2, CapRights::WRITE, 0),
                Err(CapError::SlotExhausted)
            );
        }
    }

    #[test]
    fn test_notif_signal_without_waiter_coalesces() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_harness();
            assert_eq!(sys_notify_create(1), 0);
            let root = CPtr::new(1, 1);

            assert_eq!(sys_notify_signal(root, 1 << 0), 0);
            assert_eq!(NOTIFICATIONS[0].pending_bits, 1);

            assert_eq!(sys_notify_signal(root, 1 << 1), 0);
            assert_eq!(NOTIFICATIONS[0].pending_bits, 3);

            // Repeated signal coalesces
            assert_eq!(sys_notify_signal(root, 1 << 0), 0);
            assert_eq!(NOTIFICATIONS[0].pending_bits, 3);
        }
    }

    #[test]
    fn test_notif_signal_followed_by_wait_fast_path() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_harness();
            assert_eq!(sys_notify_create(1), 0);
            let root = CPtr::new(1, 1);

            assert_eq!(sys_notify_signal(root, 0b1010), 0);

            // Fast path wait
            assert_eq!(sys_notify_wait(root), 0b1010);
            assert_eq!(NOTIFICATIONS[0].pending_bits, 0);
        }
    }

    #[test]
    fn test_notif_wait_followed_by_signal() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_harness();
            assert_eq!(sys_notify_create(1), 0);
            let root = CPtr::new(1, 1);

            // Register task 1 as waiting
            NOTIFICATIONS[0].waiting_task = Some(1);
            let t1 = get_task_mut(1).unwrap();
            t1.state = TaskState::BlockedOnNotification { notification_id: 0 };
            t1.ipc_message = 0;

            // Task 0 signals
            assert_eq!(sys_notify_signal(root, 0xABC), 0);

            // Verify task 1 woke up with delivered bits in ipc_message
            let t1 = get_task_mut(1).unwrap();
            assert_eq!(t1.state, TaskState::Runnable);
            assert_eq!(t1.ipc_message, 0xABC);
            assert_eq!(NOTIFICATIONS[0].waiting_task, None);
            assert_eq!(NOTIFICATIONS[0].pending_bits, 0);
        }
    }

    #[test]
    fn test_notif_multiple_coalesced_signals() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_harness();
            assert_eq!(sys_notify_create(1), 0);
            let root = CPtr::new(1, 1);

            for i in 0..10 {
                assert_eq!(sys_notify_signal(root, 1 << i), 0);
            }
            assert_eq!(NOTIFICATIONS[0].pending_bits, (1 << 10) - 1);
            assert_eq!(sys_notify_wait(root), (1 << 10) - 1);
            assert_eq!(NOTIFICATIONS[0].pending_bits, 0);
        }
    }

    #[test]
    fn test_notif_signal_after_wake_before_consumption() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_harness();
            assert_eq!(sys_notify_create(1), 0);
            let root = CPtr::new(1, 1);

            // Task 1 waiting
            NOTIFICATIONS[0].waiting_task = Some(1);
            let t1 = get_task_mut(1).unwrap();
            t1.state = TaskState::BlockedOnNotification { notification_id: 0 };

            // Signal 1 arrives -> wakes task 1
            assert_eq!(sys_notify_signal(root, 0x111), 0);
            assert_eq!(get_task_mut(1).unwrap().ipc_message, 0x111);
            assert_eq!(NOTIFICATIONS[0].waiting_task, None);

            // Signal 2 arrives while task 1 is Runnable but hasn't consumed
            assert_eq!(sys_notify_signal(root, 0x222), 0);
            // First signal remains in ipc_message
            assert_eq!(get_task_mut(1).unwrap().ipc_message, 0x111);
            // Second signal retained in pending_bits
            assert_eq!(NOTIFICATIONS[0].pending_bits, 0x222);

            // Task 1 runs and finishes wait, consuming 0x111
            set_current_task(1);
            let t1 = get_task_mut(1).unwrap();
            let bits = t1.ipc_message;
            t1.ipc_message = 0;
            assert_eq!(bits, 0x111);

            // Next wait on task 1 consumes pending_bits 0x222 via fast path!
            // First install cap in task 1's CSpace
            crate::capability::get_task_cnode_mut(1).unwrap().slots[1] =
                crate::capability::get_task_cnode_mut(0).unwrap().slots[1];
            assert_eq!(sys_notify_wait(root), 0x222);
            assert_eq!(NOTIFICATIONS[0].pending_bits, 0);
        }
    }

    #[test]
    fn test_notif_poll_empty_returns_zero() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_harness();
            assert_eq!(sys_notify_create(1), 0);
            let root = CPtr::new(1, 1);

            assert_eq!(sys_notify_poll(root), 0);
        }
    }

    #[test]
    fn test_notif_poll_with_pending_bits() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_harness();
            assert_eq!(sys_notify_create(1), 0);
            let root = CPtr::new(1, 1);

            assert_eq!(sys_notify_signal(root, 0x55), 0);
            assert_eq!(sys_notify_poll(root), 0x55);
            assert_eq!(sys_notify_poll(root), 0); // Read and clear
        }
    }

    #[test]
    fn test_notif_single_waiter_busy_rejection() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_harness();
            assert_eq!(sys_notify_create(1), 0);
            let root = CPtr::new(1, 1);

            // Register task 1 as waiter
            NOTIFICATIONS[0].waiting_task = Some(1);

            // Task 0 attempts wait on the same notification -> busy rejection
            assert_eq!(sys_notify_wait(root), !0);
        }
    }

    #[test]
    fn test_notif_invalid_mask_bit63_rejected() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_harness();
            assert_eq!(sys_notify_create(1), 0);
            let root = CPtr::new(1, 1);

            // Bit 63 set
            assert_eq!(sys_notify_signal(root, 1u64 << 63), !0);
            assert_eq!(sys_notify_signal(root, u64::MAX), !0);
            assert_eq!(NOTIFICATIONS[0].pending_bits, 0);
        }
    }

    #[test]
    fn test_notif_rights_failures() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_harness();
            assert_eq!(sys_notify_create(1), 0);
            let root = CPtr::new(1, 1);

            // Read-only cap
            let ro_cap = mint_cap(0, root, 2, CapRights::READ, 0).unwrap();
            assert_eq!(sys_notify_signal(ro_cap, 1), !0); // Cannot signal without WRITE
            assert_eq!(sys_notify_poll(ro_cap), 0); // Can poll with READ

            // Write-only cap
            let wo_cap = mint_cap(0, root, 3, CapRights::WRITE, 0).unwrap();
            assert_eq!(sys_notify_signal(wo_cap, 1), 0); // Can signal with WRITE
            assert_eq!(sys_notify_poll(wo_cap), !0); // Cannot poll without READ
            assert_eq!(sys_notify_wait(wo_cap), !0); // Cannot wait without READ
        }
    }

    #[test]
    fn test_notif_badged_signal_enforces_badge_and_ignores_caller_bits() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_harness();
            assert_eq!(sys_notify_create(1), 0);
            let root = CPtr::new(1, 1);

            // Mint badged cap with badge 0b1000
            let badged = mint_cap(0, root, 2, CapRights::WRITE, 0b1000).unwrap();

            // Caller passes bits 0b0001 -> should be ignored, badge 0b1000 applied!
            assert_eq!(sys_notify_signal(badged, 0b0001), 0);
            assert_eq!(NOTIFICATIONS[0].pending_bits, 0b1000);
        }
    }

    #[test]
    fn test_notif_deletion_wakes_blocked_waiter() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_harness();
            assert_eq!(sys_notify_create(1), 0);
            let root = CPtr::new(1, 1);

            // Task 1 blocked
            NOTIFICATIONS[0].waiting_task = Some(1);
            let t1 = get_task_mut(1).unwrap();
            t1.state = TaskState::BlockedOnNotification { notification_id: 0 };
            t1.ipc_message = 0;

            // Delete root cap
            assert_eq!(delete_cap(0, root), Ok(()));

            // Waiter must be unblocked with !0
            let t1 = get_task_mut(1).unwrap();
            assert_eq!(t1.state, TaskState::Runnable);
            assert_eq!(t1.ipc_message, !0);
            assert_eq!(NOTIFICATIONS[0].waiting_task, None);
        }
    }

    #[test]
    fn test_endpoint_full_64bit_badge_permitted() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_harness();
            let cnode = crate::capability::get_task_cnode_mut(0).unwrap();
            cnode.slots[1] = CNodeSlot {
                cap_type: CapType::Endpoint,
                rights: CapRights::WRITE | CapRights::GRANT,
                _reserved1: 0,
                generation: 1,
                payload_a: 1,
                payload_b: 0,
                cdt_index: 1,
                _reserved2: [0; 4],
            };

            let ep_root = CPtr::new(1, 1);
            // Full 64-bit badge with bit 63 set
            let badge64 = 0x8000_0000_0000_0001u64;
            let child = mint_cap(0, ep_root, 2, CapRights::WRITE, badge64).unwrap();

            let child_slot = crate::capability::lookup_slot(0, child).unwrap();
            assert_eq!(child_slot.payload_b, badge64);
        }
    }

    #[test]
    fn test_endpoint_bit63_permitted() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_harness();
            let cnode = crate::capability::get_task_cnode_mut(0).unwrap();
            cnode.slots[1] = CNodeSlot {
                cap_type: CapType::Endpoint,
                rights: CapRights::WRITE | CapRights::GRANT,
                _reserved1: 0,
                generation: 1,
                payload_a: 1,
                payload_b: 0,
                cdt_index: 1,
                _reserved2: [0; 4],
            };

            let ep_root = CPtr::new(1, 1);
            let child = mint_cap(0, ep_root, 2, CapRights::WRITE, 1u64 << 63).unwrap();
            assert_eq!(
                crate::capability::lookup_slot(0, child).unwrap().payload_b,
                1u64 << 63
            );
        }
    }

    #[test]
    fn test_endpoint_badge_immutability_preserved() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_harness();
            let cnode = crate::capability::get_task_cnode_mut(0).unwrap();
            cnode.slots[1] = CNodeSlot {
                cap_type: CapType::Endpoint,
                rights: CapRights::WRITE | CapRights::GRANT,
                _reserved1: 0,
                generation: 1,
                payload_a: 1,
                payload_b: 0x1234,
                cdt_index: 1,
                _reserved2: [0; 4],
            };

            let ep_root = CPtr::new(1, 1);
            // Attempt to rebadge -> must fail
            assert_eq!(
                mint_cap(0, ep_root, 2, CapRights::WRITE, 0x5678),
                Err(CapError::InvalidBadge)
            );
            // Same badge -> succeeds
            assert!(mint_cap(0, ep_root, 2, CapRights::WRITE, 0x1234).is_ok());
        }
    }

    #[test]
    fn test_endpoint_copy_and_mint_preserved() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_harness();
            let cnode = crate::capability::get_task_cnode_mut(0).unwrap();
            cnode.slots[1] = CNodeSlot {
                cap_type: CapType::Endpoint,
                rights: CapRights::WRITE | CapRights::GRANT,
                _reserved1: 0,
                generation: 1,
                payload_a: 1,
                payload_b: 0xDEADBEEF,
                cdt_index: 1,
                _reserved2: [0; 4],
            };

            let ep_root = CPtr::new(1, 1);
            let copy = copy_cap(0, ep_root, 2).unwrap();
            assert_eq!(
                crate::capability::lookup_slot(0, copy).unwrap().payload_b,
                0xDEADBEEF
            );
        }
    }

    #[test]
    fn test_irq_bind_and_ack() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_harness();
            assert_eq!(sys_notify_create(1), 0);
            let notif_cptr = CPtr::new(1, 1);

            // Inject IRQ 11 capability into slot 2
            let cnode = crate::capability::get_task_cnode_mut(0).unwrap();
            cnode.slots[2] = CNodeSlot {
                cap_type: CapType::Irq,
                rights: CapRights::READ | CapRights::WRITE,
                _reserved1: 0,
                generation: 1,
                payload_a: 11, // IRQ 11
                payload_b: 0,
                cdt_index: 2,
                _reserved2: [0; 4],
            };
            let irq_cptr = CPtr::new(1, 2);

            // Bind IRQ 11 to notification with event bit 1 << 3
            assert_eq!(sys_irq_bind(irq_cptr, notif_cptr, 1 << 3), 0);
            assert_eq!(IRQ_BINDINGS[11], Some((0, 1 << 3)));
            assert_eq!(NOTIFICATIONS[0].bound_irq, Some(11));

            // Ack IRQ 11
            assert_eq!(sys_irq_ack(irq_cptr), 0);

            // Trigger hardware IRQ 11
            handle_hardware_irq(11);
            assert_eq!(NOTIFICATIONS[0].pending_bits, 1 << 3);

            // Poll pending bits
            assert_eq!(sys_notify_poll(notif_cptr), 1 << 3);
            assert_eq!(NOTIFICATIONS[0].pending_bits, 0);

            // Delete notification -> unbinds IRQ
            assert_eq!(delete_cap(0, notif_cptr), Ok(()));
            assert_eq!(IRQ_BINDINGS[11], None);
        }
    }

    unsafe fn install_test_irq_cap(task_id: u8, slot: usize, irq: u8, rights: CapRights) -> CPtr {
        let cnode = crate::capability::get_task_cnode_mut(task_id).unwrap();
        cnode.slots[slot] = CNodeSlot {
            cap_type: CapType::Irq,
            rights,
            _reserved1: 0,
            generation: 1,
            payload_a: irq as u64,
            payload_b: 0,
            cdt_index: crate::capability::cdt_index(task_id, slot as u8) as u32,
            _reserved2: [0; 4],
        };
        CPtr::new(1, slot as u16)
    }

    #[test]
    fn test_irq_task_destruction_cleans_binding_and_unmasks_when_masked() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_harness();
            assert_eq!(sys_notify_create(1), 0);
            let notif_cptr = CPtr::new(1, 1);
            let irq_cptr = install_test_irq_cap(
                0,
                2,
                11,
                CapRights::READ | CapRights::WRITE | CapRights::GRANT,
            );

            assert_eq!(sys_irq_bind(irq_cptr, notif_cptr, 1 << 2), 0);
            mark_irq_masked(11);
            assert!(IRQ_MASKED[11]);

            // Destroy Task 0's CSpace while IRQ 11 is bound and masked
            crate::capability::destroy_task_cspace(0);

            assert_eq!(IRQ_BINDINGS[11], None);
            assert!(!IRQ_MASKED[11]);
            // Stale IRQ CPtr must be rejected
            assert_eq!(sys_irq_ack(irq_cptr), !0);
        }
    }

    #[test]
    fn test_irq_revocation_and_ack_after_revocation() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_harness();
            assert_eq!(sys_notify_create(1), 0);
            let notif_cptr = CPtr::new(1, 1);
            let root_irq = install_test_irq_cap(
                0,
                2,
                11,
                CapRights::READ | CapRights::WRITE | CapRights::GRANT,
            );
            let child_irq =
                mint_cap(0, root_irq, 3, CapRights::READ | CapRights::WRITE, 0).unwrap();

            assert_eq!(sys_irq_bind(child_irq, notif_cptr, 1 << 4), 0);
            mark_irq_masked(11);

            // Revoke child via root: root still owns IRQ 11, so binding/mask remain
            assert_eq!(revoke_cap(0, root_irq), Ok(()));
            assert_eq!(IRQ_BINDINGS[11], Some((0, 1 << 4)));
            assert!(IRQ_MASKED[11]);

            // Ack using revoked child capability must fail and leave IRQ masked
            assert_eq!(sys_irq_ack(child_irq), !0);
            assert!(IRQ_MASKED[11]);

            // Ack using root capability succeeds and unmasks
            assert_eq!(sys_irq_ack(root_irq), 0);
            assert!(!IRQ_MASKED[11]);
        }
    }

    #[test]
    fn test_irq_shared_ownership_and_non_owner_destruction() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_test_harness();
            assert_eq!(sys_notify_create(1), 0);
            let notif_cptr = CPtr::new(1, 1);
            let root_irq = install_test_irq_cap(
                0,
                2,
                11,
                CapRights::READ | CapRights::WRITE | CapRights::GRANT,
            );
            assert_eq!(sys_irq_bind(root_irq, notif_cptr, 1 << 1), 0);
            mark_irq_masked(11);

            // Register Task 2 (non-owner) and destroy it -> must NOT affect Task 0's IRQ 11
            let task2 = Task::new(2, PhysAddr::new(0), CPtr(0), 0);
            register_task(task2);
            crate::capability::destroy_task_cspace(2);
            assert_eq!(IRQ_BINDINGS[11], Some((0, 1 << 1)));
            assert!(IRQ_MASKED[11]);

            // Give Task 1 its own notification and independent IRQ 11 capability
            set_current_task(1);
            assert_eq!(sys_notify_create(1), 0);
            let task1_notif = CPtr::new(1, 1);
            let task1_irq = install_test_irq_cap(1, 2, 11, CapRights::READ | CapRights::WRITE);
            assert_eq!(sys_irq_bind(task1_irq, task1_notif, 1 << 5), 0);
            set_current_task(0);

            // Destroy Task 0: Task 1 still owns IRQ 11, so binding and mask must remain!
            crate::capability::destroy_task_cspace(0);
            assert_eq!(IRQ_BINDINGS[11], Some((1, 1 << 5)));
            assert!(IRQ_MASKED[11]);

            // Now destroy Task 1 (the last owner) -> must unbind and unmask IRQ 11
            crate::capability::destroy_task_cspace(1);
            assert_eq!(IRQ_BINDINGS[11], None);
            assert!(!IRQ_MASKED[11]);
            set_current_task(1);
            assert_eq!(sys_irq_ack(task1_irq), !0);
        }
    }
}
