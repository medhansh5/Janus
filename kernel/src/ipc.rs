use crate::capability::{CPtr, Capability};
use crate::task::{current_task_mut, get_task_mut, TaskState, switch_context};

pub const SYS_SEND: u64 = 1;
pub const SYS_RECV: u64 = 2;
pub const SYS_REPLY: u64 = 3;
pub const SYS_YIELD: u64 = 4;

/// The main entry point for syscalls dispatched from assembly.
pub fn handle_syscall(sys_num: u64, arg1: u64, arg2: u64, _arg3: u64, _arg4: u64, _arg5: u64) -> u64 {
    match sys_num {
        SYS_SEND => sys_send(CPtr(arg1 as usize), arg2),
        SYS_RECV => sys_recv(CPtr(arg1 as usize), arg2),
        SYS_REPLY => sys_reply(arg1),
        SYS_YIELD => { unsafe { crate::task::schedule() }; 0 },
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
        sender.state = TaskState::BlockedOnSend { endpoint: endpoint_cptr };
        // Save the message in ipc_message so it can be retrieved when they wake us
        sender.ipc_message = msg;
        unsafe { crate::task::schedule() };
        0
    }
}

fn sys_recv(endpoint_cptr: CPtr, _msg_ptr: u64) -> u64 {
    let receiver = unsafe { crate::task::current_task_mut().unwrap() };
    let receiver_id = receiver.id;
    
    // Scan tasks to see if anyone is BlockedOnSend to us.
    // A sender is targeting us if their endpoint capability's target_task_id == our id.
    let mut found_sender = None;
    unsafe {
        for i in 0..crate::task::MAX_TASKS {
            if let Some(t) = crate::task::get_task_mut(i as u64) {
                if let TaskState::BlockedOnSend { endpoint } = t.state {
                    // Look up the sender's capability to see if it targets us
                    let sender_cnode = &*(t.root_cnode.0 as *const crate::capability::CNode);
                    if endpoint.0 < crate::capability::CNODE_SLOTS {
                        if let Capability::Endpoint { target_task_id } = sender_cnode.slots[endpoint.0] {
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
        receiver.state = TaskState::BlockedOnRecv { endpoint: endpoint_cptr };
        unsafe { crate::task::schedule() };
        
        // When we wake up, the sender will have written the message into our ipc_message.
        let receiver = unsafe { crate::task::current_task_mut().unwrap() };
        receiver.ipc_message
    }
}

fn sys_reply(_msg: u64) -> u64 {
    0
}
