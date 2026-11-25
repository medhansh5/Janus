use crate::capability::{CNodeSlot, CPtr, CapRights, CapType};
use crate::task::TaskState;

pub const SYS_SEND: u64 = 1;
pub const SYS_RECV: u64 = 2;
pub const SYS_REPLY: u64 = 3;
pub const SYS_YIELD: u64 = 4;
pub const SYS_CAP_INFO: u64 = 5;
pub const SYS_IN_PORT: u64 = 6;
pub const SYS_OUT_PORT: u64 = 7;
pub const SYS_LOG: u64 = 8;
pub const SYS_CAP_MINT: u64 = 13;
pub const SYS_CAP_REVOKE: u64 = 14;
pub const SYS_CAP_COPY: u64 = 20;
pub const SYS_CAP_DELETE: u64 = 21;

/// The main entry point for syscalls dispatched from assembly.
pub fn handle_syscall(sys_num: u64, arg1: u64, arg2: u64, arg3: u64, arg4: u64, _arg5: u64) -> u64 {
    match sys_num {
        SYS_SEND => sys_send(CPtr(arg1), arg2),
        SYS_RECV => sys_recv(CPtr(arg1), arg2),
        SYS_REPLY => sys_reply(arg1),
        SYS_YIELD => {
            unsafe { crate::task::schedule() };
            0
        }
        SYS_CAP_INFO => sys_cap_info(CPtr(arg1)),
        SYS_IN_PORT => sys_in_port(CPtr(arg1), arg2 as u16, arg3 as u8),
        SYS_OUT_PORT => sys_out_port(CPtr(arg1), arg2 as u16, arg3 as u32, arg4 as u8),
        SYS_LOG => {
            let mut serial = crate::SerialPort(0x3F8);
            match crate::paging::copy_user_u32(arg1) {
                Ok(val) => {
                    serial.write_str("[KERNEL] USERSPACE LOG DATA: ");
                    serial.write_hex(val as u64);
                    serial.write_str("\n");
                    0
                }
                Err(err) => {
                    serial.write_str("[KERNEL SEC-FAULT] SYS_LOG invalid user pointer: ");
                    serial.write_hex(arg1);
                    serial.write_str(" (error: ");
                    match err {
                        crate::paging::UserCopyError::NullPointer => {
                            serial.write_str("NullPointer")
                        }
                        crate::paging::UserCopyError::KernelAddress => {
                            serial.write_str("KernelAddress")
                        }
                        crate::paging::UserCopyError::NonCanonical => {
                            serial.write_str("NonCanonical")
                        }
                        crate::paging::UserCopyError::ArithmeticOverflow => {
                            serial.write_str("ArithmeticOverflow")
                        }
                        crate::paging::UserCopyError::NotMapped => serial.write_str("NotMapped"),
                        crate::paging::UserCopyError::SupervisorOnly => {
                            serial.write_str("SupervisorOnly")
                        }
                        crate::paging::UserCopyError::InvalidPhysicalAddress => {
                            serial.write_str("InvalidPhysicalAddress")
                        }
                        crate::paging::UserCopyError::InvalidPageTableEntry => {
                            serial.write_str("InvalidPageTableEntry")
                        }
                    }
                    serial.write_str(")\n");
                    !0
                }
            }
        }
        SYS_CAP_MINT => sys_cap_mint(CPtr(arg1), arg2 as usize, arg3 as u8, arg4),
        SYS_CAP_REVOKE => sys_cap_revoke(CPtr(arg1)),
        SYS_CAP_COPY => sys_cap_copy(CPtr(arg1), arg2 as usize),
        SYS_CAP_DELETE => sys_cap_delete(CPtr(arg1)),
        _ => !0, // ENOSYS
    }
}

/// Errors that can occur during capability lookup in a task's CNode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapLookupError {
    /// Slot index is greater than or equal to `CNODE_SLOTS` (128).
    OutOfBounds,
    /// Handle has reserved bits set or zero generation.
    InvalidHandle,
    /// CNode pointer is null, unaligned, or outside the direct physical memory map.
    InvalidCNodePointer,
    /// Generation mismatch between handle and slot.
    StaleHandle,
}

/// Safely retrieves a copy of the `CNodeSlot` from the task's root CNode.
fn lookup_cap(root_cnode: CPtr, slot: CPtr) -> Result<CNodeSlot, CapLookupError> {
    if !slot.is_canonical() {
        return Err(CapLookupError::InvalidHandle);
    }
    let vaddr = root_cnode.0;
    if vaddr < crate::paging::PHYS_MEMORY_OFFSET
        || (vaddr & 0xFFF) != 0
        || vaddr >= crate::paging::PHYS_MEMORY_OFFSET + crate::paging::DIRECT_MAP_LIMIT
    {
        return Err(CapLookupError::InvalidCNodePointer);
    }
    let cnode = unsafe { &*(vaddr as *const crate::capability::CNode) };
    let s = cnode.slots[slot.slot()];
    if s.generation != slot.generation() {
        return Err(CapLookupError::StaleHandle);
    }
    Ok(s)
}

