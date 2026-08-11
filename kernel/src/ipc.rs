use crate::capability::{CPtr, Capability};
use crate::task::TaskState;

pub const SYS_SEND: u64 = 1;
pub const SYS_RECV: u64 = 2;
pub const SYS_REPLY: u64 = 3;
pub const SYS_YIELD: u64 = 4;
pub const SYS_CAP_INFO: u64 = 5;
pub const SYS_IN_PORT: u64 = 6;
pub const SYS_OUT_PORT: u64 = 7;
pub const SYS_LOG: u64 = 8;

/// The main entry point for syscalls dispatched from assembly.
pub fn handle_syscall(sys_num: u64, arg1: u64, arg2: u64, arg3: u64, arg4: u64, _arg5: u64) -> u64 {
    match sys_num {
        SYS_SEND => sys_send(CPtr(arg1 as usize), arg2),
        SYS_RECV => sys_recv(CPtr(arg1 as usize), arg2),
        SYS_REPLY => sys_reply(arg1),
        SYS_YIELD => {
            unsafe { crate::task::schedule() };
            0
        }
        SYS_CAP_INFO => sys_cap_info(CPtr(arg1 as usize)),
        SYS_IN_PORT => sys_in_port(CPtr(arg1 as usize), arg2 as u16, arg3 as u8),
        SYS_OUT_PORT => sys_out_port(CPtr(arg1 as usize), arg2 as u16, arg3 as u32, arg4 as u8),
        SYS_LOG => {
            let mut serial = crate::SerialPort(0x3F8);
            let val = unsafe { *(arg1 as *const u32) };
            serial.write_str("[KERNEL] USERSPACE LOG DATA: ");
            serial.write_hex(val as u64);
            serial.write_str("\n");
            0
        }
        _ => !0, // ENOSYS
    }
}

/// Send a message to an endpoint.
fn sys_send(endpoint_cptr: CPtr, msg: u64) -> u64 {
    // 1. Validate the capability
    let sender_id = unsafe { crate::task::current_task_mut().unwrap().id };

    // In a real system, we'd traverse the CNode tree. For v0, we just assume a flat Root CNode.
    // Wait, we don't have access to the full CNode lookup yet. Let's build a simple lookup.
    // Actually, we can just look at `root_cnode` pointer in the task.
    // We'll leave that implementation detail for a slightly lower level.
    // For now, let's fake the capability lookup for the IPC logic proof.
    // Wait, the user specifically wants capability enforcement proved!

    // We must read the capability from the current task's CNode.
    let sender = unsafe { crate::task::current_task_mut().unwrap() };

    let cap = unsafe {
        let cnode = &*(sender.root_cnode.0 as *const crate::capability::CNode);
        if endpoint_cptr.0 >= crate::capability::CNODE_SLOTS {
            return !0; // Capability fault (out of bounds)
        }
        cnode.slots[endpoint_cptr.0]
    };

    let target_task_id = match cap {
        Capability::Endpoint { target_task_id } => target_task_id,
        _ => {
            let mut serial = crate::SerialPort(0x3F8);
            serial.write_str("[KERNEL SEC-FAULT] Task ");
            serial.write_hex(sender_id);
            serial.write_str(" attempted to use invalid capability slot! Terminating.\n");

            sender.state = crate::task::TaskState::Terminated;
            unsafe { crate::task::schedule() };
            return !0;
        }
    };

    // 2. Perform the IPC
    let target = unsafe { crate::task::get_task_mut(target_task_id).unwrap() };

    // Check if the target is blocked waiting to receive.
    // In L4-style synchronous IPC, a receiver blocks until a sender delivers.
    let target_is_waiting = matches!(target.state, TaskState::BlockedOnRecv { .. });

    if target_is_waiting {
        // Target is waiting for us! Deliver the message.
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
        // Target is not waiting for us. We block.
        let mut serial = crate::SerialPort(0x3F8);
        serial.write_str("[KERNEL IPC] Task ");
        serial.write_hex(sender_id);
        serial.write_str(" blocking on send to Task ");
        serial.write_hex(target_task_id);
        serial.write_str("\n");

        let sender = unsafe { crate::task::current_task_mut().unwrap() };
        sender.state = TaskState::BlockedOnSend {
            endpoint: endpoint_cptr,
        };
        // Save the message in ipc_message so it can be retrieved when they wake us
        sender.ipc_message = msg;
        unsafe { crate::task::schedule() };
        0
    }
}

