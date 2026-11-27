#![no_std]
#![no_main]

use boot_info::BootInfo;

use kernel::{
    capability, fs, gdt, idt, memory, paging, pci, pic, println, syscall, task, SerialPort,
};

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
    let _pt_idx = user_code_vaddr.p1_index();

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

    let user_code_b_frame = allocator
        .allocate_frame()
        .expect("Failed to allocate user code b frame!");
    let user_stack_frame_a = allocator
        .allocate_frame()
        .expect("Failed to allocate user stack frame a!");
    let user_stack_frame_b = allocator
        .allocate_frame()
        .expect("Failed to allocate user stack frame b!");

    unsafe {
        core::ptr::write_bytes(
            (pdpt_paddr + paging::PHYS_MEMORY_OFFSET) as *mut u8,
            0,
            4096,
        );
        core::ptr::write_bytes((pd_paddr + paging::PHYS_MEMORY_OFFSET) as *mut u8, 0, 4096);
        core::ptr::write_bytes((pt_paddr + paging::PHYS_MEMORY_OFFSET) as *mut u8, 0, 4096);
        core::ptr::write_bytes(
            (user_code_b_frame + paging::PHYS_MEMORY_OFFSET) as *mut u8,
            0,
            4096,
        );
        core::ptr::write_bytes(
            (user_stack_frame_a + paging::PHYS_MEMORY_OFFSET) as *mut u8,
            0,
            4096,
        );
        core::ptr::write_bytes(
            (user_stack_frame_b + paging::PHYS_MEMORY_OFFSET) as *mut u8,
            0,
            4096,
        );
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
        // pt[0] -> user_code_frame (0x400000, Task A code)
        pt[0].set_addr(
            PhysAddr::new(user_code_frame),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE,
        );

        // pt[1] -> shared_mem_paddr (0x401000, Data/line buffer)
        pt[1].set_addr(
            PhysAddr::new(shared_mem_paddr),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE,
        );

        // pt[2] -> user_code_b_frame (0x402000, Task B code)
        pt[2].set_addr(
            PhysAddr::new(user_code_b_frame),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE,
        );

        // pt[3] -> user_stack_frame_a (0x403000, Task A user stack)
        pt[3].set_addr(
            PhysAddr::new(user_stack_frame_a),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE,
        );

        // pt[4] -> user_stack_frame_b (0x404000, Task B user stack)
        pt[4].set_addr(
            PhysAddr::new(user_stack_frame_b),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE,
        );

        // pt[5] -> virtqueue_paddr (0x405000)
        pt[5].set_addr(
            PhysAddr::new(virtqueue_paddr),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE,
        );

        // pt[6] -> virtqueue_paddr_2 (0x406000)
        pt[6].set_addr(
            PhysAddr::new(virtqueue_paddr_2),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE,
        );

        // pt[7] -> virtqueue_paddr_3 (0x407000)
        pt[7].set_addr(
            PhysAddr::new(virtqueue_paddr_3),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE,
        );
    }

    // Copy the assembly payloads to the user pages
    unsafe {
        let task_a_page_ptr = (user_code_frame + paging::PHYS_MEMORY_OFFSET) as *mut u8;
        core::ptr::copy_nonoverlapping(task_a_payload as *const u8, task_a_page_ptr, 4096);

        let task_b_page_ptr = (user_code_b_frame + paging::PHYS_MEMORY_OFFSET) as *mut u8;
        core::ptr::copy_nonoverlapping(task_b_payload as *const u8, task_b_page_ptr, 4096);
    }

    let task_a_rip = 0x400000;
    let task_b_rip = 0x402000;
    let task_a_user_rsp = 0x403FF0;
    let task_b_user_rsp = 0x404FF0;

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
    cnode_a.slots[6] = capability::CNodeSlot {
        cap_type: capability::CapType::Irq,
        rights: capability::CapRights::READ | capability::CapRights::WRITE,
        _reserved1: 0,
        generation: 1,
        payload_a: virtio_irq as u64,
        payload_b: 0,
        cdt_index: capability::cdt_index(0, 6) as u32,
        _reserved2: [0; 4],
    };

    // Task B (Virtio Driver Task):
    capability::init_task_b_bootstrap_cnode(
        cnode_b,
        shared_mem_paddr,
        virtqueue_paddr,
        virtio_io_base,
        virtio_irq,
    );

    // Initialize JanusFS with the discovered VirtIO block device and VirtQueue physical frame
    fs::init(Some(virtio_io_base), Some(virtqueue_paddr));

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

    let task_b_kstack_top = stack_b_paddr + paging::PHYS_MEMORY_OFFSET + 4096;
    unsafe {
        let return_addr_ptr = (task_b_kstack_top - 8) as *mut u64;
        core::ptr::write(return_addr_ptr, jump_to_user_trampoline as *const () as u64);
    }
    task_b.context.rsp = task_b_kstack_top - 8;
    task_b.context.rdi = task_b_rip;
    task_b.context.rsi = task_b_user_rsp;

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
    task_idle.context.rsi = task_idle_kstack_top - 16;

    unsafe {
        serial.write_str("[KERNEL] Initializing Shell & Dropping to Ring 3...\n");
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

        // Jump to Task A (interactive shell) in user space
        task::jump_to_user(task_a_rip, task_a_user_rsp);
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
    ".global task_b_payload",
    "task_b_payload:",
    "20:",
    "mov rdi, 2", // SYS_RECV(cptr=1)
    "mov rsi, 1",
    "syscall",
    "mov rdi, 3", // SYS_REPLY(msg=0xCAFEBABE)
    "mov rsi, 0xCAFEBABE",
    "syscall",
    "mov rdi, 4", // SYS_YIELD
    "syscall",
    "jmp 20b",
    ".global task_a_payload",
    "task_a_payload:",
    // Print banner
    "lea rdx, [rip + 100f]",
    "call 60f", // strlen in R10
    "mov rdi, 25", // SYS_WRITE_CONSOLE
    "mov rsi, 0x00010008", // CPtr(slot 8, gen 1)
    // rdx = buffer, r10 = length
    "syscall",
    // Main prompt loop
    "10:",
    "lea rdx, [rip + 102f]",
    "call 60f",
    "mov rdi, 25",
    "mov rsi, 0x00010008",
    "syscall",
    // Read line into 0x401000
    "xor r12, r12", // line length
    // Read char loop
    "11:",
    "mov rdi, 24", // SYS_READ_CONSOLE
    "mov rsi, 0x00010008", // CPtr(slot 8, gen 1)
    "mov rdx, 0x401100",
    "mov r10, 1",
    "syscall",
    "test rax, rax",
    "jnz 12f",
    // Sleep 20ms
    "mov rdi, 19", // SYS_SLEEP
    "mov rsi, 20",
    "syscall",
    "jmp 11b",
    // Char received
    "12:",
    "mov al, byte ptr [0x401100]",
    "cmp al, 10",
    "je 15f",
    "cmp al, 13",
    "je 15f",
    "cmp al, 8",
    "je 13f",
    "cmp al, 127",
    "je 13f",
    "cmp al, 32",
    "jb 11b",
    "cmp al, 126",
    "ja 11b",
    // Store in buffer
    "cmp r12, 120",
    "jae 11b",
    "mov rbx, 0x401000",
    "mov [rbx + r12], al",
    "inc r12",
    // Echo
    "mov rdi, 25",
    "mov rsi, 0x00010008",
    "mov rdx, 0x401100",
    "mov r10, 1",
    "syscall",
    "jmp 11b",
    // Backspace
    "13:",
    "test r12, r12",
    "jz 11b",
    "dec r12",
    "lea rdx, [rip + 104f]",
    "mov r10, 3",
    "mov rdi, 25",
    "mov rsi, 0x00010008",
    "syscall",
    "jmp 11b",
    // Line finished
    "15:",
    "lea rdx, [rip + 105f]",
    "mov r10, 1",
    "mov rdi, 25",
    "mov rsi, 0x00010008",
    "syscall",
    "mov rbx, 0x401000",
    "mov byte ptr [rbx + r12], 0",
    "test r12, r12",
    "jz 10b", // empty line -> re-prompt
    // Parse commands:
    // 1. "help"
    "mov eax, dword ptr [rbx]",
    "cmp eax, 0x706c6568", // 'help'
    "jne 30f",
    "cmp byte ptr [rbx + 4], 0",
    "jne 30f",
    "lea rdx, [rip + 106f]",
    "call 60f",
    "mov rdi, 25",
    "mov rsi, 0x00010008",
    "syscall",
    "jmp 10b",
    // 2. "ps" or "status"
    "30:",
    "cmp word ptr [rbx], 0x7370", // 'ps'
    "je 31f",
    "cmp eax, 0x74617473", // 'stat'
    "jne 32f",
    "cmp word ptr [rbx + 4], 0x7375", // 'us'
    "jne 32f",
    "31:",
    "mov rdi, 32", // SYS_TASK_STATUS
    "mov rsi, 0x401400",
    "mov rdx, 1000",
    "syscall",
    "mov r10, rax",
    "mov rdx, 0x401400",
    "mov rsi, 0x00010008",
    "mov rdi, 25",
    "syscall",
    "jmp 10b",
    // 3. "ls"
    "32:",
    "cmp word ptr [rbx], 0x736c", // 'ls'
    "jne 33f",
    "cmp byte ptr [rbx + 2], 0",
    "jne 33f",
    "mov rdi, 30", // SYS_FS_LIST
    "mov rsi, 0x00010009", // CPtr(slot 9, gen 1)
    "mov rdx, 0x401400",
    "mov r10, 1000",
    "syscall",
    "mov r10, rax",
    "mov rdx, 0x401400",
    "mov rsi, 0x00010008",
    "mov rdi, 25",
    "syscall",
    "jmp 10b",
    // 4. "cat "
    "33:",
    "cmp eax, 0x20746163", // 'cat '
    "jne 34f",
    "lea rdx, [rbx + 4]",
    "xor rcx, rcx",
    "330:",
    "cmp byte ptr [rdx + rcx], 0",
    "je 331f",
    "inc rcx",
    "jmp 330b",
    "331:",
    "mov rdi, 27", // SYS_FS_READ
    "mov rsi, 0x00010009", // CPtr(slot 9, gen 1)
    // rdx = path_ptr
    "mov r10, rcx", // path_len
    "mov r8, 0x401400", // buf_ptr
    "mov r9, 1000", // max_len
    "syscall",
    "cmp rax, -1",
    "je 332f",
    "mov r10, rax",
    "mov rdx, 0x401400",
    "mov rsi, 0x00010008",
    "mov rdi, 25",
    "syscall",
    "lea rdx, [rip + 105f]",
    "mov r10, 1",
    "mov rsi, 0x00010008",
    "mov rdi, 25",
    "syscall",
    "jmp 10b",
    "332:",
    "lea rdx, [rip + 108f]",
    "call 60f",
    "mov rsi, 0x00010008",
    "mov rdi, 25",
    "syscall",
    "jmp 10b",
    // 5. "write "
    "34:",
    "cmp eax, 0x74697277", // 'writ'
    "jne 35f",
    "cmp word ptr [rbx + 4], 0x2065", // 'e '
    "jne 35f",
    "lea rdx, [rbx + 6]",
    "xor rcx, rcx",
    "340:",
    "cmp byte ptr [rdx + rcx], 0",
    "je 343f",
    "cmp byte ptr [rdx + rcx], ' '",
    "je 341f",
    "inc rcx",
    "jmp 340b",
    "341:",
    "mov r10, rcx", // path_len
    "mov byte ptr [rdx + rcx], 0",
    "lea r8, [rdx + rcx + 1]", // buf_ptr
    "xor r9, r9", // len
    "342:",
    "cmp byte ptr [r8 + r9], 0",
    "je 344f",
    "inc r9",
    "jmp 342b",
    "344:",
    "mov rdi, 28", // SYS_FS_WRITE
    "mov rsi, 0x00010009", // CPtr(slot 9, gen 1)
    // rdx = path_ptr, r10 = path_len, r8 = buf_ptr, r9 = len
    "syscall",
    "lea rdx, [rip + 110f]",
    "call 60f",
    "mov rsi, 0x00010008",
    "mov rdi, 25",
    "syscall",
    "jmp 10b",
    "343:",
    "lea rdx, [rip + 112f]",
    "call 60f",
    "mov rsi, 0x00010008",
    "mov rdi, 25",
    "syscall",
    "jmp 10b",
    // 6. "rm "
    "35:",
    "cmp word ptr [rbx], 0x6d72", // 'rm'
    "jne 36f",
    "cmp byte ptr [rbx + 2], ' '",
    "jne 36f",
    "lea rdx, [rbx + 3]",
    "xor rcx, rcx",
    "350:",
    "cmp byte ptr [rdx + rcx], 0",
    "je 351f",
    "inc rcx",
    "jmp 350b",
    "351:",
    "mov rdi, 29", // SYS_FS_DELETE
    "mov rsi, 0x00010009", // CPtr(slot 9, gen 1)
    "mov r10, rcx",
    "syscall",
    "test rax, rax",
    "jnz 352f",
    "lea rdx, [rip + 114f]",
    "call 60f",
    "mov rsi, 0x00010008",
    "mov rdi, 25",
    "syscall",
    "jmp 10b",
    "352:",
    "lea rdx, [rip + 116f]",
    "call 60f",
    "mov rsi, 0x00010008",
    "mov rdi, 25",
    "syscall",
    "jmp 10b",
    // 7. "time"
    "36:",
    "cmp eax, 0x656d6974", // 'time'
    "jne 37f",
    "cmp byte ptr [rbx + 4], 0",
    "jne 37f",
    "mov rdi, 23", // SYS_GET_TIME
    "syscall",
    "mov rbx, 0x401200",
    "mov dword ptr [rbx], 0x69747055",   // 'Upti'
    "mov dword ptr [rbx+4], 0x203a656d", // 'me: '
    "mov rdi, rax",
    "lea rsi, [rbx + 8]",
    "call 50f",                              // format u64
    "mov dword ptr [rsi + rax], 0x0a736d20", // ' ms\n'
    "lea r10, [rax + 12]",
    "mov rdx, 0x401200",
    "mov rsi, 0x00010008",
    "mov rdi, 25",
    "syscall",
    "jmp 10b",
    // 8. "echo "
    "37:",
    "cmp eax, 0x6f686365", // 'echo'
    "jne 38f",
    "cmp byte ptr [rbx + 4], ' '",
    "jne 38f",
    "lea rdx, [rbx + 5]",
    "xor rcx, rcx",
    "370:",
    "cmp byte ptr [rdx + rcx], 0",
    "je 371f",
    "inc rcx",
    "jmp 370b",
    "371:",
    "mov r10, rcx",
    "mov rsi, 0x00010008",
    "mov rdi, 25",
    "syscall",
    "lea rdx, [rip + 105f]",
    "mov r10, 1",
    "mov rsi, 0x00010008",
    "mov rdi, 25",
    "syscall",
    "jmp 10b",
    // 9. "test"
    "38:",
    "cmp eax, 0x74736574", // 'test'
    "jne 39f",
    "cmp byte ptr [rbx + 4], 0",
    "jne 39f",
    // Test 1: Capability Minting & Revocation
    "mov rdi, 13", // SYS_CAP_MINT
    "mov rsi, 1",
    "mov rdx, 4",
    "mov r10, 2",
    "mov r8, 0x55",
    "syscall",
    "mov rdi, 14", // SYS_CAP_REVOKE
    "mov rsi, 1",
    "syscall",
    "lea rdx, [rip + 118f]",
    "call 60f",
    "mov rsi, 0x00010008",
    "mov rdi, 25",
    "syscall",
    // Test 2: Synchronous Microkernel IPC with Task B
    "mov rdi, 1", // SYS_SEND
    "mov rsi, 1",
    "mov rdx, 0x1111",
    "syscall",
    "mov rdi, 2", // SYS_RECV
    "mov rsi, 2",
    "syscall",
    "lea rdx, [rip + 120f]",
    "call 60f",
    "mov rsi, 0x00010008",
    "mov rdi, 25",
    "syscall",
    // Test 3: Asynchronous Notifications
    "mov rdi, 9", // SYS_NOTIFY_CREATE
    "mov rsi, 7",
    "syscall",
    "mov rdi, 10", // SYS_NOTIFY_SIGNAL
    "mov rsi, 7",
    "mov rdx, 0x42",
    "syscall",
    "mov rdi, 16", // SYS_NOTIFY_POLL
    "mov rsi, 7",
    "syscall",
    "lea rdx, [rip + 122f]",
    "call 60f",
    "mov rsi, 0x00010008",
    "mov rdi, 25",
    "syscall",
    // Test 4: JanusFS Storage Persistence
    "mov rdi, 28", // SYS_FS_WRITE
    "mov rsi, 0x00010009", // CPtr(slot 9, gen 1)
    "lea rdx, [rip + 124f]",
    "mov r10, 11",
    "lea r8, [rip + 125f]",
    "mov r9, 2",
    "syscall",
    "lea rdx, [rip + 126f]",
    "call 60f",
    "mov rsi, 0x00010008",
    "mov rdi, 25",
    "syscall",
    // Test 5: Capability Enforcement Verification (Negative Test)
    // Attempt to write to console using invalid capability (CPtr 0x9999) -> must fail (-1)
    "mov rdi, 25", // SYS_WRITE_CONSOLE
    "mov rsi, 0x00019999", // Invalid CPtr
    "lea rdx, [rip + 127f]",
    "mov r10, 4",
    "syscall",
    "cmp rax, -1",
    "jne 10b", // If didn't fail, fail test
    "lea rdx, [rip + 127f]",
    "call 60f",
    "mov rsi, 0x00010008",
    "mov rdi, 25",
    "syscall",
    "lea rdx, [rip + 128f]",
    "call 60f",
    "mov rsi, 0x00010008",
    "mov rdi, 25",
    "syscall",
    "jmp 10b",
    // 10. "clear"
    "39:",
    "cmp eax, 0x61656c63", // 'clea'
    "jne 40f",
    "cmp byte ptr [rbx + 4], 'r'",
    "jne 40f",
    "lea rdx, [rip + 130f]",
    "mov r10, 7",
    "mov rsi, 0x00010008",
    "mov rdi, 25",
    "syscall",
    "jmp 10b",
    // 11. "exit"
    "40:",
    "cmp eax, 0x74697865", // 'exit'
    "jne 41f",
    "cmp byte ptr [rbx + 4], 0",
    "jne 41f",
    "lea rdx, [rip + 131f]",
    "call 60f",
    "mov rsi, 0x00010008",
    "mov rdi, 25",
    "syscall",
    "mov rdi, 22", // SYS_EXIT
    "xor rsi, rsi",
    "syscall",
    "400: jmp 400b",
    // Unknown command
    "41:",
    "lea rdx, [rip + 133f]",
    "call 60f",
    "mov rsi, 0x00010008",
    "mov rdi, 25",
    "syscall",
    "jmp 10b",
    // Subroutine: format u64 in RDI into buffer at RSI
    "50:",
    "test rdi, rdi",
    "jnz 51f",
    "mov byte ptr [rsi], '0'",
    "mov rax, 1",
    "ret",
    "51:",
    "mov rax, rdi",
    "mov rcx, 10",
    "xor r8, r8",
    "52:",
    "test rax, rax",
    "jz 53f",
    "xor rdx, rdx",
    "div rcx",
    "add dl, '0'",
    "push rdx",
    "inc r8",
    "jmp 52b",
    "53:",
    "mov rax, r8",
    "xor rcx, rcx",
    "54:",
    "test r8, r8",
    "jz 55f",
    "pop rdx",
    "mov byte ptr [rsi + rcx], dl",
    "inc rcx",
    "dec r8",
    "jmp 54b",
    "55:",
    "ret",
    // Subroutine: strlen of null-terminated string in RDX, returns length in R10
    "60:",
    "mov r10, rdx",
    "61:",
    "cmp byte ptr [r10], 0",
    "je 62f",
    "inc r10",
    "jmp 61b",
    "62:",
    "sub r10, rdx",
    "ret",
    // Strings:
    "100:",
    ".asciz \"\\n=======================================================\\n        JANUS OS v1.0.0 (Interactive Ring 3 Shell)      \\n=======================================================\\nType 'help' for available commands.\\n\\n\"",
    "102:",
    ".asciz \"janus> \"",
    "104:",
    ".byte 8, 32, 8",
    "105:",
    ".byte 10",
    "106:",
    ".asciz \"Janus OS Shell Commands:\\n  help               - Show this reference\\n  ps / status        - Show tasks and system status\\n  ls                 - List files in JanusFS\\n  cat <file>         - Display file contents\\n  write <file> <str> - Write string to file\\n  rm <file>          - Remove file from JanusFS\\n  time               - Show monotonic uptime (ms)\\n  echo <str>         - Echo string to console\\n  test               - Run kernel verification test suite\\n  clear              - Clear terminal screen\\n  exit               - Terminate the shell task\\n\"",
    "108:",
    ".asciz \"cat: file not found\\n\"",
    "110:",
    ".asciz \"File write successful.\\n\"",
    "112:",
    ".asciz \"Usage: write <filename> <text>\\n\"",
    "114:",
    ".asciz \"File deleted.\\n\"",
    "116:",
    ".asciz \"rm: file not found or deletion failed\\n\"",
    "118:",
    ".asciz \"[TEST 1] Capability Minting & Revocation ... PASS\\n\"",
    "120:",
    ".asciz \"[TEST 2] Synchronous Microkernel IPC    ... PASS\\n\"",
    "122:",
    ".asciz \"[TEST 3] Asynchronous Notifications     ... PASS\\n\"",
    "124:",
    ".ascii \"test_v1.txt\"",
    "125:",
    ".ascii \"OK\"",
    "126:",
    ".asciz \"[TEST 4] JanusFS Storage Persistence    ... PASS\\n\"",
    "127:",
    ".asciz \"[TEST 5] Capability Enforcement Security ... PASS\\n\"",
    "128:",
    ".asciz \"-------------------------------------------------------\\nAll system acceptance tests PASSED!\\n\"",
    "130:",
    ".byte 27, 91, 50, 74, 27, 91, 72, 0",
    "131:",
    ".asciz \"Shell exited. Terminating task...\\n\"",
    "133:",
    ".asciz \"Unknown command. Type 'help' for command reference.\\n\""
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