/// Send a message to an endpoint.
fn sys_send(endpoint_cptr: CPtr, msg: u64) -> u64 {
    // 1. Snapshot sender id and root_cnode in a short, isolated scope
    let (sender_id, sender_cnode) = {
        let sender = match unsafe { crate::task::current_task_mut() } {
            Some(s) => s,
            None => return !0,
        };
        (sender.id, sender.root_cnode)
    }; // Borrow of sender drops here

    // 2. Validate the capability and WRITE rights
    let target_task_id = match lookup_cap(sender_cnode, endpoint_cptr) {
        Ok(s) if s.cap_type == CapType::Endpoint => {
            if !s.rights.contains(CapRights::WRITE) {
                return !0;
            }
            s.payload_a
        }
        Err(CapLookupError::OutOfBounds)
        | Err(CapLookupError::InvalidHandle)
        | Err(CapLookupError::InvalidCNodePointer)
        | Err(CapLookupError::StaleHandle) => {
            // Out-of-bounds, invalid pointer, or stale slot: return !0 without mutating task state
            return !0;
        }
        Ok(_) => {
            // Valid slot index (< 128) containing an invalid type (Empty, Frame, IOPort):
            // Preserves existing security-fault logging and task termination contract.
            let mut serial = crate::SerialPort(0x3F8);
            serial.write_str("[KERNEL SEC-FAULT] Task ");
            serial.write_hex(sender_id);
            serial.write_str(" attempted to use invalid capability slot! Terminating.\n");

            if let Some(sender) = unsafe { crate::task::current_task_mut() } {
                sender.state = crate::task::TaskState::Terminated;
            }
            unsafe { crate::task::schedule() };
            return !0;
        }
    };

    // 3. Reject self-send and invalid IDs before any mutation
    if target_task_id == sender_id || target_task_id >= crate::task::MAX_TASKS as u64 {
        return !0;
    }

    // 4. Inspect target state in a short, isolated scope (no unwrap)
    let target_authorized_and_waiting = {
        let target = match unsafe { crate::task::get_task_mut(target_task_id) } {
            Some(t) => t,
            None => return !0, // Nonexistent target
        };
        if target.state == TaskState::Terminated {
            return !0; // Dead peer
        }
        if let TaskState::BlockedOnRecv { endpoint: recv_ep } = target.state {
            match lookup_cap(target.root_cnode, recv_ep) {
                Ok(s) if s.cap_type == CapType::Endpoint => {
                    s.rights.contains(CapRights::READ) && s.payload_a == sender_id
                }
                _ => false, // Target waiting on IRQ or invalid cap: NEVER wake
            }
        } else {
            false
        }
    }; // Borrow of target drops here

    // 5. Action: Exactly one task mutated at a time
    if target_authorized_and_waiting {
        if let Some(target) = unsafe { crate::task::get_task_mut(target_task_id) } {
            target.ipc_message = msg;
            target.state = TaskState::Runnable;

            let mut serial = crate::SerialPort(0x3F8);
            serial.write_str("[KERNEL IPC] Task ");
            serial.write_hex(sender_id);
            serial.write_str(" sent message to waiting Task ");
            serial.write_hex(target_task_id);
            serial.write_str("\n");

            0 // Success
        } else {
            !0
        }
    } else {
        // Target is not waiting for us. We block.
        let mut serial = crate::SerialPort(0x3F8);
        serial.write_str("[KERNEL IPC] Task ");
        serial.write_hex(sender_id);
        serial.write_str(" blocking on send to Task ");
        serial.write_hex(target_task_id);
        serial.write_str("\n");

        {
            if let Some(sender) = unsafe { crate::task::current_task_mut() } {
                sender.state = TaskState::BlockedOnSend {
                    endpoint: endpoint_cptr,
                };
                // Save the message in ipc_message so it can be retrieved when they wake us
                sender.ipc_message = msg;
            } else {
                return !0;
            }
        } // Borrow of sender drops here BEFORE schedule!
        unsafe { crate::task::schedule() };
        0
    }
}

