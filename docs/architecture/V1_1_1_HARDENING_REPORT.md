# Janus OS — V1.1.1 Hardening Milestone Report

**Repository:** `medhansh5/Janus` (`C:/Users/medha/.gemini/antigravity/scratch/Janus`)  
**Branch:** `main`  
**Baseline HEAD:** `f3130f9a19ab9efc2cc3e5b9a37ca76f36221b6a`  
**Date:** 2026-11-27  

---

## 1. Starting Git State & Baseline

- **Branch:** `main`
- **HEAD:** `f3130f9a19ab9efc2cc3e5b9a37ca76f36221b6a` (unchanged throughout the milestone)
- **Git Rules Compliance:** All changes remain strictly local and unstaged. No commits, staging, resets, branches, PRs, merges, or remote operations were performed. Existing V1.1 audit reports and test artifacts were preserved untouched.

---

## 2. Files Changed & Reasons

| File | Reason |
|:---|:---|
| `kernel/src/ipc.rs` | Added `ipc::tests::test_syscalls_24_30_capability_negative_and_positive` exercising `handle_syscall` directly across syscalls 24–30. |
| `kernel/src/console.rs` | Guarded hardware port I/O (`inb`/`outb`) with `#[cfg(not(test))]` and added `TEST_CONSOLE_OUT_COUNT` so host unit tests exercise full `sys_read_console` / `sys_write_console` capability & buffer checks without privileged-instruction faults. |
| `kernel/src/fs.rs` | Implemented transactional `write_file` / `delete_file` / `create_file`, explicit `Result<(), ()>` propagation in `flush_block_to_disk` & `sync`, static `.bss` `ROLLBACK_BLOCKS` (preventing 4 KiB kernel stack overflow), `inode.size` clamping in `read_file`, deterministic `TEST_FAIL_ON_WRITE_NUM` fault injection, and 3 new transactional regression tests (total 5 FS tests). |
| `kernel/src/notification.rs` | Added `IRQ_MASKED: [bool; 16]`, `mark_irq_masked`, `mark_irq_unmasked`, `cleanup_irq_on_cap_removal`, fixed `reset_notification_object` / `sys_irq_bind` binding ownership check, and added 3 IRQ lifecycle tests. |
| `kernel/src/capability.rs` | Updated `cleanup_slot` to invoke `notification::cleanup_irq_on_cap_removal` whenever a `CapType::Irq` capability is removed. |
| `kernel/src/idt.rs` | Recorded `notification::mark_irq_masked(irq_num)` when masking hardware IRQs in vectors 33..=47. |
| `docs/ARCHITECTURE.md` | Updated syscall register ABI (`rdi`/`rsi`/`rdx`/`r10`/`r8`/`r9`) and capability types (`Notification`, `MmioFrame`, `Irq`, `Console`, `Storage`) to match V1.1.1 implementation. |
| `docs/LIMITATIONS.md` | Documented UP-only execution, `syscall_entry` scratch assumptions, legacy VirtIO polling & queue/BAR limits, and JanusFS capacity & lack of power-loss journaling. |
| `README.md` | Updated implemented vs. planned feature matrix for V1.1.1. |
| `docs/architecture/V1_1_1_HARDENING_PLAN.md` | Pre-implementation hardening plan and acceptance criteria. |
| `docs/architecture/V1_1_1_HARDENING_REPORT.md` | This final milestone report. |

---

## 3. Capability Negative-Test Coverage (Syscalls 24–30)

`ipc::tests::test_syscalls_24_30_capability_negative_and_positive` invokes `crate::ipc::handle_syscall(...)` directly for all 7 syscalls (`SYS_READ_CONSOLE=24`, `SYS_WRITE_CONSOLE=25`, `SYS_FS_CREATE=26`, `SYS_FS_READ=27`, `SYS_FS_WRITE=28`, `SYS_FS_DELETE=29`, `SYS_FS_LIST=30`):
- **Tested failure modes:** Non-canonical `CPtr`, empty slot `CPtr`, stale generation `CPtr` (post-delete), wrong capability type (`Storage` vs `Console`), missing `READ` right, missing `WRITE` right, unauthorized task (Task 1 with no Console/Storage caps), and null user pointers with unauthorized caps.
- **Verified invariants:** All denied calls return `!0`, do not modify user output buffers, do not drain the console input queue or increment `TEST_CONSOLE_OUT_COUNT`, and do not mutate JanusFS. Positive controls for Task 0 succeed for all 7 syscalls.
- **Remaining test gap:** Host unit tests use `copy_user_slice_*` in `#[cfg(test)]` (pointer range check + copy) rather than x86_64 page-table walks (`paging::copy_from_user` / `copy_to_user`), which are tested separately in `paging::tests` and at runtime in QEMU.

---

## 4. Filesystem Transactional Guarantees & Limitations

- **Guarantees:**
  - `write_file` validates `name` (`1..=16` bytes) and `data.len() <= MAX_FILE_SIZE` (2,560 bytes) and pre-allocates all `needed_blocks` before mutating `INODES` or `BLOCK_CACHE`.
  - Disk-full errors during creation or overwrite return `Err(())` with zero partial block leaks and full preservation of prior file contents and metadata.
  - Synchronous VirtIO write errors (`flush_block_to_disk` returning `Err(())`) roll back `INODES[inode_idx]` and modified `BLOCK_CACHE` blocks using `.bss` `ROLLBACK_BLOCKS`.
  - Shrinking overwrites release trailing blocks after metadata commit.
  - `read_file` clamps `inode.size` to `MAX_FILE_SIZE`.
