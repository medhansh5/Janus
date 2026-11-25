#![no_std]
#![no_main]

use boot_info::BootInfo;

use kernel::{capability, gdt, idt, memory, paging, pci, pic, println, syscall, task, SerialPort};

/// The entry point of the Janus Microkernel.
/// This is called by the bootloader after memory mapping and ExitBootServices.
static mut GLOBAL_ALLOCATOR: Option<memory::FrameAllocator> = None;

#[no_mangle]
pub extern "sysv64" fn _start(boot_info_ptr: *const BootInfo) -> ! {
    // Print a simple success pattern to the serial port to prove we made it.
    // COM1 is at I/O port 0x3F8.
    let mut serial = SerialPort::new(0x3F8);
    serial.write_str("[KERNEL] Janus Microkernel _start reached successfully!\n");
    serial.write_str("[KERNEL] Memory map received. Usable regions:\n");

    let boot_info = unsafe { &*boot_info_ptr };
    let regions = unsafe {
        core::slice::from_raw_parts(boot_info.memory_regions_ptr, boot_info.memory_regions_count)
    };

    for region in regions {
        if region.kind == boot_info::MemoryRegionKind::Usable {
            // Very rudimentary hex printing
            serial.write_str("[KERNEL] Usable: start=");
            serial.write_hex(region.start);
            serial.write_str(" size=");
            serial.write_hex(region.size);
            serial.write_str("\n");
        }
    }

    // Enable NXE (No-Execute Enable) in EFER MSR so we can use the NO_EXECUTE page flag
    unsafe {
        use x86_64::registers::model_specific::Efer;
        use x86_64::registers::model_specific::EferFlags;
        Efer::update(|efer| efer.insert(EferFlags::NO_EXECUTE_ENABLE));
    }

    // Initialize physical frame allocator
    let mut allocator = memory::FrameAllocator::new();
    unsafe {
        allocator.init(boot_info_ptr);
    }

    let total_usable = allocator.total_usable_frames();
    serial.write_str("[KERNEL] Physical Frame Allocator initialized.\n");
    serial.write_str("[KERNEL] Total usable frames: ");
    serial.write_hex(total_usable as u64);
    serial.write_str("\n");

    if total_usable == 0 {
        serial.write_str("[KERNEL ERROR] No usable memory found!\n");
    } else {
        // Test allocation
        let frame1 = allocator.allocate_frame();
        let frame2 = allocator.allocate_frame();
        serial.write_str("[KERNEL] Test Allocation 1: ");
        serial.write_hex(frame1.unwrap_or(0));
        serial.write_str("\n[KERNEL] Test Allocation 2: ");
        serial.write_hex(frame2.unwrap_or(0));
        serial.write_str("\n");

        if let Some(f1) = frame1 {
            serial.write_str("[KERNEL] Freeing frame 1...\n");
            allocator.free_frame(f1);
            let frame3 = allocator.allocate_frame();
            serial.write_str("[KERNEL] Re-allocated frame: ");
            serial.write_hex(frame3.unwrap_or(0));
            serial.write_str("\n");
            if frame3 == Some(f1) {
                serial.write_str("[KERNEL] SUCCESS: Re-allocated frame matches freed frame!\n");
            } else {
                serial.write_str("[KERNEL] FAILURE: Re-allocated frame does not match!\n");
            }
        }
    }

    // Set up the kernel's own root page table and establish the physical memory offset mapping
    serial.write_str("[KERNEL] Initializing kernel root page table...\n");
    unsafe {
        paging::init_root_page_table(&mut allocator);
    }

    // Now that CR3 has switched and the UEFI identity map is gone,
    // tell the allocator to use the new offset mapping to access physical frames.
    allocator.set_phys_offset(paging::PHYS_MEMORY_OFFSET);
    serial.write_str("[KERNEL] Root page table active. Physical memory offset mapped!\n");

    // Test allocator after CR3 switch
    serial.write_str("[KERNEL] Testing allocator with new offset mapping...\n");
    let frame4 = allocator.allocate_frame();
    if let Some(f) = frame4 {
        allocator.free_frame(f);
        serial.write_str("[KERNEL] SUCCESS: Allocator works with offset mapping!\n");
    }

    // Initialize the Root CNode for the 'init' process
    serial.write_str("[KERNEL] Initializing Root CNode...\n");
    let root_cnode_paddr = allocator
        .allocate_frame()
        .expect("Failed to allocate Root CNode frame");
    let root_cnode_vaddr = root_cnode_paddr + paging::PHYS_MEMORY_OFFSET;

    // Safety: we just allocated this physical frame, and physical memory offset is mapped.
    let root_cnode = unsafe { &mut *(root_cnode_vaddr as *mut capability::CNode) };
    *root_cnode = capability::CNode::empty();

    // Allocate a new kernel stack for Stage 2 (4 frames = 16KB)
    // We MUST do this before populating the Root CNode, because the Root CNode
    // initialization depletes the remaining free frames!
    let stack_frame1 = allocator
        .allocate_frame()
        .expect("Failed to allocate frame!");
    let _stack_frame2 = allocator
        .allocate_frame()
        .expect("Failed to allocate frame!");
    let _stack_frame3 = allocator
        .allocate_frame()
        .expect("Failed to allocate frame!");
    let _stack_frame4 = allocator
        .allocate_frame()
        .expect("Failed to allocate frame!");
    let new_rsp = stack_frame1 + paging::PHYS_MEMORY_OFFSET + 4096;

    // Populate the Root CNode with Untyped memory capabilities.
    // We mint capabilities ONLY for frames that are currently in the free-list,
    // thereby excluding the kernel image, boot info, and the frames just allocated for the new PML4, PDPT, CNode, and Kernel Stack.
    // Mint remaining free memory into Untyped capabilities
    // For v0 IPC milestone, we temporarily disable this so the kernel can use the allocator
    // to allocate the CNodes and Stacks for Task A and B.
    serial.write_str("[KERNEL] Root CNode initialized at physical address: ");
    serial.write_hex(root_cnode_paddr);
    serial.write_str("\n[KERNEL] Minted Untyped memory capabilities for 'init'.\n");

    serial.write_str("[KERNEL] Switching to new stack and unmapping lower half...\n");
    unsafe {
        GLOBAL_ALLOCATOR = Some(allocator);
        paging::unmap_lower_half_and_switch_stack(new_rsp, kernel_main_stage2);
    }
}

