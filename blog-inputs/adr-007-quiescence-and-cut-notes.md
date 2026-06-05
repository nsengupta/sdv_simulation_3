# Blog / README input — cuts and FSM quiescence (ADR-7)

Source of truth: [`docs/adr-007-fsm-quiescence-and-cut.md`](../docs/adr-007-fsm-quiescence-and-cut.md).

## One paragraph for Iteration 3

The digital twin’s journey is a **sequence of cuts** — each cut is operational mode plus the
summated vehicle context at one instant. Every ledger row is one hop between cuts. The FSM
**table alone** decides the next mode; when the world after a hop is unsafe but the table needs
another fact, the brain emits an **internal** event (e.g. lighting unsafe), runs another hop in
the **same** actor handling, and records a second row. That keeps the machine testable and the
offline tool honest about *why* mode changed.

## Diagram (reuse in post)

External ingress → (optional headlamp tell-back) → `run_to_quiescence` until no internal
events → persist final cut. Internal events never go back through the mailbox mid-chain.

## Terms to use consistently

| Term | Meaning |
| ---- | ------- |
| **Cut** | `(FsmState, VehicleContext)` at one instant |
| **Hop** | One FSM event processed (zone + `step`); one ledger row when applied |
| **Quiescence** | No more internal events queued for this external ingress |
| **Detector** | Pure “exit cut → optional `FsmEvent::Internal`” — not override |

## Ownership (blog one-liner)

`PendingBrainTurn` waits in the actor; `ResolvedTurn` exists only for one commit (moved in,
dropped out); quiescence is pure, apply is async. No shared pointers — see ADR-7 § Ownership.

## Pyramid (blog one-liner)

Detectors and `run_to_quiescence` live in L4; only `transition_map` changes mode (L2).
Twinlets never import the FSM. Downward calls only — see ADR-7 § Pyramid layering.

## Detectors — now vs target

**Step 7:** first small detector(s) (lighting) on the quiescence hook → `Internal` events → FSM table.

**Target:** a **library of detectors** — write and test each rule outside the twin, then plug
into brain quiescence (catalog). The table still owns mode; detectors only propose internal
events. Per-cell table function pointers are optional, not the main arc.

**Step 7 locked:** see ADR-7 § Step 7 confirmations (seven design Q&As — may be pruned when docs compact).
