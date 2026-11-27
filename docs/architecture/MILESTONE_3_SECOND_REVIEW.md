# Janus OS — Milestone 3 Second Independent Architecture & Security Audit
**Author:** Independent Systems & Security Auditor  
**Date:** November 24, 2026  
**Repository:** `medhansh5/Janus`  
**Branch:** `main`  
**Baseline Commit:** `d288cadd17aee34ddac16f95de89ddb16b329f92`  
**Documents Audited:**
- `docs/architecture/MILESTONE_3_DESIGN.md` (v2.0.0 Revised)
- `docs/architecture/MILESTONE_3_REVIEW.md` (v1.0.0 Initial Audit)
- `docs/architecture/MILESTONE_3_REVISION_NOTES.md` (v1.0.0 Revision Summary)
**Verdict:** **APPROVED WITH CONDITIONS FOR STAGE 3.0 IMPLEMENTATION (STRICTLY GATED)**

---

## 1. Executive Summary & Verification Baseline

This document presents a strictly independent, adversarial systems architecture and security audit of the revised **Milestone 3 — Driver Isolation & Capability Delegation Framework** (v2.0.0). The audit directly inspects the integrated codebase at commit `d288cad` against the claims, data structures, state machines, and hardware execution models specified in the revised design.

### 1.1 Repository Preflight
- **Repository Remote:** `https://github.com/medhansh5/Janus.git` (verified `origin`)
- **Current Branch:** `main`
- **HEAD Commit:** `d288cadd17aee34ddac16f95de89ddb16b329f92`
- **Working Tree State:** Pristine; zero tracked modifications; existing architecture documents (`MILESTONE_3_DESIGN.md`, `MILESTONE_3_REVIEW.md`, `MILESTONE_3_REVISION_NOTES.md`) verified present and intact.
- **Audit Mode:** Strictly read-only with respect to existing code and documents; only this review document is created.

### 1.2 Summary of Verdict: Conditional Approval for Stage 3.0 Only
The revised design v2.0.0 makes significant progress by adopting the mandatory maintainer decisions (acknowledging non-IOMMU DMA boundaries, planning embedded ELF validation, and ordering Stage 3.0 as a prerequisite). However, this second audit uncovers **three critical architectural contradictions and several structural defects**:
1. **Critical DMA Proxy Contradiction:** While Section 4.5.1 introduces a "Trusted Kernel Bounce-Buffer DMA Proxy", Sections 3.1, 3.4, and the Threat Matrix still grant the Ring 3 driver an `IOPort` capability to the VirtIO PCI I/O BAR (`0xC000..0xC040`). Because the legacy VirtIO BAR contains the `Queue Address` register (offset `0x08`), an untrusted Ring 3 driver holding this capability can program arbitrary physical DRAM addresses directly into the device and trigger bus-master DMA, completely bypassing the kernel proxy!
2. **CNodeSlot Memory Layout Struct Bug:** The proposed Rust `CNodeSlot` layout contains an alignment bug. Due to natural 8-byte alignment padding between `generation: u16` and `payload_a: u64`, having `_reserved: [u8; 8]` at the end makes the struct **40 bytes**, violating the 32-byte slot budget and expanding `CNode` to **5,120 bytes**, which breaks the 4 KiB frame static assertion.
3. **CI / Testing Reality Gap:** The design claims CI constructs GPT/FAT32 images, runs QEMU boot tests, and performs serial assertions. Inspection of `.github/workflows/ci.yml` reveals that CI only executes `cargo check`, host unit tests, and cross-compilation; **no disk images are built, and QEMU is never executed in CI**.

**Stage 3.0 (Dynamic Virtual Memory Subsystem) is APPROVED WITH CONDITIONS**, as it is an internal kernel prerequisite isolated from driver DMA and capability syscalls. Stages 3.1 through 3.5 remain **BLOCKED** until the architectural contradictions identified herein are formally resolved.

---

## 2. Documents Read & Source Files Inspected

