use x86_64::structures::tss::TaskStateSegment;
use x86_64::structures::gdt::{GlobalDescriptorTable, Descriptor, SegmentSelector};
use x86_64::VirtAddr;

use spin::Once;

pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;

static TSS: Once<TaskStateSegment> = Once::new();
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

pub unsafe fn init() {
    let mut tss = TaskStateSegment::new();
    // Set up a basic TSS (useful later for interrupts and syscall ring transitions)
    // For syscalls, the CPU needs RSP0 from the TSS when transitioning Ring 3 -> Ring 0.
    tss.privilege_stack_table[0] = VirtAddr::new(0); // We'll update this on task switch

    let mut gdt = GlobalDescriptorTable::new();
    
    // We MUST define them in this exact order for SYSRET/SYSCALL to work correctly!
    let kernel_code = gdt.append(Descriptor::kernel_code_segment());
    let kernel_data = gdt.append(Descriptor::kernel_data_segment());
    let user_data = gdt.append(Descriptor::user_data_segment());
    let user_code = gdt.append(Descriptor::user_code_segment());
    
    let tss_ref = TSS.call_once(|| tss);
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
pub unsafe fn set_kernel_stack(rsp0: u64) {
    if let Some(tss) = TSS.get() {
        // Safe because the TSS is only read/written from a single core
        #[allow(mutable_transmutes)]
        let tss_mut: &mut TaskStateSegment = core::mem::transmute(tss);
        tss_mut.privilege_stack_table[0] = VirtAddr::new(rsp0);
    }
}