fn sys_recv(endpoint_cptr: CPtr, _msg_ptr: u64) -> u64 {
    let receiver = unsafe { crate::task::current_task_mut().unwrap() };
    let receiver_id = receiver.id;

    // 1. Validate capability
    let cap = unsafe {
        let cnode = &*(receiver.root_cnode.0 as *const crate::capability::CNode);
        if endpoint_cptr.0 >= crate::capability::CNODE_SLOTS {
            return !0; // Capability fault
        }
        cnode.slots[endpoint_cptr.0]
    };

    // Check if this is an InterruptEndpoint
    if let Capability::InterruptEndpoint { irq: _ } = cap {
        receiver.state = TaskState::BlockedOnRecv {
            endpoint: endpoint_cptr,
        };
        unsafe { crate::task::schedule() };
        // Hardware IRQ will wake us up
        let receiver = unsafe { crate::task::current_task_mut().unwrap() };
        return receiver.ipc_message;
    }

    // Not an InterruptEndpoint, check for tasks blocked on send
    let mut found_sender = None;
    unsafe {
        for i in 0..crate::task::MAX_TASKS {
            if let Some(t) = crate::task::get_task_mut(i as u64) {
                if let TaskState::BlockedOnSend { endpoint } = t.state {
                    // Look up the sender's capability to see if it targets us
                    let sender_cnode = &*(t.root_cnode.0 as *const crate::capability::CNode);
                    if endpoint.0 < crate::capability::CNODE_SLOTS {
                        if let Capability::Endpoint { target_task_id } =
                            sender_cnode.slots[endpoint.0]
                        {
                            if target_task_id == receiver_id {
                                found_sender = Some(i as u64);
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    if let Some(sender_id) = found_sender {
        let sender = unsafe { crate::task::get_task_mut(sender_id).unwrap() };
        let msg = sender.ipc_message;
        sender.state = TaskState::Runnable;

        let mut serial = crate::SerialPort(0x3F8);
        serial.write_str("[KERNEL IPC] Task ");
        serial.write_hex(receiver_id);
        serial.write_str(" received message from blocked Task ");
        serial.write_hex(sender_id);
        serial.write_str("\n");

        return msg;
    } else {
        let receiver = unsafe { crate::task::current_task_mut().unwrap() };
        receiver.state = TaskState::BlockedOnRecv {
            endpoint: endpoint_cptr,
        };
        unsafe { crate::task::schedule() };

        // When we wake up, the sender will have written the message into our ipc_message.
        let receiver = unsafe { crate::task::current_task_mut().unwrap() };
        receiver.ipc_message
    }
}

fn sys_reply(_msg: u64) -> u64 {
    0
}

fn sys_cap_info(endpoint_cptr: CPtr) -> u64 {
    let task = unsafe { crate::task::current_task_mut().unwrap() };
    let task_id = task.id;

    let cap = unsafe {
        let cnode = &*(task.root_cnode.0 as *const crate::capability::CNode);
        if endpoint_cptr.0 >= crate::capability::CNODE_SLOTS {
            return !0; // Capability fault (out of bounds)
        }
        cnode.slots[endpoint_cptr.0]
    };

    match cap {
        Capability::Frame { paddr } => paddr,
        Capability::IOPort { base, size } => (base as u64) | ((size as u64) << 16),
        _ => {
            let mut serial = crate::SerialPort(0x3F8);
            serial.write_str("[KERNEL SEC-FAULT] Task ");
            serial.write_hex(task_id);
            serial.write_str(
                " attempted to query invalid capability slot via SYS_CAP_INFO! Terminating.\n",
            );

            task.state = crate::task::TaskState::Terminated;
            unsafe { crate::task::schedule() };
            !0
        }
    }
}

pub fn wake_irq(irq: u8) {
    unsafe {
        for i in 0..crate::task::MAX_TASKS {
            if let Some(t) = crate::task::get_task_mut(i as u64) {
                if let TaskState::BlockedOnRecv { endpoint } = t.state {
                    let cnode = &*(t.root_cnode.0 as *const crate::capability::CNode);
                    if endpoint.0 < crate::capability::CNODE_SLOTS {
                        if let Capability::InterruptEndpoint { irq: cap_irq } =
                            cnode.slots[endpoint.0]
                        {
                            if cap_irq == irq {
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
}

fn sys_in_port(endpoint_cptr: CPtr, offset: u16, width: u8) -> u64 {
    let task = unsafe { crate::task::current_task_mut().unwrap() };

    let cap = unsafe {
        let cnode = &*(task.root_cnode.0 as *const crate::capability::CNode);
        if endpoint_cptr.0 >= crate::capability::CNODE_SLOTS {
            return !0;
        }
        cnode.slots[endpoint_cptr.0]
    };

    if let Capability::IOPort { base, size } = cap {
        if offset >= size {
            return !0;
        }
        let port = base + offset;
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
    } else {
        !0
    }
}

fn sys_out_port(endpoint_cptr: CPtr, offset: u16, val: u32, width: u8) -> u64 {
    let task = unsafe { crate::task::current_task_mut().unwrap() };

    let cap = unsafe {
        let cnode = &*(task.root_cnode.0 as *const crate::capability::CNode);
        if endpoint_cptr.0 >= crate::capability::CNODE_SLOTS {
            return !0;
        }
        cnode.slots[endpoint_cptr.0]
    };

    if let Capability::IOPort { base, size } = cap {
        if offset >= size {
            return !0;
        }
        let port = base + offset;
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
    } else {
        !0
    }
}