### Architecture & Review Documents
- `docs/architecture/MILESTONE_3_DESIGN.md` (v2.0.0): Read in its entirety (445+ lines).
- `docs/architecture/MILESTONE_3_REVIEW.md` (v1.0.0): Analyzed initial findings and maintainer decisions.
- `docs/architecture/MILESTONE_3_REVISION_NOTES.md` (v1.0.0): Evaluated claims of resolved findings.

### Repository Source Files Inspected
- `kernel/src/memory.rs` (lines 1–203): Inspected `FrameAllocator` implementation; confirmed freelist-based allocation where `allocate_frame()` returns unzeroed memory containing stale pointers.
- `kernel/src/paging.rs` (lines 1–280): Inspected root page table setup (`init_root_page_table`), `PHYS_MEMORY_OFFSET = 0xFFFF_9000_0000_0000`, and `translate_user_page` two-phase walk.
- `kernel/src/task.rs` (lines 1–278): Inspected `Task`, `TaskContext`, and `schedule()`; confirmed CR3 reload is already executed during context switch.
- `kernel/src/main.rs` (lines 230–330): Inspected manual page table construction (`pdpt_paddr`, `pd_paddr`, `pt_paddr`) and confirmed all user tasks share a single global `user_pml4_paddr`.
- `kernel/src/capability.rs` (lines 1–58): Confirmed 128-slot `CNode` and compile-time size assertion `assert!(size_of::<CNode>() <= 4096)`.
- `kernel/src/idt.rs` (lines 240–280): Confirmed IRQ 42..47 handling calls `pic::end_of_interrupt` immediately without masking.
- `kernel/src/pic.rs` (lines 65–97): Inspected `pic::mask(irq)`, `pic::unmask(irq)`, and `pic::end_of_interrupt(irq)`.
- `kernel/src/ipc.rs` (lines 1–100, 370–478): Inspected syscall dispatch, `wake_irq`, and port I/O syscalls.
- `kernel/src/pci.rs` (lines 1–100): Inspected `discover_virtio()` and PCI bus-master configuration.
- `.github/workflows/ci.yml` (lines 1–62): Audited CI jobs for formatting, host tests, and target builds.
- `Makefile` (lines 1–26): Audited QEMU invocation, Linux shell dependencies, and hardcoded OVMF path.

---

## 3. Independent Audit Findings by Severity

```
+--------------------------------------------------------------------------------------------------+
| Severity Breakdown:                                                                              |
|  [CRITICAL] 1. VirtIO I/O Port Delegation Contradicts Trusted DMA Proxy Boundary                 |
|  [HIGH]     2. CNodeSlot Layout Alignment Bug Breaks 32-Byte Budget (40-Byte Actual)             |
|  [HIGH]     3. Unzeroed FrameAllocator Output Threatens Page Table Integrity                     |
|  [HIGH]     4. CI / Testing Claims Conflate Compile Checks with Non-Existent QEMU Runtime Tests |
|  [MEDIUM]   5. Missing Direct-Map Upper-Bound Checks on Physical Page Mapping                    |
|  [MEDIUM]   6. Ambiguous Shared-IRQ and Spurious-IRQ Handling in PIC Dispatch                    |
|  [LOW]      7. Makefile Incompatible with Windows Host Development Environment                   |
+--------------------------------------------------------------------------------------------------+
```

---

## 4. Critical Audit: DMA Proxy & Device Authority (Stage 3.4)

### 4.1 Evaluation of the 10 Specific DMA Architecture Questions

1. **Who owns the VirtIO device registers?**  
   *Design Flaw:* Section 4.5.1 claims the kernel proxy owns the device registers, but Section 3.1 (lines 88–105), Section 3.4 (lines 247–255), and THREAT-01 state that `devmgr` mints an `IOPort` capability covering the VirtIO I/O range (`base: 0xC000, size: 0x40`) and assigns it to the Ring 3 driver.
2. **Who owns and programs the VirtIO queue configuration?**  
   *Contradiction:* Section 4.5.2 claims the kernel proxy allocates VirtQueues, but Section 3.1 claims the driver manages virtqueues and descriptor rings directly via `SYS_IN_PORT` / `SYS_OUT_PORT`.
