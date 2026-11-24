use core::cell::UnsafeCell;
use spin::Once;
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;
use x86_64::VirtAddr;

pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;

// Dedicated, 16-byte aligned double-fault stack (4KB).
// Placed statically in .bss so it is permanently mapped and accessible
// even if the active kernel stack overflows or becomes unmapped.
#[repr(align(16))]
struct DoubleFaultStack {
    _storage: [u8; 4096],
}
static mut DOUBLE_FAULT_STACK: DoubleFaultStack = DoubleFaultStack {
    _storage: [0; 4096],
};

/// Encapsulated, sound interior-mutability wrapper for the Task State Segment.
///
/// # Soundness Invariants:
/// 1. Janus is a uniprocessor x86_64 microkernel (`LIMITATIONS.md`). Exactly one CPU core
///    executes kernel code and accesses this TSS.
/// 2. Hardware accesses to the TSS occur via MMU reads during privilege-level transitions
///    (Ring 3 -> 0 reading RSP0) and IST lookups.
/// 3. Software access is strictly limited to single-threaded boot initialization (`init`)
///    and atomic 64-bit writes of `privilege_stack_table[0]` during task switches (`set_kernel_stack`).
/// 4. No shared or mutable Rust references (`&TaskStateSegment` or `&mut TaskStateSegment`)
///    persist past initialization. All runtime updates write directly through the raw pointer
///    obtained from `UnsafeCell::get()`, completely avoiding mutable transmutes of immutable borrows.
#[repr(transparent)]
struct KernelTss(UnsafeCell<TaskStateSegment>);

unsafe impl Sync for KernelTss {}

static TSS: KernelTss = KernelTss(UnsafeCell::new(TaskStateSegment::new()));
static GDT: Once<GlobalDescriptorTable> = Once::new();

#[derive(Clone, Copy)]
pub struct Selectors {
    pub kernel_code: SegmentSelector,
    pub kernel_data: SegmentSelector,
    pub user_data: SegmentSelector,
    pub user_code: SegmentSelector,
    pub tss: SegmentSelector,
}

pub static SELECTORS: Once<Selectors> = Once::new();

/// Initializes the Global Descriptor Table (GDT) and Task State Segment (TSS).
///
/// # Safety
/// Must be called during single-threaded kernel boot before ring transitions or syscall setup.
pub unsafe fn init() {
    let tss_ptr = TSS.0.get();

    // Set up a basic TSS (useful later for interrupts and syscall ring transitions)
    // For syscalls, the CPU needs RSP0 from the TSS when transitioning Ring 3 -> Ring 0.
    (*tss_ptr).privilege_stack_table[0] = VirtAddr::new(0); // We'll update this on task switch

    // Configure the dedicated Double Fault stack in the IST
    let stack_top = VirtAddr::from_ptr(
        core::ptr::addr_of!(DOUBLE_FAULT_STACK)
            .cast::<u8>()
            .add(4096),
    );
    (*tss_ptr).interrupt_stack_table[DOUBLE_FAULT_IST_INDEX as usize] = stack_top;

    let mut gdt = GlobalDescriptorTable::new();

    // We MUST define them in this exact order for SYSRET/SYSCALL to work correctly!
    let kernel_code = gdt.append(Descriptor::kernel_code_segment());
    let kernel_data = gdt.append(Descriptor::kernel_data_segment());
    let user_data = gdt.append(Descriptor::user_data_segment());
    let user_code = gdt.append(Descriptor::user_code_segment());

    // Descriptor::tss_segment extracts the pointer base and limit to encode
    // into the 16-byte system segment descriptor in the GDT.
    let tss_ref: &'static TaskStateSegment = &*tss_ptr;
    let tss_selector = gdt.append(Descriptor::tss_segment(tss_ref));

    let gdt_ref = GDT.call_once(|| gdt);
    gdt_ref.load();

    use x86_64::instructions::tables::load_tss;
    use x86_64::registers::segmentation::{Segment, CS, DS, ES, FS, GS, SS};

    CS::set_reg(kernel_code);
    SS::set_reg(kernel_data);

    // We don't strictly need these in 64-bit mode, but it's good practice.
    DS::set_reg(kernel_data);
    ES::set_reg(kernel_data);
    FS::set_reg(kernel_data);
    GS::set_reg(kernel_data);

    load_tss(tss_selector);

    SELECTORS.call_once(|| Selectors {
        kernel_code,
        kernel_data,
        user_data,
        user_code,
        tss: tss_selector,
    });
}

/// Updates the RSP0 field in the TSS.
/// This must be called during a context switch so that if a user task is interrupted
/// or makes a syscall, the CPU knows where to find the kernel stack for that task.
///
/// # Safety
/// `rsp0` must point to a valid, aligned kernel stack boundary.
pub unsafe fn set_kernel_stack(rsp0: u64) {
    let tss_ptr = TSS.0.get();
    // In x86_64 uniprocessor, this is an atomic 64-bit aligned store.
    // We write directly through raw pointer with no long-lived borrows.
    (*tss_ptr).privilege_stack_table[0] = VirtAddr::new(rsp0);
}