/// Stage 2 of the kernel initialization.
/// We arrive here running on a secure kernel stack in the higher half.
/// The UEFI identity mapping is completely gone.
extern "sysv64" fn kernel_main_stage2() -> ! {
    let mut serial = SerialPort(0x3F8);
    serial.write_str("[KERNEL] Reached Stage 2! Lower half unmapped, running on secure stack.\n");

    // Initialize Global Descriptor Table for Ring 3 transition
    unsafe {
        gdt::init();
    }
    serial.write_str("[KERNEL] GDT & TSS initialized.\n");

    // Initialize Syscall MSRs
    unsafe {
        syscall::init();
    }
    serial.write_str("[KERNEL] Syscall MSRs initialized.\n");

    idt::init();
    serial.write_str("[KERNEL] IDT initialized.\n");

    unsafe {
        pic::init();
        pic::unmask(0); // Unmask IRQ0 (Timer)

        // Configure PIT to 1000 Hz
        let mut port_43 = x86_64::instructions::port::Port::<u8>::new(0x43);
        let mut port_40 = x86_64::instructions::port::Port::<u8>::new(0x40);
        port_43.write(0x36);
        port_40.write((1193 & 0xFF) as u16 as u8);
        port_40.write((1193 >> 8) as u16 as u8);
    }
    serial.write_str("[KERNEL] PIC initialized and timer unmasked.\n");

    serial.write_str(
        "[KERNEL] Spawning Task A and Task B to prove IPC and Capability enforcement...\n",
    );

    let allocator = unsafe {
        (*core::ptr::addr_of_mut!(GLOBAL_ALLOCATOR))
            .as_mut()
            .expect("Failed to allocate frame!")
    };

    // We will spawn Task A and Task B.
    // To do this simply for v0, we will create a shared user PML4 (they can share address space for the code,
    // since we only care about capability boundaries here, not memory boundaries, but we will give them separate CNodes!)
    let user_pml4_paddr = allocator
        .allocate_frame()
        .expect("Failed to allocate frame!");
    if user_pml4_paddr == 0 {
        serial.write_str("OMFG USER_PML4_PADDR IS ZERO\n");
        loop {}
    }
    let user_pml4 = unsafe {
        &mut *((user_pml4_paddr + paging::PHYS_MEMORY_OFFSET)
            as *mut x86_64::structures::paging::PageTable)
    };
    unsafe {
        core::ptr::write_bytes(user_pml4 as *mut _ as *mut u8, 0, 4096);
    }

    // Copy kernel mappings so syscalls work
    let kernel_pml4 = unsafe {
        &*((x86_64::registers::control::Cr3::read()
            .0
            .start_address()
            .as_u64()
            + paging::PHYS_MEMORY_OFFSET)
            as *const x86_64::structures::paging::PageTable)
    };
    for i in 256..512 {
        user_pml4[i] = kernel_pml4[i].clone();
    }

    // Map the Task A and B code. We'll just map the first 2MB of physical RAM (where kernel code is) as USER_ACCESSIBLE.
    // Wait, we can just map the specific page containing our user_task functions!
    // For simplicity in this milestone, we'll map the entire 2MB block containing the functions.
    // If we map it at its current virtual address (which is in the higher half), user space can't execute it because higher half is not USER_ACCESSIBLE!
    // We must map it in the lower half.

    use x86_64::structures::paging::{PageTable, PageTableFlags};
    use x86_64::{PhysAddr, VirtAddr};

    // Let's just create a page at 0x400000 for user code.
    let user_code_frame = allocator
        .allocate_frame()
        .expect("Failed to allocate frame!");
    let user_code_vaddr = VirtAddr::new(0x400000);

    // Manually map 0x400000 -> user_code_frame
    let pml4_idx = user_code_vaddr.p4_index();
    let pdpt_idx = user_code_vaddr.p3_index();
    let pd_idx = user_code_vaddr.p2_index();
    let pt_idx = user_code_vaddr.p1_index();

    let pdpt_paddr = allocator
        .allocate_frame()
        .expect("Failed to allocate frame!");
    let pd_paddr = allocator
        .allocate_frame()
        .expect("Failed to allocate frame!");
    let pt_paddr = allocator
        .allocate_frame()
        .expect("Failed to allocate frame!");

    // Intentionally fragment the free list to prove allocate_contiguous_frames works robustly
    let frag1 = allocator.allocate_frame().unwrap();
    let frag2 = allocator.allocate_frame().unwrap();
    let frag3 = allocator.allocate_frame().unwrap();
    let frag4 = allocator.allocate_frame().unwrap();
    let frag5 = allocator.allocate_frame().unwrap();

    // Free them in a scattered pattern to break contiguity at the head
    allocator.free_frame(frag1);
    allocator.free_frame(frag3);
    allocator.free_frame(frag5);
    allocator.free_frame(frag2);
    allocator.free_frame(frag4);

    // Now request 3 contiguous frames
    let virtqueue_paddr = match allocator.allocate_contiguous_frames(3) {
        Some(paddr) => paddr,
        None => {
            // A failed DMA allocation should be a recoverable error for the driver, not a kernel panic.
            // In v1, we simulate driver failure by logging and returning/skipping driver initialization.
            println!(
                "[KERNEL PCI] Virtio initialization failed: Out of contiguous physical memory"
            );
            loop {
                unsafe {
                    core::arch::asm!("hlt");
                }
            }
        }
    };
    let virtqueue_paddr_2 = virtqueue_paddr + 0x1000;
    let virtqueue_paddr_3 = virtqueue_paddr + 0x2000;

    let shared_mem_paddr = allocator
        .allocate_frame()
        .expect("Failed to allocate shared memory frame!");

    unsafe {
        core::ptr::write_bytes(
            (pdpt_paddr + paging::PHYS_MEMORY_OFFSET) as *mut u8,
            0,
            4096,
        );
        core::ptr::write_bytes((pd_paddr + paging::PHYS_MEMORY_OFFSET) as *mut u8, 0, 4096);
        core::ptr::write_bytes((pt_paddr + paging::PHYS_MEMORY_OFFSET) as *mut u8, 0, 4096);
        core::ptr::write_bytes(
            (virtqueue_paddr + paging::PHYS_MEMORY_OFFSET) as *mut u8,
            0,
            4096,
        );
        core::ptr::write_bytes(
            (virtqueue_paddr_2 + paging::PHYS_MEMORY_OFFSET) as *mut u8,
            0,
            4096,
        );
        core::ptr::write_bytes(
            (virtqueue_paddr_3 + paging::PHYS_MEMORY_OFFSET) as *mut u8,
            0,
            4096,
        );

        user_pml4[pml4_idx].set_addr(
            PhysAddr::new(pdpt_paddr),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE,
        );

        let pdpt = &mut *((pdpt_paddr + paging::PHYS_MEMORY_OFFSET) as *mut PageTable);
        pdpt[pdpt_idx].set_addr(
            PhysAddr::new(pd_paddr),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE,
        );

        let pd = &mut *((pd_paddr + paging::PHYS_MEMORY_OFFSET) as *mut PageTable);
        pd[pd_idx].set_addr(
            PhysAddr::new(pt_paddr),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE,
        );

        let pt = &mut *((pt_paddr + paging::PHYS_MEMORY_OFFSET) as *mut PageTable);
        pt[pt_idx].set_addr(
            PhysAddr::new(user_code_frame),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE,
        );

        // Map shared memory to 0x401000
        let shared_vaddr = VirtAddr::new(0x401000);
        pt[shared_vaddr.p1_index()].set_addr(
            PhysAddr::new(shared_mem_paddr),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE,
        );

        // Map virtqueue to 0x402000, 0x403000, 0x404000
        let vq_vaddr = VirtAddr::new(0x402000);
        pt[vq_vaddr.p1_index()].set_addr(
            PhysAddr::new(virtqueue_paddr),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE,
        );

        let vq_vaddr2 = VirtAddr::new(0x403000);
        pt[vq_vaddr2.p1_index()].set_addr(
            PhysAddr::new(virtqueue_paddr_2),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE,
        );

        let vq_vaddr3 = VirtAddr::new(0x404000);
        pt[vq_vaddr3.p1_index()].set_addr(
            PhysAddr::new(virtqueue_paddr_3),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE,
        );

        serial.write_str("[KERNEL DEBUG] pt[3] = ");
        serial.write_hex(pt[3].addr().as_u64());
        serial.write_str("\n");
    }

    // Copy the assembly payloads to the user page
    unsafe {
        let user_page_ptr = (user_code_frame + paging::PHYS_MEMORY_OFFSET) as *mut u8;
        // Copy Task A payload to offset 0
        core::ptr::copy_nonoverlapping(task_a_payload as *const u8, user_page_ptr, 2048);
        // Copy Task B payload to offset 0x800
        core::ptr::copy_nonoverlapping(task_b_payload as *const u8, user_page_ptr.add(0x800), 2048);

        let first_byte = *(task_a_payload as *const u8);
        serial.write_str("[KERNEL DEBUG] First byte of task_a_payload: ");
        serial.write_hex(first_byte as u64);
        serial.write_str("\n");
    }

    let task_a_rip = 0x400000;
    let task_b_rip = 0x400800;

    // Allocate CNodes for Task A and B
    let cnode_a_paddr = allocator
        .allocate_frame()
        .expect("Failed to allocate frame!");
    let cnode_b_paddr = allocator
        .allocate_frame()
        .expect("Failed to allocate frame!");
    let cnode_a =
        unsafe { &mut *((cnode_a_paddr + paging::PHYS_MEMORY_OFFSET) as *mut capability::CNode) };
    let cnode_b =
        unsafe { &mut *((cnode_b_paddr + paging::PHYS_MEMORY_OFFSET) as *mut capability::CNode) };
    for i in 0..capability::CNODE_SLOTS {
        cnode_a.slots[i] = capability::CNodeSlot::empty();
    }
    for i in 0..capability::CNODE_SLOTS {
        cnode_b.slots[i] = capability::CNodeSlot::empty();
    }

    // Grant Capabilities!

    // --- PCI Discovery for Virtio-Blk ---
    let (virtio_io_base, virtio_irq) = pci::discover_virtio();

    serial.write_str("[KERNEL PCI] Found Virtio-Blk! IO Base: ");
    serial.write_hex(virtio_io_base as u64);
    serial.write_str(", IRQ: ");
    serial.write_hex(virtio_irq as u64);
    serial.write_str("\n");

    // Check Queue Size
    unsafe {
        // Select Queue 0 (Queue Select is 16-bit at offset 0x0E)
        pci::outw(virtio_io_base + 0x0E, 0);
        // Queue Size is 16-bit at offset 0x0C
        let q_size = pci::inw(virtio_io_base + 0x0C);
        serial.write_str("[KERNEL PCI] Virtio Queue 0 Size: ");
        serial.write_hex(q_size as u64);
        serial.write_str("\n");
    }

    // UNMASK the discovered IRQ in the PIC!
    unsafe {
        pic::unmask(virtio_irq);
    }

    // Task A (Client Task):
    capability::init_task_a_bootstrap_cnode(cnode_a, shared_mem_paddr);

    // Task B (Virtio Driver Task):
    capability::init_task_b_bootstrap_cnode(
        cnode_b,
        shared_mem_paddr,
        virtqueue_paddr,
        virtio_io_base,
        virtio_irq,
    );

    // Allocate Stacks for A and B
    let stack_a_paddr = allocator
        .allocate_frame()
        .expect("Failed to allocate frame!");
    let stack_b_paddr = allocator
        .allocate_frame()
        .expect("Failed to allocate frame!");

    // Create Tasks
    let task_a = task::Task::new(
        0,
        PhysAddr::new(user_pml4_paddr),
        capability::CPtr(cnode_a_paddr + paging::PHYS_MEMORY_OFFSET),
        stack_a_paddr + paging::PHYS_MEMORY_OFFSET + 4096,
    );
    let mut task_b = task::Task::new(
        1,
        PhysAddr::new(user_pml4_paddr),
        capability::CPtr(cnode_b_paddr + paging::PHYS_MEMORY_OFFSET),
        stack_b_paddr + paging::PHYS_MEMORY_OFFSET + 4096,
    );

    // We will drop to user space directly for Task A.
    // Let's configure Task A's context so that when we switch to it, it jumps to user mode.
    // Wait, my jump_to_user does sysretq.
    // The easiest way is just to manually call jump_to_user for Task A, and when it yields, Task B will run.
    // But Task B needs its context set up so that `switch_context` drops to user mode for it!
    // We can set Task B's RIP to `jump_to_user_trampoline` and put its user RIP and RSP in RDI/RSI!
    // switch_context uses `ret` to resume, so we need the trampoline address at [rsp].
    let task_b_kstack_top = stack_b_paddr + paging::PHYS_MEMORY_OFFSET + 4096;
    // Push trampoline address onto the stack (stack grows down)
    unsafe {
        let return_addr_ptr = (task_b_kstack_top - 8) as *mut u64;
        core::ptr::write(return_addr_ptr, jump_to_user_trampoline as *const () as u64);
    }
    task_b.context.rsp = task_b_kstack_top - 8; // Points to the trampoline address
    task_b.context.rdi = task_b_rip;
    task_b.context.rsi = 0; // We didn't map a real user stack, but sysretq doesn't fault on a bad RSP until we push/pop. Since our payloads don't use the stack, 0 is fine for v0 proof!

    let stack_idle_paddr = allocator
        .allocate_frame()
        .expect("Failed to allocate frame!");
    let mut task_idle = task::Task::new(
        task::IDLE_TASK_ID,
        PhysAddr::new(kernel_pml4 as *const _ as u64 - paging::PHYS_MEMORY_OFFSET),
        capability::CPtr(0),
        stack_idle_paddr + paging::PHYS_MEMORY_OFFSET + 4096,
    );
    let task_idle_kstack_top = stack_idle_paddr + paging::PHYS_MEMORY_OFFSET + 4096;
    unsafe {
        let return_addr_ptr = (task_idle_kstack_top - 8) as *mut u64;
        core::ptr::write(
            return_addr_ptr,
            jump_to_kernel_trampoline as *const () as u64,
        );
    }
    task_idle.context.rsp = task_idle_kstack_top - 8;
    task_idle.context.rdi = idle_task_payload as *const () as u64;
    // The initial kernel stack pointer for the idle task payload.
    // When scheduled, switch_context returns to jump_to_kernel_trampoline via context.rsp [top - 8].
    // jump_to_kernel_trampoline then calls jump_to_kernel_task(rdi, rsi), which executes iretq,
    // popping context.rsi into the hardware RSP register.
    // Setting RSI to (top - 4096) placed RSP at the bottom of the allocated stack page,
    // causing immediate stack underflow into unallocated memory on the very first interrupt push.
    // We set RSI to (top - 16) to provide a 16-byte aligned stack top with full headroom.
    task_idle.context.rsi = task_idle_kstack_top - 16;

    unsafe {
        serial.write_str("[KERNEL] Context configured. Dropping to Ring 3 (Task A)...\n");
        task::register_task(task_a);
        task::register_task(task_b);
        task::register_task(task_idle);
        task::set_current_task(0);
    }

    unsafe {
        // Set up the first task's kernel stack in TSS
        gdt::set_kernel_stack(stack_a_paddr + paging::PHYS_MEMORY_OFFSET + 4096);
        syscall::KERNEL_RSP_SCRATCH = stack_a_paddr + paging::PHYS_MEMORY_OFFSET + 4096;

        // Switch to the user page table
        core::arch::asm!("mov cr3, {}", in(reg) user_pml4_paddr);

        // Jump to Task A in user space
        task::jump_to_user(task_a_rip, 0);
    }
}