3. **Who allocates and populates physical DMA descriptors?**  
   *Inconsistency:* Section 4.5.1 specifies the kernel populates descriptors in kernel frames, whereas Section 3.4 specifies the driver requests contiguous memory and passes physical addresses to the device virtqueue.
4. **Can the Ring 3 driver write queue addresses or device registers directly?**  
   **YES, in the current design specification!** If the driver holds an `IOPort` capability covering `0xC000..0xC040`, it can invoke `SYS_OUT_PORT(cap, 0x08, target_paddr, 4)`. Offset `0x08` in a legacy VirtIO PCI BAR is the **Queue Address Register** (the physical page frame number of the descriptor table).
5. **Can the driver enable PCI Bus Mastering?**  
   No direct PCI config space access is granted, but `pci.rs:97` enables Bus Mastering globally during boot in Ring 0. The hardware device remains bus-master enabled permanently.
6. **Can the driver modify descriptors after the kernel validates them?**  
   If descriptors reside in kernel-controlled bounce-buffer memory unmapped from the driver's page table, the driver cannot modify them. However, if the driver holds the `IOPort` capability, it can point the device to a different descriptor table entirely via `Queue Address`!
7. **Can the driver trick the kernel into DMAing into arbitrary physical memory?**  
   If the kernel bounce-buffer proxy is used exclusively, the kernel only programs kernel-allocated bounce buffers. But if the driver has raw port access, it bypasses the kernel entirely.
8. **Who owns bounce buffers, and how are their physical addresses protected?**  
   Bounce buffers must be owned exclusively by the kernel proxy in direct-mapped RAM (`PHYS_MEMORY_OFFSET + paddr`) and never mapped into any user page table.
9. **What happens if a driver submits malicious lengths, offsets, or descriptor chains?**  
   Under a pure kernel proxy, the kernel validates sector count and length against maximum bounded limits (e.g. max 128 sectors = 64 KiB) before copying.
10. **What exact operation proves that DMA has stopped before memory reclamation?**  
    Writing `0` to the VirtIO status register alone is **insufficient**.

### 4.2 The Direct I/O Port Contradiction (CRITICAL VULNERABILITY)
In legacy VirtIO over PCI (standard QEMU `-drive if=virtio`), the I/O BAR layout is standardized:
- `Offset 0x00`: Device Features (32-bit R)
- `Offset 0x04`: Guest Features (32-bit R/W)
- `Offset 0x08`: **Queue Address (32-bit R/W - Physical Page Number of VirtQueue!)**
- `Offset 0x0C`: Queue Size (16-bit R)
- `Offset 0x0E`: Queue Select (16-bit R/W)
- `Offset 0x10`: **Queue Notify (16-bit R/W - Kicks Hardware DMA!)**
- `Offset 0x12`: **Device Status (8-bit R/W)**
- `Offset 0x13`: ISR Status (8-bit R)

```
[VULNERABILITY PATHWAY IN DESIGN V2.0.0]
+--------------------------------------------------------------------------------------+
| Ring 3 Driver                                                                        |
|   Holds IOPort capability for base 0xC000, size 0x40                                 |
|   1. Calls SYS_OUT_PORT(cptr, offset=0x08, val=0x00001, width=4)                     |
|      --> Points VirtQueue Physical Page to 0x1000 (Kernel PML4 Frame!)               |
|   2. Calls SYS_OUT_PORT(cptr, offset=0x10, val=0, width=2)                           |
|      --> Kicks Queue Notify                                                          |
+--------------------------------------------------------------------------------------+
                                   |
                                   v (Allowed by IOPort capability check in ipc.rs!)
+--------------------------------------------------------------------------------------+
| VirtIO Hardware Device (PCI Bus Master)                                              |
|   Directly DMAs incoming disk data into physical address 0x1000                      |
|   KERNEL ROOT PAGE TABLE OVERWRITTEN. COMPLETE RING 0 TAKEOVER.                      |
+--------------------------------------------------------------------------------------+
```