- **Explicit Limitation:** Because JanusFS uses in-place sector writes without a write-ahead journal or copy-on-write, **true power-loss atomicity** across multi-sector flushes (data + inode + superblock) is **not** guaranteed.
- **Note on Self-Review Fix:** Initial QEMU testing caught a `#PF` (`RIP: 0x0`) caused by allocating `saved_blocks` (2,560 bytes) on the 4 KiB kernel stack inside `write_file`. Moving `ROLLBACK_BLOCKS` to static `.bss` eliminated the stack overflow and was verified in QEMU.

---

## 5. IRQ Ownership Design & Cleanup Behavior

- `capability::cleanup_slot` (invoked by `delete_cap`, `revoke_cap`, and `destroy_task_cspace`) captures `old_cap_type` and `old_payload_a` before zeroing the slot and calls `notification::cleanup_irq_on_cap_removal(irq)`.
- `cleanup_irq_on_cap_removal` scans all `MAX_TASKS` CNodes; only when **zero** remaining `CapType::Irq` capabilities for `irq` exist does it clear `IRQ_BINDINGS[irq]` and unmask the line if `IRQ_MASKED[irq]` was set.
- `reset_notification_object` and `sys_irq_bind` were also hardened so destroying an older notification cannot clear an IRQ binding that was subsequently rebound to a different active notification.
- Existing single-EOI-per-real-IRQ and spurious IRQ7/IRQ15 handling in `idt.rs` and `pic.rs` are preserved unchanged.

---

## 6. Hardware & Architecture Support Matrix

| Subsystem | Supported in V1.1.1 | Not Supported / Future Work |
|:---|:---|:---|
| CPU / Scheduling | Single-core x86_64 (UP), 8259 PIC, 1000 Hz PIT IRQ0, `SFMask` `IF=0` syscall entry | SMP, x2APIC, IPIs, `swapgs` per-CPU stacks, NMI-reentrant syscall entry |
| Storage Transport | Legacy VirtIO 0.9.5 PCI (`0x1AF4:0x1001`, I/O BAR0, queue `<= 256`, synchronous polling with 2M spin timeout) | Modern VirtIO 1.x MMIO/PCI, NVMe, async IRQ11 block completion |
| Filesystem | JanusFS (512B sectors, 64 inodes, 64 cached blocks, 2560B max file, cross-reboot persistence, in-memory & I/O rollback) | Journaling / CoW power-loss atomicity, subdirectories, >2560B files |

---

## 7. Quality Gates, Commands & Test Counts

| Command | Exit Code | Result |
|:---|:---:|:---|
| `cargo +nightly fmt --all -- --check` | `0` | Clean |
| `cargo +nightly test -p kernel --lib` | `0` | **122 passed, 0 failed, 0 ignored** (115 original + 7 new V1.1.1 tests) |
| `cargo +nightly clippy -p kernel --lib --tests -- -D warnings` | `0` | Clean (0 warnings) |
| `cargo +nightly build -p kernel --target x86_64-unknown-none --release "-Zbuild-std=core,compiler_builtins"` | `0` | Built `target/x86_64-unknown-none/release/kernel` |
| `cargo +nightly build -p bootloader --target x86_64-unknown-uefi --release "-Zbuild-std=core,alloc,compiler_builtins"` | `0` | Built `target/x86_64-unknown-uefi/release/bootloader.efi` |

---

## 8. QEMU Validation Results (Isolated Temporary Artifacts)

Executed in `%TEMP%\janus_v111_*` against freshly built `kernel` and `bootloader.efi` with a 1 MiB raw VirtIO disk image (`SHA0=30e14955ebf1352266dc2ff8067e68104607e750abb9d3b36582b8af909fcb58`):
- **Boot 1:** UEFI boot -> Ring 3 shell -> `help`, `ps`, `ls`, `cat /welcome.txt`, `write /v111.txt HardenedV111`, `cat /v111.txt`, `write /temp.txt ToDelete`, `test` (Tests 1–5 PASSED), `exit` (`SHA1=22ab834836ade611439bbf2cb6b9b3b7795bb878eeb4396d910ff57e3168b271`).
- **Boot 2:** Restored from VirtIO disk (`[JanusFS] Existing filesystem restored from VirtIO disk.`), verified `/v111.txt`, deleted `/temp.txt` (`rm /temp.txt`), `exit` (`SHA2=1026f047d62d64a267a94a8edb033c469ce50feecfb54107356320050168e325`).
- **Boot 3:** Restored from VirtIO disk, verified `cat /temp.txt` -> `cat: file not found` and `cat /v111.txt` -> `HardenedV111`, `exit`.

---

## 9. Remaining Known Limitations

1. Uniprocessor-only `USER_RSP_SCRATCH` / `KERNEL_RSP_SCRATCH` (safe under UP + `IF=0`, not SMP-safe, not NMI-reentrant).
2. Synchronous VirtIO polling (`used.idx`) without async IRQ11 completion.
3. No power-loss journaling/CoW in JanusFS.
4. `pci::discover_virtio` assumes legacy I/O BAR0 and panics if VirtIO-Blk is absent; `VirtioBlockDevice::init` assumes `q_size <= 256`.

---

## 10. Final Verdict

**V1.1.1 HARDENING COMPLETE — ALL ACCEPTANCE CRITERIA VERIFIED**