fn sys_recv(endpoint_cptr: CPtr, _msg_ptr: u64) -> u64 {
    // 1. Snapshot receiver id and root_cnode in a short, isolated scope
    let (receiver_id, receiver_cnode) = {
        let receiver = match unsafe { crate::task::current_task_mut() } {
            Some(r) => r,
            None => return !0,
        };
        (receiver.id, receiver.root_cnode)
    }; // Borrow of receiver drops here

    // 2. Validate receiver capability
    let cap = match lookup_cap(receiver_cnode, endpoint_cptr) {
        Ok(c) => c,
        Err(_) => return !0,
    };

    match cap.cap_type {
        // Check if this is an Irq capability (replaces InterruptEndpoint)
        CapType::Irq => {
            if !cap.rights.contains(CapRights::READ) {
                return !0;
            }
            {
                if let Some(receiver) = unsafe { crate::task::current_task_mut() } {
                    receiver.state = TaskState::BlockedOnRecv {
                        endpoint: endpoint_cptr,
                    };
                } else {
                    return !0;
                }
            } // Borrow of receiver drops here BEFORE schedule!
            unsafe { crate::task::schedule() };

            // Hardware IRQ will wake us up and set ipc_message = 0
            if let Some(receiver) = unsafe { crate::task::current_task_mut() } {
                let msg = receiver.ipc_message;
                receiver.ipc_message = 0; // Sanitize
                msg
            } else {
                !0
            }
        }

        CapType::Endpoint => {
            if !cap.rights.contains(CapRights::READ) {
                return !0;
            }
            let expected_sender_id = cap.payload_a;

            // Reject self-receive and invalid IDs before any mutation
            if expected_sender_id == receiver_id
                || expected_sender_id >= crate::task::MAX_TASKS as u64
            {
                return !0;
            }

            // Inspect expected sender directly without full-table scan (no unwrap)
            let sender_waiting_and_authorized = {
                let sender = match unsafe { crate::task::get_task_mut(expected_sender_id) } {
                    Some(s) => s,
                    None => return !0, // Nonexistent sender
                };
                if sender.state == TaskState::Terminated {
                    return !0; // Dead peer
                }
                if let TaskState::BlockedOnSend { endpoint: send_ep } = sender.state {
                    match lookup_cap(sender.root_cnode, send_ep) {
                        Ok(s) if s.cap_type == CapType::Endpoint => {
                            s.rights.contains(CapRights::WRITE) && s.payload_a == receiver_id
                        }
                        _ => false,
                    }
                } else {
                    false
                }
            }; // Borrow of sender drops here

            if sender_waiting_and_authorized {
                // Immediate rendezvous: copy message before clearing it and waking sender
                let msg = if let Some(sender) =
                    unsafe { crate::task::get_task_mut(expected_sender_id) }
                {
                    let msg = sender.ipc_message;
                    sender.ipc_message = 0; // Sanitize
                    sender.state = TaskState::Runnable;
                    msg
                } else {
                    return !0;
                };

                let mut serial = crate::SerialPort(0x3F8);
                serial.write_str("[KERNEL IPC] Task ");
                serial.write_hex(receiver_id);
                serial.write_str(" received message from blocked Task ");
                serial.write_hex(expected_sender_id);
                serial.write_str("\n");

                msg
            } else {
                // Sender is not ready or blocked on another peer: preserve synchronous blocking wait
                {
                    if let Some(receiver) = unsafe { crate::task::current_task_mut() } {
                        receiver.state = TaskState::BlockedOnRecv {
                            endpoint: endpoint_cptr,
                        };
                    } else {
                        return !0;
                    }
                } // Borrow of receiver drops here BEFORE schedule!
                unsafe { crate::task::schedule() };

                // When we wake up, sender will have written message into our ipc_message.
                // Read delivered message before clearing the receiver's message field.
                if let Some(receiver) = unsafe { crate::task::current_task_mut() } {
                    let msg = receiver.ipc_message;
                    receiver.ipc_message = 0; // Sanitize
                    msg
                } else {
                    !0
                }
            }
        }

        // Reject invalid capability types (Empty, Frame, IOPort, CNode, Untyped)
        _ => !0,
    }
}

fn sys_reply(_msg: u64) -> u64 {
    0
}

fn sys_cap_info(endpoint_cptr: CPtr) -> u64 {
    let task_cnode = {
        let task = match unsafe { crate::task::current_task_mut() } {
            Some(t) => t,
            None => return !0,
        };
        task.root_cnode
    };

    let cap = match lookup_cap(task_cnode, endpoint_cptr) {
        Ok(c) => c,
        Err(_) => return !0,
    };

    match cap.cap_type {
        CapType::Frame | CapType::MmioFrame => cap.payload_a,
        CapType::IOPort => cap.payload_a,
        CapType::Empty => !0,
        _ => {
            (cap.cap_type as u64)
                | ((cap.rights.bits() as u64) << 8)
                | ((cap.generation as u64) << 16)
        }
    }
}

