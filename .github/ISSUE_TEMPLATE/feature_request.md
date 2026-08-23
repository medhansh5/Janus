name: Feature Request
description: Propose a new feature, syscall capability, or userland service for Janus OS.
title: "[FEATURE]: "
labels: ["enhancement"]
assignees: []
body:
  - type: textarea
    id: problem
    attributes:
      label: Problem Statement
      description: Is your feature request related to a problem or limitation?
      placeholder: E.g., The kernel currently only supports single-core execution...
    validations:
      required: true
  - type: textarea
    id: proposed_solution
    attributes:
      label: Proposed Solution
      description: Describe the feature, capability type, or component design.
    validations:
      required: true
  - type: textarea
    id: security_impact
    attributes:
      label: Capability & Security Model Impact
      description: Does this feature affect the microkernel TCB, capability table, or Ring 3 isolation boundary?
    validations:
      required: true
  - type: textarea
    id: alternatives
    attributes:
      label: Alternatives Considered
      description: Any alternative designs or implementations considered.