#### Mandatory Architectural Correction for Stage 3.4
The Ring 3 driver must **NEVER receive an `IOPort` capability to the VirtIO device BAR**.
- All VirtIO registers (ports `0xCF8/0xCFC` for PCI config and the entire BAR range `0xC000..0xC040`) must be owned **exclusively by the kernel**.
- The Ring 3 driver communicates with the block subsystem purely via a high-level block request IPC protocol (`SYS_CALL(driver_endpoint, BLOCK_READ, lba, count, grant_frame)`).
- Sections 3.1, 3.4, and THREAT-01 in `MILESTONE_3_DESIGN.md` must be corrected to purge all references to driver-held `IOPort` capabilities for VirtIO.

### 4.3 Hardware Quiescence & PCI Read-After-Write Flush
To guarantee all in-flight DMA has terminated before memory reclamation:
1. Clear Bit 2 (`Bus Master Enable`) in the PCI Command Register via `pci::write_config`.
2. **PCI Read-After-Write Flush:** Perform `pci::read_config` on the Command Register. PCI configuration writes are posted cycles; reading back forces the host bridge to flush the write transaction to the PCI bus.
3. Write `0` to the VirtIO Device Status register.
4. Read back the Device Status register in a loop until it returns `0` (with a bounded timeout).
5. If the readback does not return `0` within the timeout, the device is considered wedged. The kernel must trigger a fail-stop halt and **never recycle the memory frames**.

---

## 5. Critical Audit: Stage 3.0 Dynamic Virtual Memory

### 5.1 Existing Allocator Realities (`kernel/src/memory.rs`)
1. **Allocator Type:** The concrete type in `memory.rs` is `pub struct FrameAllocator`. It is aliased in `paging.rs` as `PhysFrameAllocator`. It is a freelist allocator, **not a bitmap allocator**.
2. **Unzeroed Memory Invariant (HIGH HAZARD):**
   - Lines 61–72 of `memory.rs`:
     ```rust
     pub fn allocate_frame(&mut self) -> Option<u64> {
         if let Some(addr) = self.head {
             unsafe {
                 let ptr = (addr + self.phys_offset) as *const Option<u64>;
                 self.head = core::ptr::read(ptr);
             }
             self.total_frames -= 1;
             Some(addr)
         } else { None }
     }
     ```
   - When a frame is allocated, the first 8 bytes contain the previous freelist `head` pointer. The remaining 4,088 bytes contain stale heap data or previous memory contents.
   - **Critical Requirement:** Every function in Stage 3.0 that allocates a page-table frame (`create_user_pml4`, `map_user_page`) **MUST explicitly zero all 4,096 bytes via direct-map before reading or linking it**:
     ```rust
     unsafe {
         core::ptr::write_bytes((paddr + PHYS_MEMORY_OFFSET) as *mut u8, 0, 4096);
     }
     ```
   - If an unzeroed frame is linked into a page table, random non-zero bits will be decoded by the CPU MMU as `PRESENT` page table entries pointing to arbitrary memory!

3. **Direct-Map Upper-Bound Checks:**
   - Physical memory direct-map is initialized in `paging.rs:48` up to 4 GiB (`2048 * 2MB`).
   - If `allocate_frame()` returns an address $\ge \text{4 GiB}$ (`0x1_0000_0000`), accessing it via `paddr + PHYS_MEMORY_OFFSET` triggers an immediate kernel page fault.
   - `map_user_page` must validate: `paddr < 0x1_0000_0000` and `(paddr & 0xFFF) == 0`.

### 5.2 Dynamic Mapping Contracts & Explicit Invariants

#### Function 1: `create_user_pml4`
```rust
pub fn create_user_pml4(allocator: &mut PhysFrameAllocator) -> Result<u64, PagingError>
```
- **Invariants:**
  1. Allocates exactly one 4 KiB physical frame `pml4_paddr`.
  2. Zeroes PML4 entries `0..255` (user space is completely unmapped).
  3. Shallow-copies PML4 entries `256..511` directly from the active kernel root PML4.
  4. Returns `Ok(pml4_paddr)`. On allocation failure, returns `Err(PagingError::OutOfMemory)`.

