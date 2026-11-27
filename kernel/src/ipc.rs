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
pub const SYS_NOTIFY_CREATE: u64 = 9;
pub const SYS_NOTIFY_SIGNAL: u64 = 10;
pub const SYS_CAP_MINT: u64 = 13;
pub const SYS_CAP_REVOKE: u64 = 14;
pub const SYS_NOTIFY_WAIT: u64 = 15;
pub const SYS_NOTIFY_POLL: u64 = 16;
pub const SYS_IRQ_BIND: u64 = 17;
pub const SYS_IRQ_ACK: u64 = 18;
pub const SYS_SLEEP: u64 = 19;
pub const SYS_CAP_COPY: u64 = 20;
pub const SYS_CAP_DELETE: u64 = 21;
pub const SYS_EXIT: u64 = 22;
pub const SYS_GET_TIME: u64 = 23;
pub const SYS_READ_CONSOLE: u64 = 24;
pub const SYS_WRITE_CONSOLE: u64 = 25;
pub const SYS_FS_CREATE: u64 = 26;
pub const SYS_FS_READ: u64 = 27;
pub const SYS_FS_WRITE: u64 = 28;
pub const SYS_FS_DELETE: u64 = 29;
pub const SYS_FS_LIST: u64 = 30;
pub const SYS_TASK_STATUS: u64 = 32;

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
        SYS_NOTIFY_CREATE => crate::notification::sys_notify_create(arg1 as usize),
        SYS_NOTIFY_SIGNAL => crate::notification::sys_notify_signal(CPtr(arg1), arg2),
        SYS_CAP_MINT => sys_cap_mint(CPtr(arg1), arg2 as usize, arg3 as u8, arg4),
        SYS_CAP_REVOKE => sys_cap_revoke(CPtr(arg1)),
        SYS_NOTIFY_WAIT => crate::notification::sys_notify_wait(CPtr(arg1)),
        SYS_NOTIFY_POLL => crate::notification::sys_notify_poll(CPtr(arg1)),
        SYS_IRQ_BIND => crate::notification::sys_irq_bind(CPtr(arg1), CPtr(arg2), arg3),
        SYS_IRQ_ACK => crate::notification::sys_irq_ack(CPtr(arg1)),
        SYS_SLEEP => crate::task::sys_sleep(arg1),
        SYS_CAP_COPY => sys_cap_copy(CPtr(arg1), arg2 as usize),
        SYS_CAP_DELETE => sys_cap_delete(CPtr(arg1)),
        SYS_EXIT => crate::task::terminate_current_task(arg1),
        SYS_GET_TIME => crate::idt::get_tick_count(),
        SYS_READ_CONSOLE => crate::console::sys_read_console(CPtr(arg1), arg2, arg3),
        SYS_WRITE_CONSOLE => crate::console::sys_write_console(CPtr(arg1), arg2, arg3),
        SYS_FS_CREATE => crate::fs::sys_fs_create(CPtr(arg1), arg2, arg3),
        SYS_FS_READ => crate::fs::sys_fs_read(CPtr(arg1), arg2, arg3, arg4, _arg5),
        SYS_FS_WRITE => crate::fs::sys_fs_write(CPtr(arg1), arg2, arg3, arg4, _arg5),
        SYS_FS_DELETE => crate::fs::sys_fs_delete(CPtr(arg1), arg2, arg3),
        SYS_FS_LIST => crate::fs::sys_fs_list(CPtr(arg1), arg2, arg3),
        SYS_TASK_STATUS => sys_task_status(arg1, arg2),
        _ => !0, // ENOSYS
    }
}

/// Copies `buf.len()` bytes from user virtual memory into kernel buffer `buf`.
pub fn copy_user_slice_from(user_vaddr: u64, buf: &mut [u8]) -> Result<(), u64> {
    #[cfg(not(test))]
    {
        crate::paging::copy_from_user(user_vaddr, buf).map_err(|_| !0)
    }
    #[cfg(test)]
    {
        if user_vaddr == 0 || user_vaddr >= 0x0000_8000_0000_0000 {
            return Err(!0);
        }
        unsafe {
            core::ptr::copy(user_vaddr as *const u8, buf.as_mut_ptr(), buf.len());
        }
        Ok(())
    }
}

