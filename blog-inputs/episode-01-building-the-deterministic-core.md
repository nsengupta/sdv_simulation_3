# Episode 1: Building the Deterministic Core for an SDV Twin

## Preface

This series documents the engineering journey of a software-defined vehicle (SDV) simulation stack, starting from a strict deterministic core and moving toward hardened gateway and bus behavior.

Episode 1 focuses on the foundational layer: the part that must remain predictable under changing runtime, transport, and device integrations.

## Motivation

Vehicle behavior should not depend on incidental runtime details like thread scheduling, connector latency, or transport quirks. We wanted a core that is:

- deterministic and replay-friendly,
- easy to verify with contract tests,
- explicitly separated from physical I/O and protocol concerns.

That leads to one central design principle: **domain logic should emit intent, not perform side effects**.

## Scope

This episode covers:

- the FSM step contract as the authoritative transition boundary,
- the action model used by the domain,
- the projection/controller boundary that feeds the actor runtime,
- how this structure enables strict contract testing.

This episode intentionally excludes gateway bus details and output media (screenshots/videos).

## Approach

The core revolves around a single function contract in `crates/common/src/fsm/step.rs`:

- input: `current_state`, `current_ctx`, `event`, `now`
- output: `StepResult { next_state, modified_ctx, actions, transition_record }`

The key part is that `actions` are pure domain intents (`DomainAction`) and not direct I/O operations.

### Canonical Flow

1. An event enters the domain as `FsmEvent`.
2. `step(...)` updates context and computes state transitions.
3. Domain emits intent actions (e.g., publish sync, request front headlamp, warnings).
4. Runtime/controller layers decide how to execute those intents.

## Architecture Diagram

Primary diagrams for this episode:

- `blog-inputs/diagrams/01-system-context.mmd`
- `blog-inputs/diagrams/02-core-design-component-view.mmd`

Quick render option (if using Mermaid CLI):

```bash
mmdc -i blog-inputs/diagrams/01-system-context.mmd -o blog-inputs/diagrams/01-system-context.svg
mmdc -i blog-inputs/diagrams/02-core-design-component-view.mmd -o blog-inputs/diagrams/02-core-design-component-view.svg
```

```text
VehicleEvent / Physical input
          |
          v
PhysicalCarVocabulary  --(projector)-->  FsmEvent
          |                               |
          |                               v
          |                        step(current_state, current_ctx, event, now)
          |                               |
          |                               v
          +------------------------> StepResult
                                      |- next_state
                                      |- modified_ctx
                                      |- actions (DomainAction intents only)
                                      |- transition_record (audit/replay)
```

## Key Design Aspects

### 1) Single authoritative step boundary

The module defines explicit transition semantics and keeps domain output intent-only:

```rust
// crates/common/src/fsm/step.rs
#[derive(Debug, Clone, PartialEq)]
pub struct StepResult {
    pub next_state: FsmState,
    pub modified_ctx: VehicleContext,
    pub actions: Vec<DomainAction>,
    pub transition_record: TransitionRecord,
}

pub fn step(
    current_state: &FsmState,
    current_ctx: &VehicleContext,
    event: &FsmEvent,
    now: Instant,
) -> StepResult {
    // updates context, computes transition, emits actions
    // no hardware/network side effects here
}
```

Why this matters:

- deterministic input/output behavior is easy to reason about,
- replay and diagnostics become natural via `transition_record`,
- runtime concerns do not leak into the domain contract.

### 2) Domain action vocabulary separates intent from execution

`DomainAction` gives a stable contract from domain to runtime:

```rust
// crates/common/src/fsm/step.rs
pub enum DomainAction {
    StartBuzzer,
    StopBuzzer,
    PublishStateSync,
    LogWarning(String),
    RequestFrontHeadlampOn,
    RequestFrontHeadlampOff,
    EnterMode(ActorModeHintFromDomain),
}
```

The domain decides *what should happen*; runtime layers decide *how to do it*.

### 3) Projection/controller boundary keeps ingress disciplined

In `VehicleController`, physical vocabulary is projected before entering actor processing:

```rust
// crates/common/src/engine/controller/vehicle_controller.rs
pub async fn submit_physical_car_event(
    &self,
    event: PhysicalCarVocabulary,
) -> Result<(), VehicleControllerError> {
    let msg = self
        .projector
        .project(event)
        .map_err(|e| VehicleControllerError::Projection(format!("{e:?}")))?;
    self.actor
        .send_message(msg)
        .map_err(|e| VehicleControllerError::Messaging(format!("{e}")))?;
    Ok(())
}
```

This prevents transport-specific event forms from bypassing the canonical conversion layer.

### 4) Auditability and recoverability are first-class outputs

Transition records are generated on every step:

```rust
// crates/common/src/fsm/step.rs
pub struct TransitionRecord {
    pub at: Instant,
    pub event: FsmEvent,
    pub old_state: FsmState,
    pub next_state: FsmState,
    pub old_ctx: VehicleContext,
    pub current_ctx: VehicleContext,
}
```

This gives an observable trail for:

- debugging regressions,
- verifying contract behavior,
- replay-driven troubleshooting.

### 5) Runtime-actuation boundary exists even before hardening

The default actuation manager already routes domain actions through a command channel, not direct bus operations:

```rust
// crates/common/src/engine/controller/actuation_manager.rs
if let (Some(tx), Some(correlation_id)) =
    (&self.actuation_command_tx, self.next_correlation_id())
{
    let _ = tx
        .send(ActuationCommand::SwitchFrontHeadlampOn { correlation_id })
        .await;
}
```

This early separation makes later gateway hardening incremental rather than invasive.

## Output Artifacts

No screenshots or videos for this episode (by design).  
The emphasis is on architectural correctness and deterministic contracts.

## What We Proved in Episode 1

- The domain has a single deterministic transition boundary (`step`).
- Side effects are represented as intent (`DomainAction`) instead of direct I/O.
- Ingress projection and controller APIs enforce clear boundaries into actor runtime.

## Link to Episode 2

In Episode 2, we wire this deterministic core into runtime loops and gateway actuation paths:

- timer ticks and ingress dispatch,
- command routing through channels,
- initial end-to-end control loop behavior.

Working title: **Episode 2: From Deterministic Core to Runtime Wiring**.

Draft file: `blog-inputs/episode-02-runtime-wiring-and-actuation-path.md`
