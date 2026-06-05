# ADR-7 — FSM quiescence, cuts, and internal operational events

**Status:** `ACCEPTED` (target architecture; step 7 implementation pending)  
**Date:** 2026-06-04  
**Series:** follows [`adr-006-twin-brain-ingress-coordination.md`](adr-006-twin-brain-ingress-coordination.md)  
**Related:** [`milestone-actor-headlamp-scope.md`](milestone-actor-headlamp-scope.md), [`design-notes-runtime-observation.md`](design-notes-runtime-observation.md) (Q6 / cuts), headlamp tell-back (step 6)

**Audience:** README, blog Iteration 3, code comments, offline ledger tool.

---

## Context

Step 6 decoupled the brain and headlamp twinlet mailboxes (tell / tell-back). Step 7 adds
**operational policy**: when the summated world is unsafe (e.g. driving in the dark without a
confirmed lamp), the twin must enter a distinct operational mode and alarm — without putting
policy inside assembly actors and without **overriding** `next_state` outside the FSM table.

We also need **replay**: the offline tool must see **internal** FSM events explicitly in the
ledger, not silent jumps in mode.

---

## Glossary — what is a **cut**?

A **cut** is one **snapshot** of the digital twin at an instant in the journey:

```text
cut = (FsmState, VehicleContext)
```

| Part | Role |
| ---- | ---- |
| **`FsmState`** | Operational mode at that instant (Off, Idle, Driving, `DrivingDangerously`, …). |
| **`VehicleContext`** | Summated **world** at that instant: headlamp, visibility (lux), powertrain, health, … |

**Ledger:** each applied transition record is one **hop** between cuts. It stores
`old_state` / `old_ctx` (entry cut) and `next_state` / `current_ctx` (exit cut), plus
`event` and `record_seq`. The journey is a **sequence of cuts** ordered by `record_seq`; wall
time comes from published `at_unix` (see runtime observation notes).

**Verifiers** fold pure laws (e.g. [`verify_state_laws`](../crates/common/src/digital_twin/car_behaviour_checker.rs)) over each cut — node consistency, not edge legality.

**Detectors** (below) read the **exit cut after a hop** and may enqueue another FSM event; they
do not mutate `next_state` directly.

---

## Decision summary

| Topic | Choice |
| ----- | ------ |
| Who sets `next_state` | **`transition_map` only** — inviolable; read the table to predict mode. |
| Operational “policy” | **Detectors** emit `FsmEvent::Internal(Operational::…)`; table handles the transition. |
| No override | No post-`step` patch of `next_state` or shadow state machine. |
| Runner | **`run_to_quiescence`** — mandatory at every **turn commit**; processes a queue until empty. |
| Internal events in ledger | **One row per hop** (external + internal); tool filters `Internal(...)`. |
| Twinlets | Assembly truth only; **no** brain worldview in zone actors. |
| Actor mailbox | One external ingress per logical turn; internal hops run **inside** the same `handle` (no `cast` self-Fsm for the chain). |
| Code shape | **One commit helper** replaces scattered `commit_brain_turn` + `apply_committed_turn` call sites — must **reduce** duplication, not add parallel types. |
| Detectors (step 7) | First **small** lighting detector(s) in L4 after each hop → optional `FsmEvent::Internal`. |
| Detectors (target) | A **library of detectors** — developed/tested **outside** the hot path, then **plugged into** brain quiescence (catalog + ordered run). Not twinlet code; not override of `next_state`. |
| Transition table `fn` pointers (optional) | Per-cell handlers only if they **reduce** duplication vs the detector library; **not** the primary end-state — the library is. |

---

## Inviolable FSM rule

On every hop:

1. Zone merge (or pre-merged embed from tell-back) → `VehicleContext` for this event.
2. **`transition(current_state, event, ctx, now)`** → `next_state` (only authority).
3. **`output(old, new, ctx)`** → buzzer / sync actions as today.

Detectors run **after** step 3 for that hop. If they enqueue
`FsmEvent::Internal(Operational::LightingUnsafe)`, the **next** hop uses the **same** pipeline;
`Driving` + `Internal(LightingUnsafe)` → `DrivingDangerously` must be a **row in the table**.

---

## Detector library (target — post step 7)

**Intent:** operational rules live in a **catalog of small detectors** that:

- Take the **exit cut** (and the event that just fired) → return `Option<FsmEvent::Internal(…)>`.
- Are **authored and unit-tested in isolation** (no actor, no ractor) before integration.
- Are **registered** with the digital twin brain (ordered list or table of `fn` pointers — implementation detail later).
- Stay in **L4** (or a dedicated module the runtime owns); **never** in L1 twinlets.