/// Copies `buf.len()` bytes from kernel buffer `buf` into user virtual memory.
pub fn copy_user_slice_to(user_vaddr: u64, buf: &[u8]) -> Result<(), u64> {
    #[cfg(not(test))]
    {
        crate::paging::copy_to_user(user_vaddr, buf).map_err(|_| !0)
    }
    #[cfg(test)]
    {
        if user_vaddr == 0 || user_vaddr >= 0x0000_8000_0000_0000 {
            return Err(!0);
        }
        unsafe {
            core::ptr::copy(buf.as_ptr(), user_vaddr as *mut u8, buf.len());
        }
        Ok(())
    }
}

/// Syscall 32: Diagnostic system and task status reporter.
pub fn sys_task_status(buf_ptr: u64, max_len: u64) -> u64 {
    let mut temp = [0u8; 1024];
    let mut offset = 0;

    let title = b"=== Janus OS System Status ===\n";
    if temp.len() > title.len() {
        temp[..title.len()].copy_from_slice(title);
        offset += title.len();
    }

    let uptime = crate::idt::get_tick_count();
    let uptime_str = b"Uptime: ";
    if offset + uptime_str.len() < temp.len() {
        temp[offset..offset + uptime_str.len()].copy_from_slice(uptime_str);
        offset += uptime_str.len();
    }
    offset += format_u64(uptime, &mut temp[offset..]);
    let ms_str = b" ms\n\nTasks:\nPID  STATE\n-----------------\n";
    if offset + ms_str.len() < temp.len() {
        temp[offset..offset + ms_str.len()].copy_from_slice(ms_str);
        offset += ms_str.len();
    }

    for i in 0..crate::task::MAX_TASKS {
        if let Some((pid, state)) = unsafe { crate::task::get_task_info(i) } {
            offset += format_u64(pid, &mut temp[offset..]);
            if offset + 5 < temp.len() {
                temp[offset] = b' ';
                temp[offset + 1] = b' ';
                temp[offset + 2] = b' ';
                offset += 3;
            }
            let state_str: &[u8] = match state {
                TaskState::Runnable => b"Runnable\n",
                TaskState::BlockedOnRecv { .. } => b"Blocked(Recv)\n",
                TaskState::BlockedOnSend { .. } => b"Blocked(Send)\n",
                TaskState::BlockedOnNotification { .. } => b"Blocked(Notif)\n",
                TaskState::BlockedOnSleep { .. } => b"Blocked(Sleep)\n",
                TaskState::Terminated => b"Terminated\n",
            };
            let slen = core::cmp::min(state_str.len(), temp.len() - offset);
            temp[offset..offset + slen].copy_from_slice(&state_str[..slen]);
            offset += slen;
        }
    }

    let to_copy = core::cmp::min(offset, max_len as usize);
    if to_copy > 0 && copy_user_slice_to(buf_ptr, &temp[..to_copy]).is_err() {
        return !0;
    }
    to_copy as u64
}

