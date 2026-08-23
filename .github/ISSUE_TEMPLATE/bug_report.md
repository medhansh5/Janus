name: Bug Report
description: Report an issue or unexpected behavior in Janus kernel, bootloader, or drivers.
title: "[BUG]: "
labels: ["bug", "triage"]
assignees: []
body:
  - type: markdown
    attributes:
      value: |
        Thank you for reporting an issue in Janus OS! Please provide as much detailed context as possible.
  - type: textarea
    id: summary
    attributes:
      label: Bug Summary
      description: A concise description of the unexpected behavior.
      placeholder: E.g., Virtio-Blk driver triggers page fault during IRQ acknowledgment.
    validations:
      required: true
  - type: dropdown
    id: environment
    attributes:
      label: Execution Environment
      options:
        - QEMU x86_64 (OVMF UEFI)
        - Bare-Metal x86_64 Hardware
        - Other Hypervisor (KVM / Xen / VMware)
    validations:
      required: true
  - type: textarea
    id: steps
    attributes:
      label: Steps to Reproduce
      description: Exact commands or instructions to reproduce the issue.
      placeholder: |
        1. Run `make run`
        2. Wait for IPC stress test task execution
        3. Trigger interrupt scenario...
    validations:
      required: true
  - type: textarea
    id: expected
    attributes:
      label: Expected Behavior
      description: What should happen instead.
    validations:
      required: true
  - type: textarea
    id: serial_log
    attributes:
      label: Serial Console / Kernel Log
      description: Paste relevant serial output, QEMU console output, or panic messages.
      render: text
  - type: textarea
    id: context
    attributes:
      label: Additional Context
      description: Host OS, Rust toolchain version (`rustc --version`), QEMU version.
