name: Architecture RFC / Proposal
description: Propose significant architectural changes to Janus microkernel design, IPC, or capability management.
title: "[RFC]: "
labels: ["architecture", "discussion"]
assignees: []
body:
  - type: textarea
    id: summary
    attributes:
      label: Summary & Goal
      description: Brief explanation of the proposed architectural change.
    validations:
      required: true
  - type: textarea
    id: motivation
    attributes:
      label: Motivation & Problem Context
      description: Why is this change necessary? What design constraint does it address?
    validations:
      required: true
  - type: textarea
    id: detailed_design
    attributes:
      label: Detailed Design
      description: Detail the proposed changes to the microkernel, bootloader, userspace services, or IPC ABI.
    validations:
      required: true
  - type: textarea
    id: tcb_and_threat_model
    attributes:
      label: TCB & Threat Model Implications
      description: How does this proposal impact the Trusted Computing Base and threat boundaries?
    validations:
      required: true
