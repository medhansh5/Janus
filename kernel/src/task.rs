use crate::capability::CPtr;
use x86_64::PhysAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Runnable,
    /// Blocked waiting to receive a message on the specified endpoint capability
    BlockedOnRecv {
        endpoint: CPtr,
    },
    /// Blocked waiting to send a message on the specified endpoint capability
    BlockedOnSend {
        endpoint: CPtr,
    },
    /// Blocked waiting for an asynchronous event on the specified notification object
    BlockedOnNotification {
        notification_id: u16,
    },
    /// Blocked waiting for a timer deadline to expire
    BlockedOnSleep {
        wake_tick: u64,
    },
    /// Terminated by the kernel due to a fault or security violation
    Terminated,
}

#[repr(C)]
#[derive(Debug, Clone, Default)]
pub struct TaskContext {
    // Callee-saved registers for context switching
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub rbx: u64,
    pub rbp: u64,
    pub rsi: u64,
    pub rdi: u64,
    // The stack pointer (either kernel stack or user stack, depending on state)
    pub rsp: u64,
    // The instruction pointer (return address)
    pub rip: u64,
}

impl TaskContext {
    pub const fn empty() -> Self {
        TaskContext {
            r15: 0,
            r14: 0,
            r13: 0,
            r12: 0,
            rbx: 0,
            rbp: 0,
            rsi: 0,
            rdi: 0,
            rsp: 0,
            rip: 0,
        }
    }
}

pub struct Task {
    pub id: u64,
    pub page_table: PhysAddr,
    pub root_cnode: CPtr,
    pub context: TaskContext,
    pub state: TaskState,
    pub kernel_stack_top: u64, // Used to set RSP0 in TSS on switch
    pub ipc_message: u64,      // Dedicated slot for synchronous IPC messages
}

impl Task {
    pub fn new(id: u64, page_table: PhysAddr, root_cnode: CPtr, kernel_stack_top: u64) -> Self {
        Self {
            id,
            page_table,
            root_cnode,
            context: TaskContext::default(),
            state: TaskState::Runnable,
            kernel_stack_top,
            ipc_message: 0,
        }
    }
}

// Global task list (for v0, we can just use a simple static array)
pub const MAX_TASKS: usize = 16;
pub const IDLE_TASK_ID: u64 = (MAX_TASKS - 1) as u64;

static mut TASKS: [Option<Task>; MAX_TASKS] = [
    None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None,
];
static mut CURRENT_TASK_ID: Option<u64> = None;

pub fn current_task_id() -> Option<u64> {
    unsafe { CURRENT_TASK_ID }
}

/// Returns snapshot of task ID and state for diagnostic reporting.
///
/// # Safety
/// Caller must ensure single-threaded execution in kernel mode.
pub unsafe fn get_task_info(id: usize) -> Option<(u64, TaskState)> {
    if id < MAX_TASKS {
        TASKS[id].as_ref().map(|t| (t.id, t.state))
    } else {
        None
    }
}

/// Wakes any tasks that are BlockedOnSleep whose deadline has expired.
///
/// # Safety
/// Caller must ensure single-threaded execution with interrupts disabled.
pub unsafe fn wake_sleeping_tasks(current_tick: u64) {
    for task in TASKS.iter_mut().flatten() {
        if let TaskState::BlockedOnSleep { wake_tick } = task.state {
            if current_tick >= wake_tick {
                task.state = TaskState::Runnable;
            }
        }
    }
}

/// Terminates the current task, releases its CSpace, marks it Terminated, and yields.
pub fn terminate_current_task(_exit_code: u64) -> ! {
    unsafe {
        if let Some(task_id) = current_task_id() {
            crate::capability::destroy_task_cspace(task_id as u8);
            if let Some(task) = current_task_mut() {
                task.state = TaskState::Terminated;
            }
        }
        schedule();
        loop {
            core::arch::asm!("hlt");
        }
    }
}

/// Syscall: Blocks the calling task for `duration_ms` milliseconds.
pub fn sys_sleep(duration_ms: u64) -> u64 {
    if duration_ms == 0 {
        unsafe { schedule() };
        return 0;
    }
    let current_tick = crate::idt::get_tick_count();
    let wake_tick = current_tick.saturating_add(duration_ms);
    {
        let task = match unsafe { current_task_mut() } {
            Some(t) => t,
            None => return !0,
        };
        task.state = TaskState::BlockedOnSleep { wake_tick };
    }
    unsafe { schedule() };
    0
}

/// # Safety
/// Caller must ensure `task` has valid state and stack pointers.
pub unsafe fn register_task(task: Task) {
    let id = task.id as usize;
    if id < MAX_TASKS {
        TASKS[id] = Some(task);
    }
}

/// # Safety
/// - Caller must guarantee single-threaded access to `id`.
/// - Caller must ensure no other active reference (`&Task` or `&mut Task`) to task `id` exists
///   concurrently for the lifetime `'a`.
/// - The returned mutable reference must not be retained across operations that access the task table
///   (such as `schedule()` or subsequent calls to `get_task_mut`/`current_task_mut`).
pub unsafe fn get_task_mut<'a>(id: u64) -> Option<&'a mut Task> {
    if id < MAX_TASKS as u64 {
        TASKS[id as usize].as_mut()
    } else {
        None
    }
}