#### Function 2: `map_user_page`
```rust
pub fn map_user_page(
    pml4_paddr: u64,
    vaddr: u64,
    paddr: u64,
    flags: PageTableFlags,
    allocator: &mut PhysFrameAllocator,
) -> Result<(), PagingError>
```
- **Invariants:**
  1. **Canonical & Bounds Check:** `vaddr < 0x0000_8000_0000_0000` and `(vaddr & 0xFFF) == 0`. Target PML4 index must be $< 256$.
  2. **Physical Bounds Check:** `paddr < 0x1_0000_0000` and `(paddr & 0xFFF) == 0`.
  3. **W^X Enforcement:** If `flags` contains `WRITABLE`, it **must** also contain `NO_EXECUTE`. Any violation returns `Err(PagingError::WxViolation)`.
  4. **Anti-Aliasing Check:** If the target leaf PT entry is already `PRESENT`, the function must **abort and return `Err(PagingError::AlreadyMapped)`**. Overwriting an existing mapping without unmapping leaks the underlying frame and causes aliasing bugs.
  5. **Intermediate Table Creation:**
     - Walks PML4 $\to$ PDPT $\to$ PD $\to$ PT.
     - Any missing intermediate table frame is allocated, zeroed completely, and linked with flags:
       `PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE`.
  6. **Atomic Rollback on Allocation Failure:** If allocating PDPT, PD, or PT fails midway, any intermediate frames allocated *during this specific function call* must be immediately freed back to `allocator`, clearing the parent entry before returning `Err(PagingError::OutOfMemory)`.
  7. **TLB Invalidation:** If `pml4_paddr == active_cr3`, issue `invlpg [vaddr]`. If not active, TLB invalidation is deferred to CR3 reload during context switch.

#### Function 3: `unmap_user_page`
```rust
pub fn unmap_user_page(
    pml4_paddr: u64,
    vaddr: u64,
    allocator: &mut PhysFrameAllocator,
) -> Result<u64, PagingError>
```
- **Invariants:**
  1. Verifies `vaddr < 0x0000_8000_0000_0000` and `(vaddr & 0xFFF) == 0`.
  2. Walks page table to leaf PT entry. If any intermediate table or the leaf PT entry is not `PRESENT`, returns `Err(PagingError::NotMapped)`.
  3. Extracts mapped physical address `paddr = pt_entry.addr().as_u64()`.
  4. Sets `pt_entry = PageTableEntry::new()` (cleared).
  5. If `pml4_paddr == active_cr3`, issues `invlpg [vaddr]`.
  6. Returns `Ok(paddr)`. (Intermediate table cleanup is deferred to address space teardown in Stage 3.0 MVP).

#### Function 4: `destroy_user_pml4`
```rust
pub fn destroy_user_pml4(pml4_paddr: u64, allocator: &mut PhysFrameAllocator)
```
- **Invariants:**
  1. **Strict Higher-Half Protection:** Iterates strictly over PML4 entries `0..255`. Entries `256..511` must **NEVER be read, traversed, cleared, or freed**.
  2. For every present user PML4 entry `i in 0..256`:
     - Walks PDPT entries `0..512`. For every present PDPT entry, walks PD entries `0..512`.
     - For every present PD entry, walks PT entries `0..512`. For every present leaf PT entry, frees the user physical frame.
     - Frees the PT frame.
     - Frees the PD frame.
     - Frees the PDPT frame.
  3. Finally, frees the root PML4 frame `pml4_paddr`.

---

## 6. Critical Audit: Capability Slots & CDT