**Integration point (stable):** inside `run_to_quiescence`, after each hop — same hook as step 7’s first lighting detector.

**Not the goal:** a monolithic `apply_operational_policy` override; detectors scattered inside `transition_map` match arms unless that layout genuinely wins over the catalog.

**Table stays the mode story:** detectors only **propose events**; `transition_map` still owns every `next_state` change.

---

## `FsmEvent::Internal` (ledger-visible)

```rust
// Target shape (illustrative)
pub enum FsmEvent {
    // … existing external facts …
    Internal(Operational),
}

pub enum Operational {
    LightingUnsafe,       // e.g. driving in dark without confirmed ON
    LightingRecovered,    // TBD — recovery paths in transition table
    // grow: other brain-synthesized facts
}
```

Project to `PublishedFsmEvent::Internal { … }` for the offline tool. External ingress rows
stay distinguishable from brain-synthesized rows.

---

## `run_to_quiescence` — who, when, always?

**What:** Pure L4 function: given initial `(state, ctx)`, one **external** `FsmEvent`, zone
embeds, and `now`, run hops until detectors enqueue nothing.

**When:** **Always** at the **commit boundary** for a **completed** turn (zone work fully
merged for that external event). Not optional when “no policy applies”: zero internal events
⇒ **one hop** then exit (same as today’s single `commit_brain_turn`).

**Who calls it:** Brain orchestration only — not controller, not headlamp twinlet.

**Not called when:**

- `GetStatus` (read-only).
- `begin_fsm_turn` returns after **tell only** (turn incomplete; pending).
- Fsm pushed to **backlog** while another turn is pending (ADR-6 will replace backlog with
  ledger `applied: false` — see ADR-6 shutdown observability).

### Two call timings, one function (tell-back deferral)

```text
External Fsm(evt)
  ├─ needs headlamp tell?  YES → tell → pending → handle returns
  │                         later HeadlampZoneReady → commit_resolved_turn(evt, reply)
  └─ NO  → commit_resolved_turn(evt) inside same handle
```

**Not** two different policies — same `commit_resolved_turn` → `run_to_quiescence` → apply +
ledger.

### Hop loop (per mailbox commit)

```text
queue ← [external_evt]
while let evt = queue.pop():
  zone_merge(evt) → ctx
  step(state, ctx, evt) → state', actions     // table sets state'
  if let Some(internal) = detect(exit_cut, evt):
      queue.push(internal)
emit ledger row per hop
apply_step(final cut)
actuation / diagnostics from merged actions
```

Cap max hops (e.g. 8) against detector cycles.

---

## Code shape (avoid noise)

**Target:** one orchestration entry — e.g. `commit_resolved_turn` in
`virtual_car_actor` / `twin_turn` — that:

1. Calls `run_to_quiescence` (pure).
2. Applies capsule + ledger + egress once.

**Remove** duplicate `commit_brain_turn` + `apply_committed_turn` paths when implementing.
Do **not** add a second parallel “policy apply” API alongside `step`.

Pure tests (`operational_policy_contract`, `fsm_engine_contract`) call
`run_to_quiescence` directly without ractor.

---

## Ownership (Rust — commit boundary only)

Step 7 introduces **short-lived owned values** at commit time. No `Arc`/`Rc`, no `'_`
lifetime parameters on these structs (all fields are owned/`Clone`/`Copy` today).

| Value | Stored in actor state? | Lifecycle |
| ----- | ---------------------- | --------- |
| **`PendingBrainTurn`** | **Yes** — `Option<…>` while tell(s) outstanding | Created when tell deferred; removed with `Option::take()` at commit |
| **`ResolvedTurn`** | **No** — stack / local only | Built only when zone embed(s) for the external event are **complete**; **moved** into `commit_resolved_turn` by value; dropped when commit returns |
| **`QuiescentResult`** | **No** | Returned from pure `run_to_quiescence`; **moved** into `apply_committed_quiescence`; dropped after ledger + `apply_step` |

**Handoff (use move semantics, not clones at boundaries):**

```text
pending_turn.take()  →  PendingBrainTurn  (owned)
       .into_resolved(...)  →  ResolvedTurn  (consumes pending)
commit_resolved_turn(runtime, resolved)     (consumes resolved)
    run_to_quiescence(&resolved, …)  →  QuiescentResult  (owned)
    apply_committed_quiescence(runtime, quiescent)  (consumes quiescent)
```

**Rules:**

- Do **not** store `ResolvedTurn` beside `pending_turn` (one source of truth for “waiting”).
- Build `ResolvedTurn` only after the **last** required tell-back (including ignition-off reset when applicable).
- Split **pure** `run_to_quiescence` from **async** `apply_committed_quiescence` so nothing borrows `runtime` across `.await`.
- `#[must_use]` on `ResolvedTurn` / `QuiescentResult` where it prevents build-and-drop mistakes.