/// # Safety
/// - Caller must guarantee single-threaded access to current task state.
/// - Caller must ensure no other active reference (`&Task` or `&mut Task`) to current task exists
///   concurrently for the lifetime `'a`.
/// - The returned mutable reference must not be retained across operations that access the task table
///   (such as `schedule()`).
pub unsafe fn current_task_mut<'a>() -> Option<&'a mut Task> {
    if let Some(id) = CURRENT_TASK_ID {
        get_task_mut(id)
    } else {
        None
    }
}

/// # Safety
/// Must be called with valid task context and stack setups.
pub unsafe fn schedule() {
    let current_id = CURRENT_TASK_ID.unwrap();
    let mut next_id = current_id + 1;

    // Find the next runnable task (excluding the Idle Task)
    let mut found = false;
    for _ in 0..MAX_TASKS - 1 {
        if next_id >= IDLE_TASK_ID {
            next_id = 0;
        }
        if let Some(ref t) = TASKS[next_id as usize] {
            if t.state == TaskState::Runnable {
                found = true;
                break;
            }
        }
        next_id += 1;
    }

    if !found {
        // No normal tasks are runnable. Fall back to the Idle Task.
        next_id = IDLE_TASK_ID;
    }

    if next_id == current_id {
        return; // Still the same task, no context switch needed
    }

    // Perform context switch
    #[allow(unused_variables)]
    let prev = get_task_mut(current_id).unwrap();
    #[allow(unused_variables)]
    let next = get_task_mut(next_id).unwrap();

    CURRENT_TASK_ID = Some(next_id);

    #[cfg(not(test))]
    {
        // Update TSS RSP0 to the new task's kernel stack (for interrupts)
        crate::gdt::set_kernel_stack(next.kernel_stack_top);

        // Also update the scratch variable for `syscall`
        unsafe {
            crate::syscall::KERNEL_RSP_SCRATCH = next.kernel_stack_top;
        }

        // Switch CR3 to the new task's page table
        let _cr3_val = next.page_table.as_u64() | 0x3;
        core::arch::asm!(
            "mov cr3, {0}",
            in(reg) next.page_table.as_u64(),
            options(nostack)
        );

        switch_context(&mut prev.context, &next.context);
    }
}

/// # Safety
/// Must be called during boot initialization to set initial task ID.
pub unsafe fn set_current_task(id: u64) {
    CURRENT_TASK_ID = Some(id);
}

core::arch::global_asm!(
    ".global switch_context",
    "switch_context:",
    // rdi = *mut TaskContext (prev)
    // rsi = *const TaskContext (next)

    // Save current context into `prev`
    "mov [rdi + 0x00], r15",
    "mov [rdi + 0x08], r14",
    "mov [rdi + 0x10], r13",
    "mov [rdi + 0x18], r12",
    "mov [rdi + 0x20], rbx",
    "mov [rdi + 0x28], rbp",
    "mov [rdi + 0x30], rsi",
    "mov [rdi + 0x38], rdi",
    "mov [rdi + 0x40], rsp", // Save RSP (points to return address from `call switch_context`)
    // rip is saved implicitly: [rsp] holds the return address. We don't need to save it
    // separately because we'll use `ret` to return to it.

    // Restore context from `next`
    "mov r15, [rsi + 0x00]",
    "mov r14, [rsi + 0x08]",
    "mov r13, [rsi + 0x10]",
    "mov r12, [rsi + 0x18]",
    "mov rbx, [rsi + 0x20]",
    "mov rbp, [rsi + 0x28]",
    "mov rdi, [rsi + 0x38]",
    "mov rsp, [rsi + 0x40]", // Restore RSP (points to saved return address)
    "mov rsi, [rsi + 0x30]", // Restore next.rsi (do this last since rsi was used as the pointer)
    // `ret` pops [rsp] and jumps to it — the return address from the original
    // `call switch_context` in yield_cpu(). This properly unwinds the call frame.
    "ret"
);

core::arch::global_asm!(
    ".global jump_to_user",
    "jump_to_user:",
    // rdi = user_rip
    // rsi = user_rsp

    // Disable interrupts before setting up state to avoid getting interrupted mid-transition
    "cli",
    // iretq requires a 5-word frame on the stack:
    // SS, RSP, RFLAGS, CS, RIP

    // Push SS (User Data = 0x1B)
    "push 0x1B",
    // Push User RSP
    "push rsi",
    // Push RFLAGS (IF bit SET = 0x202, so interrupts are ENABLED in user space)
    "push 0x202",
    // Push CS (User Code = 0x23)
    "push 0x23",
    // Push User RIP
    "push rdi",
    // Switch to user stack is handled by iretq popping RSP
    "iretq"
);

core::arch::global_asm!(
    ".global jump_to_kernel_task",
    "jump_to_kernel_task:",
    // rdi = kernel_rip
    // rsi = kernel_rsp
    "cli",
    // Push SS (Kernel Data = 0x10)
    "push 0x10",
    // Push Kernel RSP
    "push rsi",
    // Push RFLAGS (IF bit SET = 0x202)
    "push 0x202",
    // Push CS (Kernel Code = 0x8)
    "push 0x8",
    // Push Kernel RIP
    "push rdi",
    "iretq"
);

extern "C" {
    pub fn switch_context(prev: *mut TaskContext, next: *const TaskContext);
    pub fn jump_to_user(user_rip: u64, user_rsp: u64) -> !;
    pub fn jump_to_kernel_task(kernel_rip: u64, kernel_rsp: u64) -> !;
}