### 6.1 The 40-Byte Struct Alignment Bug (HIGH DEFECT)
`MILESTONE_3_DESIGN.md` line 494 defines:
```rust
#[repr(C)]
pub struct CNodeSlot {
    pub cap_type: CapType,         // 1 byte  (offset 0)
    pub rights: CapRights,         // 1 byte  (offset 1)
    pub generation: u16,           // 2 bytes (offset 2..4)
    pub payload_a: u64,            // 8 bytes (needs 8-byte alignment!)
    pub payload_b: u64,            // 8 bytes (offset 16..24)
    pub cdt_index: u32,            // 4 bytes (offset 24..28)
    pub _reserved: [u8; 8],        // 8 bytes (offset 28..36)
}
```
**Layout Calculation:**
- Offset 0: `cap_type` (1 byte)
- Offset 1: `rights` (1 byte)
- Offset 2..4: `generation` (2 bytes)
- **Offset 4..8: Compiler inserts 4 bytes of padding** because `payload_a` is a `u64` requiring 8-byte alignment!
- Offset 8..16: `payload_a` (8 bytes)
- Offset 16..24: `payload_b` (8 bytes)
- Offset 24..28: `cdt_index` (4 bytes)
- Offset 28..36: `_reserved: [u8; 8]` (8 bytes)
- Offset 36..40: Compiler rounds struct size up to multiple of alignment (8 bytes) $\to$ **Size = 40 bytes!**
- With 128 slots: $128 \times 40 = \mathbf{5,120 \text{ bytes}} > 4,096 \text{ bytes}$!
- **The compile-time static assertion `assert!(size_of::<CNode>() <= 4096)` will FAIL!**

#### Concrete Rust Correction
To achieve exactly 32 bytes, `_reserved` must be `[u8; 4]`:
```rust
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CNodeSlot {
    pub cap_type: CapType,         // 1 byte  (offset 0)
    pub rights: CapRights,         // 1 byte  (offset 1)
    pub generation: u16,           // 2 bytes (offset 2..4)
    // 4 bytes natural alignment padding inserted here by compiler (offset 4..8)
    pub payload_a: u64,            // 8 bytes (offset 8..16)
    pub payload_b: u64,            // 8 bytes (offset 16..24)
    pub cdt_index: u32,            // 4 bytes (offset 24..28)
    pub _reserved: [u8; 4],        // 4 bytes (offset 28..32)
}

const _: () = assert!(core::mem::size_of::<CNodeSlot>() == 32);
const _: () = assert!(core::mem::size_of::<CNode>() == 4096);
```

### 6.2 Deterministic CDT Indexing (Eliminating CDT Exhaustion)
In Janus, `MAX_TASKS = 16` and `CNODE_SLOTS = 128`.
- The maximum number of CNode slots that can exist in the system across all tasks is:
  $$\text{Total Slots} = 16 \times 128 = 2,048$$
- **Architectural Solution:** Rather than allocating CDT nodes dynamically or maintaining a dynamic free list, the CDT should be a static array of 2,048 entries indexed directly by `(task_id << 7) | slot_index`.
- Every CNode slot maps directly to a deterministic CDT entry. This completely eliminates CDT exhaustion, allocation failures, and heap fragmentation!

---

## 7. Critical Audit: Hardware-Aware Interrupt Architecture

### 7.1 PIC Line Masking & Race Conditions
- In `kernel/src/idt.rs:266`, the current code calls `pic::end_of_interrupt(irq_num)` immediately.
- For level-triggered PCI interrupts, the hardware line remains pulled LOW. If EOI is sent without masking, the PIC immediately raises another interrupt to the CPU upon `iretq`, causing an infinite livelock.
- **Audit Verification:** The proposed fix—masking the line via `pic::mask(irq)` in the kernel IDT stub before sending `pic::end_of_interrupt(irq)`—is structurally sound and verified against the existing `pic.rs` interface.

### 7.2 Missing Interrupt Edge Cases
1. **Driver Crash with Masked IRQ:** If a driver crashes while its IRQ is masked, the line remains masked forever. The fault recovery handler in Stage 3.5 must explicitly disassociate the IRQ and invoke `pic::unmask(irq)` after resetting the device.
2. **Shared IRQs:** Legacy PCI allows multiple devices to share an IRQ line. Under the proposed model, masking the line blocks all devices on that line until the driver acknowledges. Because Milestone 3 supports only a single storage device on IRQ 11, shared IRQs can be deferred to Milestone 4, but must be explicitly documented as unsupported.
3. **Spurious IRQs:** 8259 PIC delivers IRQ 7 (Master) or IRQ 15 (Slave) on spurious electrical glitches. `idt.rs` currently lacks spurious IRQ filtering; reading the PIC In-Service Register (ISR) on IRQ 7/15 should be added in Stage 3.2.

