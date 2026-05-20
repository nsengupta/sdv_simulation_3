# Episode 3: Hardening the CAN Boundary with ACK/NACK and Timeout Recovery

## Preface

Episode 2 delivered a functioning runtime and actuation path.  
This episode addresses what matters in production-like conditions: response validation, failure semantics, and predictable recovery behavior.

## Motivation

On real buses, response paths are not always clean:

- responses can be delayed or absent,
- ACK and NACK must be interpreted correctly,
- stale or mismatched responses must be ignored safely.

Without clear policy and recovery rules, control loops become brittle.

## Scope

This episode covers:

- transport-agnostic payload codec boundaries,
- correlation/direction validation policy,
- timeout-driven domain recovery paths,
- bus-level tests over `vcan0`.

## Approach

The hardening pattern is:

1. Decode wire payloads into typed actuation payload objects.
2. Validate response eligibility against pending command state.
3. Accept only correlated/direction-matching responses; ignore the rest.
4. Emit domain events for accepted responses.
5. Recover deterministically on no-response timeout.

## Arch Diagram

Primary diagram for this episode:

- `blog-inputs/diagrams/04-actuation-reliability-sequence.mmd`

Quick render option:

```bash
mmdc -i blog-inputs/diagrams/04-actuation-reliability-sequence.mmd -o blog-inputs/diagrams/04-actuation-reliability-sequence.svg
```

## Key Design Aspects

### 1) Transport codec and semantic mapping are cleanly separated

Wire encode/decode stays transport-level; semantic mapping to physical vocabulary is explicit.

Suggested snippet source:

- `crates/gateway/src/devices/front_headlamp/codec.rs`

### 2) Policy gates ingress using pending command state

Policy validates:

- pending command existence,
- correlation (`session`, `sequence`) match,
- command direction match.

Only valid responses flow inward.

Suggested snippet source:

- `crates/gateway/src/devices/front_headlamp/policy.rs`

### 3) Command frames are not treated as ingress confirmations

The boundary distinguishes command kinds from response kinds and prevents accidental self-acknowledgement paths.

Suggested snippet source:

- `crates/gateway/src/devices/front_headlamp/codec.rs`
- `crates/gateway/tests/front_headlamp_bus_e2e.rs`

### 4) Timeout recovery is domain-owned and deterministic

When no ACK arrives within configured wait windows, domain logic reverts to safe lighting state and emits warnings.

Suggested snippet source:

- `crates/common/src/fsm/step.rs` (`try_front_headlamp_ack_timeout`, recovery logic)

### 5) Bus-level tests verify real path behavior

Integration tests over `vcan0` cover:

- ACK acceptance,
- NACK handling,
- command-frame non-ingress behavior,
- no-response windows.

Suggested snippet source:

- `crates/gateway/tests/front_headlamp_bus_e2e.rs`

## Output Screenshots / Clips

Recommended artifacts for this episode:

- log screenshot: accepted ACK with session/sequence,
- log screenshot: NACK path and recovery event,
- log screenshot: timeout warning in no-response scenario,
- optional short clip running fault-injection env vars during gateway execution.

## Series Wrap-Up

Across 3 episodes, we built a coherent path:

- deterministic domain core,
- disciplined runtime wiring,
- hardened bus boundary with explicit recovery semantics.

This creates a stable base for the next phase (for example: richer actuation devices, child actor offloading, or production observability sinks).