pub fn wake_irq(irq: u8) {
    unsafe {
        for i in 0..crate::task::MAX_TASKS {
            if let Some(t) = crate::task::get_task_mut(i as u64) {
                if let TaskState::BlockedOnRecv { endpoint } = t.state {
                    if let Ok(slot) = lookup_cap(t.root_cnode, endpoint) {
                        if slot.cap_type == CapType::Irq && (slot.payload_a as u8) == irq {
                            t.state = TaskState::Runnable;
                            t.ipc_message = 0; // Empty message indicates IRQ
                            return; // Assuming only one task listens to an IRQ
                        }
                    }
                }
            }
        }
    }
}

fn sys_in_port(endpoint_cptr: CPtr, offset: u16, width: u8) -> u64 {
    let task_cnode = {
        let task = match unsafe { crate::task::current_task_mut() } {
            Some(t) => t,
            None => return !0,
        };
        task.root_cnode
    };

    let cap = match lookup_cap(task_cnode, endpoint_cptr) {
        Ok(c) => c,
        Err(_) => return !0,
    };

    let port =
        match crate::capability::validate_io_port_access(&cap, CapRights::READ, offset, width) {
            Ok(p) => p,
            Err(_) => return !0,
        };

    unsafe {
        match width {
            1 => {
                let mut v: u8;
                core::arch::asm!("in al, dx", out("al") v, in("dx") port, options(nomem, nostack, preserves_flags));
                v as u64
            }
            2 => {
                let mut v: u16;
                core::arch::asm!("in ax, dx", out("ax") v, in("dx") port, options(nomem, nostack, preserves_flags));
                v as u64
            }
            4 => {
                let mut v: u32;
                core::arch::asm!("in eax, dx", out("eax") v, in("dx") port, options(nomem, nostack, preserves_flags));
                v as u64
            }
            _ => !0,
        }
    }
}

fn sys_out_port(endpoint_cptr: CPtr, offset: u16, val: u32, width: u8) -> u64 {
    let task_cnode = {
        let task = match unsafe { crate::task::current_task_mut() } {
            Some(t) => t,
            None => return !0,
        };
        task.root_cnode
    };

    let cap = match lookup_cap(task_cnode, endpoint_cptr) {
        Ok(c) => c,
        Err(_) => return !0,
    };

    let port =
        match crate::capability::validate_io_port_access(&cap, CapRights::WRITE, offset, width) {
            Ok(p) => p,
            Err(_) => return !0,
        };

    unsafe {
        match width {
            1 => {
                core::arch::asm!("out dx, al", in("dx") port, in("al") (val as u8), options(nomem, nostack, preserves_flags));
                0
            }
            2 => {
                core::arch::asm!("out dx, ax", in("dx") port, in("ax") (val as u16), options(nomem, nostack, preserves_flags));
                0
            }
            4 => {
                core::arch::asm!("out dx, eax", in("dx") port, in("eax") val, options(nomem, nostack, preserves_flags));
                0
            }
            _ => !0,
        }
    }
}

fn sys_cap_mint(src_cptr: CPtr, dest_slot: usize, rights: u8, badge: u64) -> u64 {
    let current_task_id = match crate::task::current_task_id() {
        Some(id) => id as u8,
        None => return !0,
    };
    let cap_rights = match CapRights::from_bits(rights) {
        Some(r) => r,
        None => return !0,
    };
    match unsafe {
        crate::capability::mint_cap(current_task_id, src_cptr, dest_slot, cap_rights, badge)
    } {
        Ok(_) => 0,
        Err(_) => !0,
    }
}

fn sys_cap_copy(src_cptr: CPtr, dest_slot: usize) -> u64 {
    let current_task_id = match crate::task::current_task_id() {
        Some(id) => id as u8,
        None => return !0,
    };
    match unsafe { crate::capability::copy_cap(current_task_id, src_cptr, dest_slot) } {
        Ok(_) => 0,
        Err(_) => !0,
    }
}

fn sys_cap_revoke(cptr: CPtr) -> u64 {
    let current_task_id = match crate::task::current_task_id() {
        Some(id) => id as u8,
        None => return !0,
    };
    match unsafe { crate::capability::revoke_cap(current_task_id, cptr) } {
        Ok(()) => 0,
        Err(_) => !0,
    }
}

fn sys_cap_delete(cptr: CPtr) -> u64 {
    let current_task_id = match crate::task::current_task_id() {
        Some(id) => id as u8,
        None => return !0,
    };
    match unsafe { crate::capability::delete_cap(current_task_id, cptr) } {
        Ok(()) => 0,
        Err(_) => !0,
    }
}