fn format_u64(mut val: u64, buf: &mut [u8]) -> usize {
    if val == 0 {
        if !buf.is_empty() {
            buf[0] = b'0';
            return 1;
        }
        return 0;
    }
    let mut digits = [0u8; 20];
    let mut count = 0;
    while val > 0 && count < 20 {
        digits[count] = b'0' + (val % 10) as u8;
        val /= 10;
        count += 1;
    }
    let write_len = core::cmp::min(count, buf.len());
    for i in 0..write_len {
        buf[i] = digits[count - 1 - i];
    }
    write_len
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::{
        delete_cap, mint_cap, CNode, CNodeSlot, CdtNode, CDT, MAX_TASKS, TEST_CNODES, TEST_LOCK,
        TOTAL_CDT_SLOTS,
    };
    use crate::task::{register_task, set_current_task, Task};
    use x86_64::PhysAddr;

    unsafe fn setup_syscall_test_env() {
        TEST_CNODES = Some([CNode::empty(); MAX_TASKS]);
        CDT = [CdtNode::empty(); TOTAL_CDT_SLOTS];

        let task0 = Task::new(0, PhysAddr::new(0), CPtr(0), 0);
        let task1 = Task::new(1, PhysAddr::new(0), CPtr(0), 0);
        register_task(task0);
        register_task(task1);
        set_current_task(0);

        let cnode0 = crate::capability::get_task_cnode_mut(0).unwrap();
        cnode0.slots[8] = CNodeSlot {
            cap_type: CapType::Console,
            rights: CapRights::READ | CapRights::WRITE | CapRights::GRANT,
            _reserved1: 0,
            generation: 1,
            payload_a: 0,
            payload_b: 0,
            cdt_index: crate::capability::cdt_index(0, 8) as u32,
            _reserved2: [0; 4],
        };
        cnode0.slots[9] = CNodeSlot {
            cap_type: CapType::Storage,
            rights: CapRights::READ | CapRights::WRITE | CapRights::GRANT,
            _reserved1: 0,
            generation: 1,
            payload_a: 0,
            payload_b: 0,
            cdt_index: crate::capability::cdt_index(0, 9) as u32,
            _reserved2: [0; 4],
        };

        // Drain any leftover console characters
        while crate::console::dequeue_char().is_some() {}
        crate::fs::format();
    }

    #[test]
    fn test_syscalls_24_30_capability_negative_and_positive() {
        let _guard = TEST_LOCK.lock();
        unsafe {
            setup_syscall_test_env();

            let console_rw = CPtr::new(1, 8);
            let storage_rw = CPtr::new(1, 9);

            // Mint attenuated capabilities in Task 0
            let console_ro = mint_cap(0, console_rw, 10, CapRights::READ, 0).unwrap();
            let console_wo = mint_cap(0, console_rw, 11, CapRights::WRITE, 0).unwrap();
            let storage_ro = mint_cap(0, storage_rw, 12, CapRights::READ, 0).unwrap();
            let storage_wo = mint_cap(0, storage_rw, 13, CapRights::WRITE, 0).unwrap();

            // Mint & delete a storage cap to produce a stale generation CPtr
            let temp_cap =
                mint_cap(0, storage_rw, 14, CapRights::READ | CapRights::WRITE, 0).unwrap();
            delete_cap(0, temp_cap).unwrap();
            let stale_cptr = temp_cap;

            // Non-canonical CPtr (reserved bits set) and empty slot CPtr
            let non_canonical = CPtr(0xFFFF_0000_0001_0008);
            let empty_slot = CPtr::new(1, 50);

            let path = b"/neg.txt";
            let data = b"secret";
            let mut out_buf = [0xAAu8; 256];

            // --- 1. SYS_READ_CONSOLE (24) ---
            crate::console::enqueue_char(b'Z');
            for bad in [
                non_canonical,
                empty_slot,
                stale_cptr,
                storage_rw,
                console_wo,
            ] {
                assert_eq!(
                    handle_syscall(
                        SYS_READ_CONSOLE,
                        bad.raw(),
                        out_buf.as_mut_ptr() as u64,
                        1,
                        0,
                        0
                    ),
                    !0
                );
            }
            // Null user pointer with unauthorized cap must fail at cap check without crashing
            assert_eq!(
                handle_syscall(SYS_READ_CONSOLE, console_wo.raw(), 0, 1, 0, 0),
                !0
            );
            // Output buffer untouched and queued char still intact
            assert_eq!(out_buf[0], 0xAA);
            // Positive control: valid Console READ cap reads 'Z'
            assert_eq!(
                handle_syscall(
                    SYS_READ_CONSOLE,
                    console_ro.raw(),
                    out_buf.as_mut_ptr() as u64,
                    1,
                    0,
                    0
                ),
                1
            );
            assert_eq!(out_buf[0], b'Z');

            // --- 2. SYS_WRITE_CONSOLE (25) ---
            crate::console::TEST_CONSOLE_OUT_COUNT = 0;
            for bad in [
                non_canonical,
                empty_slot,
                stale_cptr,
                storage_rw,
                console_ro,
            ] {
                assert_eq!(
                    handle_syscall(
                        SYS_WRITE_CONSOLE,
                        bad.raw(),
                        data.as_ptr() as u64,
                        data.len() as u64,
                        0,
                        0
                    ),
                    !0
                );
            }
            assert_eq!(
                handle_syscall(SYS_WRITE_CONSOLE, console_ro.raw(), 0, 4, 0, 0),
                !0
            );
            assert_eq!(crate::console::TEST_CONSOLE_OUT_COUNT, 0);
            // Positive control
            assert_eq!(
                handle_syscall(
                    SYS_WRITE_CONSOLE,
                    console_wo.raw(),
                    data.as_ptr() as u64,
                    data.len() as u64,
                    0,
                    0
                ),
                data.len() as u64
            );
            assert_eq!(crate::console::TEST_CONSOLE_OUT_COUNT, data.len());

            // --- 3. SYS_FS_CREATE (26) ---
            for bad in [
                non_canonical,
                empty_slot,
                stale_cptr,
                console_rw,
                storage_ro,
            ] {
                assert_eq!(
                    handle_syscall(
                        SYS_FS_CREATE,
                        bad.raw(),
                        path.as_ptr() as u64,
                        path.len() as u64,
                        0,
                        0
                    ),
                    !0
                );
            }
            assert_eq!(
                handle_syscall(SYS_FS_CREATE, storage_ro.raw(), 0, 4, 0, 0),
                !0
            );
            assert!(crate::fs::read_file("/neg.txt", &mut out_buf).is_err());
            // Positive control
            assert_eq!(
                handle_syscall(
                    SYS_FS_CREATE,
                    storage_wo.raw(),
                    path.as_ptr() as u64,
                    path.len() as u64,
                    0,
                    0
                ),
                0
            );

            // --- 4. SYS_FS_WRITE (28) ---
            for bad in [
                non_canonical,
                empty_slot,
                stale_cptr,
                console_rw,
                storage_ro,
            ] {
                assert_eq!(
                    handle_syscall(
                        SYS_FS_WRITE,
                        bad.raw(),
                        path.as_ptr() as u64,
                        path.len() as u64,
                        data.as_ptr() as u64,
                        data.len() as u64,
                    ),
                    !0
                );
            }
            assert_eq!(
                handle_syscall(SYS_FS_WRITE, storage_ro.raw(), 0, 4, 0, 4),
                !0
            );
            // File must still be empty (0 bytes)
            assert_eq!(crate::fs::read_file("/neg.txt", &mut out_buf), Ok(0));
            // Positive control
            assert_eq!(
                handle_syscall(
                    SYS_FS_WRITE,
                    storage_wo.raw(),
                    path.as_ptr() as u64,
                    path.len() as u64,
                    data.as_ptr() as u64,
                    data.len() as u64,
                ),
                data.len() as u64
            );

            // --- 5. SYS_FS_READ (27) ---
            out_buf.fill(0xAA);
            for bad in [
                non_canonical,
                empty_slot,
                stale_cptr,
                console_rw,
                storage_wo,
            ] {
                assert_eq!(
                    handle_syscall(
                        SYS_FS_READ,
                        bad.raw(),
                        path.as_ptr() as u64,
                        path.len() as u64,
                        out_buf.as_mut_ptr() as u64,
                        out_buf.len() as u64,
                    ),
                    !0
                );
            }
            assert_eq!(
                handle_syscall(SYS_FS_READ, storage_wo.raw(), 0, 4, 0, 4),
                !0
            );
            assert_eq!(out_buf[0], 0xAA);
            // Positive control
            assert_eq!(
                handle_syscall(
                    SYS_FS_READ,
                    storage_ro.raw(),
                    path.as_ptr() as u64,
                    path.len() as u64,
                    out_buf.as_mut_ptr() as u64,
                    out_buf.len() as u64,
                ),
                data.len() as u64
            );
            assert_eq!(&out_buf[..data.len()], data);

            // --- 6. SYS_FS_LIST (30) ---
            out_buf.fill(0xAA);
            for bad in [
                non_canonical,
                empty_slot,
                stale_cptr,
                console_rw,
                storage_wo,
            ] {
                assert_eq!(
                    handle_syscall(
                        SYS_FS_LIST,
                        bad.raw(),
                        out_buf.as_mut_ptr() as u64,
                        out_buf.len() as u64,
                        0,
                        0
                    ),
                    !0
                );
            }
            assert_eq!(
                handle_syscall(SYS_FS_LIST, storage_wo.raw(), 0, 16, 0, 0),
                !0
            );
            assert_eq!(out_buf[0], 0xAA);
            // Positive control
            let listed = handle_syscall(
                SYS_FS_LIST,
                storage_ro.raw(),
                out_buf.as_mut_ptr() as u64,
                out_buf.len() as u64,
                0,
                0,
            );
            assert_ne!(listed, !0);
            assert!(listed > 0);

            // --- 7. SYS_FS_DELETE (29) ---
            for bad in [
                non_canonical,
                empty_slot,
                stale_cptr,
                console_rw,
                storage_ro,
            ] {
                assert_eq!(
                    handle_syscall(
                        SYS_FS_DELETE,
                        bad.raw(),
                        path.as_ptr() as u64,
                        path.len() as u64,
                        0,
                        0
                    ),
                    !0
                );
            }
            assert_eq!(
                handle_syscall(SYS_FS_DELETE, storage_ro.raw(), 0, 4, 0, 0),
                !0
            );
            // File must still exist
            assert_eq!(
                crate::fs::read_file("/neg.txt", &mut out_buf),
                Ok(data.len())
            );

            // --- 8. Unauthorized Task (Task 1 has no Console or Storage capabilities) ---
            set_current_task(1);
            crate::console::TEST_CONSOLE_OUT_COUNT = 0;
            crate::console::enqueue_char(b'Q');
            assert_eq!(
                handle_syscall(
                    SYS_READ_CONSOLE,
                    console_rw.raw(),
                    out_buf.as_mut_ptr() as u64,
                    1,
                    0,
                    0
                ),
                !0
            );
            assert_eq!(
                handle_syscall(
                    SYS_WRITE_CONSOLE,
                    console_rw.raw(),
                    data.as_ptr() as u64,
                    data.len() as u64,
                    0,
                    0
                ),
                !0
            );
            assert_eq!(
                handle_syscall(
                    SYS_FS_CREATE,
                    storage_rw.raw(),
                    path.as_ptr() as u64,
                    path.len() as u64,
                    0,
                    0
                ),
                !0
            );
            assert_eq!(
                handle_syscall(
                    SYS_FS_READ,
                    storage_rw.raw(),
                    path.as_ptr() as u64,
                    path.len() as u64,
                    out_buf.as_mut_ptr() as u64,
                    16
                ),
                !0
            );
            assert_eq!(
                handle_syscall(
                    SYS_FS_WRITE,
                    storage_rw.raw(),
                    path.as_ptr() as u64,
                    path.len() as u64,
                    data.as_ptr() as u64,
                    data.len() as u64
                ),
                !0
            );
            assert_eq!(
                handle_syscall(
                    SYS_FS_DELETE,
                    storage_rw.raw(),
                    path.as_ptr() as u64,
                    path.len() as u64,
                    0,
                    0
                ),
                !0
            );
            assert_eq!(
                handle_syscall(
                    SYS_FS_LIST,
                    storage_rw.raw(),
                    out_buf.as_mut_ptr() as u64,
                    64,
                    0,
                    0
                ),
                !0
            );
            assert_eq!(crate::console::TEST_CONSOLE_OUT_COUNT, 0);
            assert_eq!(crate::console::dequeue_char(), Some(b'Q'));

            // Positive control: Task 0 deletes /neg.txt
            set_current_task(0);
            assert_eq!(
                handle_syscall(
                    SYS_FS_DELETE,
                    storage_wo.raw(),
                    path.as_ptr() as u64,
                    path.len() as u64,
                    0,
                    0
                ),
                0
            );
            assert!(crate::fs::read_file("/neg.txt", &mut out_buf).is_err());
        }
    }
}