**Implementation:** replace `commit_brain_turn` + `apply_committed_turn` call sites with
`commit_resolved_turn` — **delete** the old pair; do not wrap them.

---

## Pyramid layering (inviolable)

Acid test ([`design-notes-pyramid-layers.md`](design-notes-pyramid-layers.md)): **no layer imports or calls upward.** TangleGuard at milestone completion must stay clean.

| Piece | Layer | May import |
| ----- | ----- | ---------- |
| `transition_map`, `step`, `FsmEvent` | **L2** | L1, L0 only |
| Detectors (`after_hop → Option<FsmEvent::Internal>`) | **L4 hook** today → **L2 / `fsm` sibling** target ([§ deferred](#deferred--detector-placement-table-slots-and-physics)) | L1, **L0 `vehicle_physics`** — **not** vice versa |
| `run_to_quiescence`, `zone_turn` | **L4** | L2, L1 |
| `DigitalTwinCar`, `apply_step` | **L3** | L2, L1 |
| `VirtualCarActor`, `commit_resolved_turn` | **L4** actor | L3, L2, L4 pure |
| Headlamp **twinlet** | **L4** child actor | L1 headlamp alphabet only — **no** `FsmState`, no detectors, no `transition` |
| Controller / gateway | **L5 / L6** | Projects to `Fsm` / `PhysicalCarVocabulary` — never calls `transition` directly |

**Forbidden:**

- L1 `vehicle_state` importing `fsm` (resolved in M2; do not reintroduce).
- Twinlet or L1 handler choosing `next_state` or emitting `FsmEvent::Internal`.
- L2 `step` calling actor, ledger, or detector side effects.
- A second “policy” module above L2 that patches `next_state` without going through `FsmEvent` + table.

**Allowed call chain (downward only):**

```text
L6 → L5 → L4 actor → run_to_quiescence → zone_turn (L4) → step / transition (L2)
                              → detectors (L4) → enqueue Internal → next hop (L2)
                              → L3 apply_step
```

README / blog / consolidated docs should cite this section when describing step 7.

---

## Relationship to ADR-6

| ADR-6 | ADR-7 |
| ----- | ----- |
| Power barrier, `applied: false` on suppressed ingress | Operational internal events, `applied: true` per hop |
| `TwinIngress`, coordination beside `FsmState` | `FsmEvent::Internal` through same quiescence runner |
| Shutdown observability in ledger | Each internal hop is its own ledger row |

Cross-link: [ADR-6 § Ledger tool / shutdown observability](adr-006-twin-brain-ingress-coordination.md#ledger-tool--shutdown-observability).

---

## Step 7 confirmations (2026-06-04)

Design Q&A closed before implementation. May be folded into README/blog when docs are
compacted; until then this is the implementation contract.

| # | Topic | Decision |
| - | ----- | ---------- |
| 1 | **LightingUnsafe** detector | Fire only when exit cut is **Driving** + `ambient_lux <= LUX_ON_THRESHOLD` + `headlamp.state == Off` (incl. post-timeout). **Not** while `OnRequested`; **not** from Idle/Off. |
| 2 | **Recovery** | **Table-only** (normal `FsmEvent` + `transition_map` + `output`). No `Internal` recovery events in 7a. |
| 3 | **`Internal` hops** | **No `zone_turn`** — brain-only; ctx from prior hop. **Strict tests** that L1 assemblies are unchanged on internal hops. |
| 4 | **Buzzer** | **`StartBuzzer` / `StopBuzzer` only via `output()`** on table transitions. Detectors emit events only. |
| 5 | **Precedence** | Owned by **twin** (detector guards / catalog order). Table runs first; lighting internal only if mode still **Driving** (not e.g. `ExtremeOperationWarning` on same hop). **Strict tests**. |
| 6 | **Ledger / apply** | **One ledger row per hop**; **`apply_step` once** on final cut; **`as_of_seq`** semantics unchanged; **actuation** = merged actions from all hops in order. |
| 7 | **`run_to_quiescence`** | **Decided** — single pure runner for tests and `commit_resolved_turn` (not optional). |

**Resume workflow:** extend **red tests first** (may fail — no implementation yet). Each test must
pin one design expectation; add **one-by-one** with review before code. Target file:
[`operational_policy_contract.rs`](../crates/common/src/test/operational_policy_contract.rs)
(+ new modules only if the file grows too large). Suggested additions still open:

| Test intent | Design pinned |
| ----------- | ------------- |
| Internal hop → **no L1 mutation** | `zone_turn` skipped; headlamp/lux unchanged vs prior hop cut |
| **OnRequested** in dark while Driving | **no** `LightingUnsafe` (ACK still pending) |
| Same hop → **ExtremeOperationWarning** | **no** `LightingUnsafe` (precedence / detector guard) |
| `run_to_quiescence` two-hop journey | external `TimerTick` then internal row semantics (final mode + buzzer) |
| Published / record `event` | `Internal(LightingUnsafe)` visible on second hop (when ledger tests exist) |

Implementation starts only when the agreed test list is stable (or per-test green as we go).

---

## Implementation phasing

| Step | Scope |
| ---- | ----- |
| **7a** | Per confirmations above: `Internal` + published mirror; `run_to_quiescence`; lighting detector; table/`output` rows; `commit_resolved_turn`; strict internal-hop + precedence tests; red tests green |
| **7b** | Recovery internal events + `output` buzzer edges |
| Later | Detector **catalog** grows (power, health, …); ADR-6 barrier; optional `fn` pointers in table only if catalog + quiescence hook is insufficient |

---

## Deferred — detector placement, table slots, and physics

**Observations (2026-06-04, extend as design matures):**

### A. Per-state detector slot in the transition table

Step 7a gates `lighting_unsafe_detector` with `exit_state == Driving`, so latched
**`DrivingDangerously`** never runs lighting rules. That works but duplicates “which modes may
synthesize events” inside each detector.

**Revisit:** attach an optional **detector `fn` pointer per `FsmState`** in `transition_map`
(or a parallel table keyed by state), default **`None` / no-op**. Quiescence calls only the slot
for the **exit state** after each hop — e.g. `Driving → lighting_unsafe`, all other states →
skip without per-detector mode checks. Mode eligibility stays with the mode story (table).

### B. Layer — co-locate with L2 (FSM), not L4 runtime

Detectors take **`FsmState`** (and exit cut) as input and synthesize **`FsmEvent::Internal`**.
They are operational-policy peers of **`transition_map`**, not orchestration.

**Revisit:** move the catalog to **`fsm/detectors/`** (part of L2) or an **L2 sibling module**
in the same crate facade — **not** `twin_runtime` long term. Step 7a keeps
`twin_runtime/detectors/` as an interim hook only; `run_to_quiescence` (L4) should **call into**
L2, not own the rules.

Pyramid rule unchanged: detectors **may import L1 + L0**; L2 `step` still does not call detectors
with side effects; L4 quiescence invokes them **after** each hop.

### C. Physical world — consult `vehicle_physics` (all detectors)

Detectors judge the summated world against the **same constitution** as L2 enforce paths.
**Every** operational detector — present and future — must take thresholds and derived
predicates from **`crates/common/src/vehicle_physics/*`** (constants, kinematics, formulas added
as the physical model grows). No ad-hoc duplicates inside detector modules.

| Detector era | `vehicle_physics` usage |
| ------------ | ----------------------- |
| **Now (lighting)** | [`LUX_ON_THRESHOLD`](../crates/common/src/vehicle_physics/constants.rs) — same as L1 headlamp zone |
| **Next (kinematic / stress)** | `speed_threshold_exceeded`, `extreme_operation_active`, `operational_warning_active`, `RPM_*` — same as [`transition_map`](../crates/common/src/fsm/transition_map.rs) |
| **Later (health, thermal, …)** | new L0 helpers first; detectors and table rows both import them |

When a rule needs a new physical predicate, **add it to `vehicle_physics`**, then wire L2
enforce, L3 law, and detector from that single definition.

Cross-link: [pyramid — Physical rules: enforce + detect](../docs/design-notes-pyramid-layers.md#physical-rules-enforce--detect-same-l0-constants).

**Step 7a interim:** `twin_runtime/detectors/` + inline `exit_state == Driving` guard. **Target:**
L2-adjacent catalog + per-state table slots + `vehicle_physics` predicates only.

---

## Consequences

**Positive:** FSM remains independently testable; table is the mode story; ledger tool sees
internal causality; aligns with step 6 tell-back (commit only when embed ready).

**Negative:** Multiple ledger rows per external ingress; transition table grows with internal
events; detectors must stay pure and small.

---

## References

- [`crates/common/src/fsm/transition_map.rs`](../crates/common/src/fsm/transition_map.rs) — mode graph
- [`crates/common/src/twin_runtime/twin_turn.rs`](../crates/common/src/twin_runtime/twin_turn.rs) — today’s single-hop turn (→ quiescence)
- [`crates/common/src/test/operational_policy_contract.rs`](../crates/common/src/test/operational_policy_contract.rs) — step 7 TDD (red)