---

## 8. Critical Audit: Embedded ELF Loader

### 8.1 Validation Pipeline Invariants
The proposed embedded ELF loader must enforce:
1. **Header Validation:** `e_ident[0..4] == [0x7F, b'E', b'L', b'F']`, `e_ident[EI_CLASS] == 2` (64-bit), `e_ident[EI_DATA] == 1` (little-endian), `e_type == 2` (`ET_EXEC`), `e_machine == 0x3E` (`EM_X86_64`).
2. **Program Header Bounds:**
   - For every `p_type == PT_LOAD`:
     - `p_offset.checked_add(p_filesz)` must be $\le \text{blob.len()}$.
     - `p_memsz >= p_filesz`.
     - `p_vaddr.checked_add(p_memsz)` must be $< \text{0x0000\_8000\_0000\_0000}$ (strictly user space).
     - `p_vaddr` must be 4 KiB aligned (`p_vaddr & 0xFFF == 0`).
3. **BSS Zeroing:** For pages covering the delta `(p_memsz - p_filesz)`, the kernel must zero memory up to the page boundary.
4. **W^X Enforcement:** No segment may have both `PF_W` and `PF_X`.
5. **Entry Point Bounds:** `e_entry` must fall strictly within an executable `PT_LOAD` segment.
6. **User Stack Allocation:** The loader must explicitly allocate and map user stack frames (e.g. at `0x0000_7FFF_FFFF_0000`) with `PRESENT | WRITABLE | USER_ACCESSIBLE | NO_EXECUTE`.

---

## 9. Verified Testing & Infrastructure Reality

### 9.1 CI Workflow Audit (`.github/workflows/ci.yml`)
Inspection of `.github/workflows/ci.yml` reveals:
- **Job 1 (`rust`):** `cargo fmt --check`, `cargo clippy -p boot-info`, `cargo clippy -p kernel --lib`.
- **Job 2 (`build-and-test`):** `cargo test -p boot-info`, `cargo test -p kernel --lib`, and target builds for `kernel` (`x86_64-unknown-none`) and `bootloader` (`x86_64-unknown-uefi`).
- **REALITY CHECK:**
  - **CI DOES NOT BUILD DISK IMAGES.**
  - **CI DOES NOT RUN QEMU.**
  - **CI DOES NOT PERFORM SERIAL LOG ASSERTIONS.**
  - All claims in `MILESTONE_3_DESIGN.md` stating that CI runs automated QEMU boot tests are **factually incorrect**. CI performs only static analysis, host unit tests, and target cross-compilations.

### 9.2 Local Development Environment Status
- **Host OS:** Windows 10/11.
- **Rust Toolchain:** `rustc 1.100.0` (MSVC stable default). Nightly Rust and bare-metal targets are not installed locally.
- **QEMU:** Installed at `C:\Program Files\qemu\qemu-system-x86_64.exe` (v11.1.0).
- **OVMF Firmware:** Verified present at `C:\Program Files\qemu\share\edk2-x86_64-code.fd`.
- **Makefile Limitations:** `Makefile:4` hardcodes `OVMF = /usr/share/ovmf/OVMF.fd` (Linux path) and uses Linux utilities (`mkdir -p`, `cp`, `dd`, `printf`), making `make run` unusable directly in standard Windows PowerShell without WSL or MSYS2.

---

## 10. Corrected Stage 3.0 MVP Specification

Stage 3.0 is an internal, foundational prerequisite. It does not touch drivers, syscalls, or hardware devices.

