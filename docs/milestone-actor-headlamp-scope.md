# Milestone: `milestone/actor-headlamp` — scope & handoff

**Repo:** `sdv_simulation_3`  
**Base:** `main` @ tag **`pyramid-m2-complete`**  
**Blog arc:** Iteration 3 (actorification) starts on this branch.

Use this file to resume work in a new chat: `@docs/milestone-actor-headlamp-scope.md`.

---

## Done on `main` (do not redo here)

- **Pyramid (modules in `common`):** L0–L6 layout; TangleGuard **clean**
- **ADR-5 M1:** L1 alphabets (`HeadlampState` / `Message` / `Outcome`; other zones stubbed)
- **ADR-5 M2:** `zone_turn` → slim `fsm::step` → `twin_turn`; L1 emits **`HeadlampOutcome`** only
- **ADR-6:** target brain / ingress / power coordination — **documented**, not implemented
- **Deferred:** `sdv_core` crate split; full `TwinIngress`; power barrier; ledger `applied`; offline replay tool

---

## Branch goal

Turn the **headlamp zone** from in-process `HeadlampContext` + parent `zone_turn` into a **child actor (twinlet)**, with **one parent brain actor** and **unchanged** user-visible behaviour (CAN, three processes, tests green).

---

## Naming (use consistently)

| Name | Meaning |
| ---- | ------- |
| **`HeadlampZoneReply`** | Zone twinlet reply after **one** [`HeadlampMessage`] — `{ ctx, outcomes }`. Not a brain/FSM *turn*. |
| **`HeadlampOutcome`** | Zone egress only (RequestOn, LogWarning, …) — L4 maps to `DomainAction`. |
| **`HeadlampContext::on_receiving_message`** | L1 pure handler → `HeadlampZoneReply` (pattern for all zones). |
| **`apply_headlamp_zone`** | Same handler via child RPC ([`HeadlampActorVocabulary`]). |
| **`HeadlampActorVocabulary`** | RPC envelope for [`apply_headlamp_zone`]. |
| **`brain_twin_turn`** | Brain-only: RPC(s) then one `twin_turn` with optional zone replies. |

Avoid `*Turn` for zone replies — reserved for brain/FSM (`twin_turn`, `brain_twin_turn`).

---

## Q5 — summated view (decided)

| Phase | L3 `VehicleContext.headlamp` | Source of truth per event |
| ----- | ---------------------------- | ------------------------- |
| **Now (A)** | Embed full `HeadlampContext` | Copy `HeadlampZoneReply.ctx` before `apply_step` |
| **Target (C)** | Handle / slim projection | Whatever the child still puts in `HeadlampZoneReply` |

**Rule:** `HeadlampZoneReply` is semantic truth; parent does not `apply` in parallel with the actor. Shrinking the reply surfaces gaps via tests (ledger / `GetStatus`).

**A→C bridge:** Brain is *ask child → wait → refresh embed → ledger/diagnostics*. Optional `headlamp_reply` on [`zone_turn`](crates/common/src/twin_runtime/zone_turn.rs) only skips local `on_receiving_message` when the twinlet already handled that message — **temporary** until demux splits.

**L1 pattern (other zones):** `{Zone}Context::on_receiving_message(msg, now) -> {Zone}ZoneReply`.

---

## Shutdown order (remember)

**Target:** assembly twinlets stop **before** the brain stops (supervisor-ordered teardown).  
**Interim:** brain `post_stop` stops headlamp — acceptable only until linked supervision / explicit ordered shutdown exists. Do not treat brain-owned `child.stop()` as the long-term model.

---

## Child → parent contract

```text
HeadlampMessage → apply_headlamp_zone → HeadlampZoneReply
Brain merges outcomes; embeds ctx; apply_step
```

---

## Tests (layers)

| Layer | Runs | Friction signal toward C |
| ----- | ---- | ------------------------ |
| **L1** | `on_receiving_message` / lighting contracts | Policy without ractor |
| **L4 pure** | `twin_turn` (sync zone reply) | Demux + FSM |
| **Actor** | Brain + headlamp child | RPC + embed |
| **Step 4** | `headlamp_reply_contract` | Ledger `current_ctx.headlamp` vs `GetStatus`; pending/settled |

Gateway e2e (Phase B) — deferred; will fail if snapshot fields shrink without reply/query path.

---

## In scope / out of scope

**In:** headlamp actor, brain dispatch, embed from `HeadlampZoneReply`, tests, README when structure changes.  
**Out:** other zone actors, ADR-6 power barrier, `TwinIngress` on controller, `sdv_core` split, actuation child.

---

## Architecture

```text
Controller → VirtualCarActor (brain)
Brain → apply_headlamp_zone (HeadlampActorVocabulary) → HeadlampActor
Child → HeadlampZoneReply → brain → actuation / diagnostics / apply_step
```

---

## Success criteria (merge to `main`)

1. Headlamp behind child actor boundary.  
2. All tests green.  
3. Layering intact; Q5 + naming + shutdown notes in PR.  
4. Step 4 ledger/embed alignment tests.

---

## Key paths

| What | Path |
| ---- | ---- |
| L1 reply + apply | `crates/common/src/vehicle_state/front_headlamp.rs` |
| Headlamp actor | `crates/common/src/twin_runtime/headlamp_actor.rs` |
| Demux / twin turn | `crates/common/src/twin_runtime/{zone_turn,twin_turn}.rs` |
| Brain | `crates/common/src/twin_runtime/controller/virtual_car_actor.rs` |
| Step 4 tests | `crates/common/src/test/headlamp_reply_contract.rs` |

---

## Completion log

| Step | Status | Notes |
| ---- | ------ | ----- |
| 1 `on_receiving_message` | done | `HeadlampZoneReply` |
| 2 `HeadlampActor` | done | `apply_headlamp_zone` / vocabulary struct |
| 3 Brain dispatch | done | `brain_twin_turn` |
| 4 Ledger/reply tests | done | `headlamp_reply_contract.rs` |
| 5 README | pending | Structural change |

---

## Process

- **Commits:** confirm before commit  
- **One line:** First zone actorification — headlamp child under unchanged parent brain.