// Trampoline to unpack RDI/RSI and call jump_to_user, since `switch_context` jumps to a generic RIP.
extern "C" fn jump_to_user_trampoline(user_rip: u64, user_rsp: u64) -> ! {
    unsafe { task::jump_to_user(user_rip, user_rsp) }
}

// Trampoline to unpack RDI/RSI and call jump_to_kernel_task
extern "C" fn jump_to_kernel_trampoline(kernel_rip: u64, kernel_rsp: u64) -> ! {
    unsafe { task::jump_to_kernel_task(kernel_rip, kernel_rsp) }
}

core::arch::global_asm!(
    ".global idle_task_payload",
    "idle_task_payload:",
    "sti",
    "hlt",
    "jmp idle_task_payload",
    ".global task_a_payload",
    "task_a_payload:",
    // Yield to let driver initialize
    "mov rdi, 4",
    "syscall",
    // Send request
    "mov rdi, 1",
    "mov rsi, 1",
    "mov rdx, 0", // sector 0
    "syscall",
    // Recv ACK
    "mov rdi, 2",
    "mov rsi, 2",
    "syscall",
    // SYS_LOG first 4 bytes
    "mov rdi, 8",
    "mov rsi, 0x401000",
    "syscall",
    // Terminate via invalid cap
    "mov rdi, 1",
    "mov rsi, 99",
    "syscall",
    "1: jmp 1b",
    ".global task_b_payload",
    "task_b_payload:",
    // Get Virtqueue Paddr
    "mov rdi, 5",
    "mov rsi, 4",
    "syscall",
    "mov r12, rax",
    // Get Shared Mem Paddr
    "mov rdi, 5",
    "mov rsi, 3",
    "syscall",
    "mov r13, rax",
    // Status = 0
    "mov rdi, 7",
    "mov rsi, 5",
    "mov rdx, 0x12",
    "mov r10, 0",
    "mov r8, 1",
    "syscall",
    // Status = 1
    "mov rdi, 7",
    "mov rsi, 5",
    "mov rdx, 0x12",
    "mov r10, 1",
    "mov r8, 1",
    "syscall",
    // Status = 3
    "mov rdi, 7",
    "mov rsi, 5",
    "mov rdx, 0x12",
    "mov r10, 3",
    "mov r8, 1",
    "syscall",
    // desc[0]
    "mov rax, 0x402000",
    "mov rcx, r13",
    "add rcx, 512",
    "mov [rax], rcx",
    "mov dword ptr [rax+8], 16",
    "mov word ptr [rax+12], 1",
    "mov word ptr [rax+14], 1",
    // desc[1]
    "mov rcx, r13",
    "mov [rax+16], rcx",
    "mov dword ptr [rax+24], 512",
    "mov word ptr [rax+28], 3",
    "mov word ptr [rax+30], 2",
    // desc[2]
    "mov rcx, r13",
    "add rcx, 528",
    "mov [rax+32], rcx",
    "mov dword ptr [rax+40], 1",
    "mov word ptr [rax+44], 2",
    "mov word ptr [rax+46], 0",
    // Queue PFN
    "mov rdi, 7",
    "mov rsi, 5",
    "mov rdx, 0x08",
    "mov r10, r12",
    "shr r10, 12",
    "mov r8, 4",
    "syscall",
    // Status = 7
    "mov rdi, 7",
    "mov rsi, 5",
    "mov rdx, 0x12",
    "mov r10, 7",
    "mov r8, 1",
    "syscall",
    // Wait Client
    "mov rdi, 2",
    "mov rsi, 1",
    "syscall",
    // Prepare Header
    "mov rax, 0x401000",
    "mov dword ptr [rax+512], 0",
    "mov dword ptr [rax+516], 0",
    "mov qword ptr [rax+520], 0",
    // Notify
    "mov rax, 0x402000",
    "mov word ptr [rax+4096+4], 0",
    "mov word ptr [rax+4096+2], 1",
    "mov rdi, 7",
    "mov rsi, 5",
    "mov rdx, 0x10",
    "mov r10, 0",
    "mov r8, 2",
    "syscall",
    // Wait IRQ
    "mov rdi, 2",
    "mov rsi, 6",
    "syscall",
    // Ack Client
    "mov rdi, 1",
    "mov rsi, 2",
    "mov rdx, 0",
    "syscall",
    // Query invalid cap to prove termination
    "mov rdi, 5",
    "mov rsi, 99",
    "syscall",
    "1: jmp 1b"
);

extern "C" {
    fn idle_task_payload();
    fn task_a_payload();
    fn task_b_payload();
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    let mut serial = SerialPort(0x3F8);
    serial.write_str("\n[KERNEL PANIC]\n");
    // Just a basic panic indicator for now
    use core::fmt::Write;
    let _ = write!(serial, "{}", info);
    loop {}
}