### 10.1 Allowed File Modifications
- **Permitted Files:** `kernel/src/paging.rs` and an internal unit-test module in `paging.rs`.
- **Prohibited Files:** Zero modifications to `main.rs`, `syscall.rs`, `ipc.rs`, `task.rs`, `capability.rs`, `Cargo.toml`, or CI workflows.

### 10.2 Required Type & Function Signatures
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PagingError {
    OutOfMemory,
    KernelAddressViolation,
    NonCanonicalAddress,
    MisalignedAddress,
    PhysicalAddressOutOfBounds,
    AlreadyMapped,
    NotMapped,
    WxViolation,
}

pub fn create_user_pml4(allocator: &mut PhysFrameAllocator) -> Result<u64, PagingError>;

pub fn map_user_page(
    pml4_paddr: u64,
    vaddr: u64,
    paddr: u64,
    flags: PageTableFlags,
    allocator: &mut PhysFrameAllocator,
) -> Result<(), PagingError>;

pub fn unmap_user_page(
    pml4_paddr: u64,
    vaddr: u64,
    allocator: &mut PhysFrameAllocator,
) -> Result<u64, PagingError>;

pub fn destroy_user_pml4(pml4_paddr: u64, allocator: &mut PhysFrameAllocator);
```

### 10.3 Mandatory Unit Test Suite for Stage 3.0
1. `test_create_user_pml4_copies_kernel_higher_half`: Verifies entries 0..255 are zero and 256..511 match kernel root.
2. `test_map_user_page_creates_intermediate_tables`: Verifies dynamic allocation of PDPT, PD, and PT frames.
3. `test_map_user_page_zeroes_frames`: Verifies newly allocated table frames are completely zeroed before linking.
4. `test_map_user_page_rejects_wx_violation`: Verifies `flags` with both `WRITABLE` and executable is rejected.
5. `test_map_user_page_rejects_kernel_address`: Verifies addresses $\ge \text{0x0000\_8000\_0000\_0000}$ are rejected.
6. `test_map_user_page_rejects_already_mapped`: Verifies mapping over an existing page returns `AlreadyMapped`.
7. `test_map_user_page_atomic_rollback`: Mock allocator exhaustion midway unwinds all allocated intermediate tables.
8. `test_unmap_user_page_clears_pt_entry`: Unmapping clears entry and returns original physical address.
9. `test_destroy_user_pml4_preserves_kernel_entries`: Teardown leaves higher-half entries completely untouched.

---

## 11. Explicit Approval Gate Verdict

# **APPROVED WITH CONDITIONS FOR STAGE 3.0 IMPLEMENTATION**

### Mandatory Conditions for Stage 3.0 Execution:
1. **Condition 1 (Frame Zeroing):** `create_user_pml4` and `map_user_page` must explicitly zero all 4,096 bytes of every newly allocated page-table frame via `PHYS_MEMORY_OFFSET + paddr` before reading or linking it.
2. **Condition 2 (Direct-Map Bounds):** `map_user_page` must reject any physical address `paddr >= DIRECT_MAP_LIMIT` (4 GiB) or misaligned `paddr`.
3. **Condition 3 (Atomic Rollback):** If frame allocation fails while building intermediate page tables, all intermediate tables allocated during that specific call must be freed back to `PhysFrameAllocator` before returning `Err(PagingError::OutOfMemory)`.
4. **Condition 4 (Anti-Aliasing):** `map_user_page` must return `Err(PagingError::AlreadyMapped)` if the target virtual address is already mapped.
5. **Condition 5 (Teardown Boundary):** `destroy_user_pml4` must iterate strictly across PML4 entries `0..255`. Entries `256..511` must never be traversed, modified, or freed.
6. **Condition 6 (Strict Stage Isolation):** Stage 3.0 must modify ONLY `kernel/src/paging.rs`. No syscalls, no CNode modifications, and no driver code may be introduced.
7. **Condition 7 (Stage 3.1–3.5 Gate):** Stages 3.1 through 3.5 remain **BLOCKED** until `MILESTONE_3_DESIGN.md` is amended to eliminate the VirtIO `IOPort` delegation contradiction and fix the 40-byte `CNodeSlot` layout bug.
