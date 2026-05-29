# Design notes: runtime observation channels & the road to actorification

Scope: the three optional channels injected via `VehicleControllerRuntimeOptions`
(`transition_tx`, `diagnostic_tx`, `actuation_command_tx`), the snapshot RPC, the
transition ledger, time handling, and how all of this should be shaped given the
upcoming split of the monolithic `VirtualCarActor` into a parent FSM actor + child
actuation/observability actors.

Anchors in the code at time of writing:
- `crates/common/src/engine/controller/vehicle_controller.rs` — `VehicleControllerRuntimeOptions`.
- `crates/common/src/engine/controller/virtual_car_actor.rs` — the actor loop (persist → emit record → run actions → emit diagnostics).
- `crates/common/src/engine/controller/actuation_manager.rs` — `DefaultActuationManager`, the no-op TODOs.
- `crates/common/src/fsm/step.rs` — pure `step(state, ctx, event, now) -> StepResult`, `TransitionRecord`.
- `crates/common/src/engine/op_strategy/transition_map.rs` — `transition` / `output` (where actions are born).
- `crates/common/src/transition_sink.rs`, `crates/common/src/diagnostic/mod.rs` — the two sinks.
- `crates/common/src/digital_twin/{mod.rs,car_behaviour_checker.rs}` — `verify_all_invariants`, the laws.
- `crates/common/src/engine/controller/actuation_contract.rs` — `ActuationCommand` / `ActuationFeedback` / `CorrelationId`.

---

## Q1 — Do `transition_tx` and `diagnostic_tx` really need to be separate?

**Verdict: keep them separate, but they are not peers.** One is a *fact ledger*, the
other is a *best-effort operational log*. The state-transition diagnostic is indeed
derived and is currently emitted twice — that duplication should be removed.

Why they differ structurally today:

| | `transition_tx` (`RawTransitionRecord`) | `diagnostic_tx` (`DiagnosticMessage`) |
|---|---|---|
| Channel | bounded `mpsc::channel(N)` | **unbounded** |
| Delivery | lossless-or-error (Full/Closed surfaced) | best-effort, fire-and-forget |
| Ordering | `sequence_no`, total order | none guaranteed |
| Audience | machines: replay, invariant checks | humans/logs |
| Sources | parent FSM only (1 per step) | many: init, timer tick, actuation failure, sink-overflow meta |

A diagnostic is *partly* derivable from transitions — `diag_state_transition` is
literally `(identity, next_state)` taken straight from a transition. But the diagnostic
stream also carries events that are **not** transitions: the init message, `TimerTick`
heartbeats, actuation failures, and "transition sink full/closed" meta-diagnostics. So
you cannot reconstruct the diagnostic stream from the transition stream alone.

The clean mental model:
- `transition_tx` = the **primitive, authoritative FSM fact ledger** (one record per event).
- `diagnostic_tx` = a **cross-cutting, multi-source presentation/telemetry bus**.

Actorification angle: after the split, child actors (actuation, headlamp connector)
will need to emit diagnostics but they do **not** produce FSM transitions. So the
diagnostic bus must remain a shared, separate channel — this *strengthens* the case for
two channels. The cleanup is the other direction: stop the parent from directly emitting
the *state-transition* diagnostic. Let a future observer/telemetry actor subscribe to the
transition ledger and **project** those diagnostics. The parent then emits to
`diagnostic_tx` only for things that are not transitions (lifecycle, actuation outcome,
sink overflow).

---

## Q2 — Can we test `actuation_command_tx` by injecting the harness's own tx/rx?

**Verdict: yes — that is already the intended seam, and it's the idiom the repo uses.**

`actuation_command_tx: Option<mpsc::Sender<ActuationCommand>>` is injected through
`VehicleControllerRuntimeOptions`. A test creates `(tx, rx) = mpsc::channel(N)`, passes
`tx` in, drives events, then asserts on `rx.recv()` (e.g.
`ActuationCommand::SwitchFrontHeadlampOn { correlation_id }`). This mirrors exactly how
`actor_contract.rs::scenario_raw_transition_records_are_emitted_in_order` already tests
`transition_tx`.

**The harness plays the role of the future child actor.** Post-actorification the
actuation child owns the rx side, performs connector I/O, and feeds `ActuationFeedback`
back. In tests the harness substitutes for that child: it owns rx (asserts the outbound
command) and can inject the ack/nack back as events
(`submit_physical_car_event` / `submit_fsm_event`) to close the round trip. The
front-headlamp e2e tests already drive the feedback side via `PhysicalCarVocabulary`.

Caveats:
- `CorrelationId.session_id` is derived from `SystemTime::now()` → **non-deterministic**.
  Assert on structure and on `sequence_no` monotonicity, never on the exact `session_id`.
- Use a channel capacity large enough that the actor never hits backpressure mid-scenario,
  or drain promptly, to keep ordering assertions deterministic.

Suggested ergonomics: a test helper returning `(controller, actuation_rx)` plus a
one-liner to inject the matching ack — so a test reads "send command → observe command →
inject ack → observe resulting transition."

---

## Q3 — `GetStatus` / `RefreshStatus`: the RESP may be stale. How to live with it.

**Verdict: keep `GetStatus`, keep it pure/read-only, and make staleness *explicit*
with a sequence stamp. Do not add a mutating `RefreshStatus`.**

`GetStatus` is a `ractor` `call` (RPC reply port). It is processed in mailbox order, so
the reply reflects the actor's state *at the moment it processes that message*. By the
time the caller reads the value, newer events may already be applied. The snapshot is
never *wrong* — it is *as-of a point in the event order*. This is intrinsic to async
actors; you cannot remove it, only make it legible.

How to deal with the inevitability:
1. **Stamp the snapshot with a logical version** = the last applied transition
   `sequence_no` (the actor already maintains `next_sequence_no`). Add e.g.
   `as_of_seq: u64` to `DigitalTwinCar` (or to the reply). A consumer then knows "this
   snapshot reflects events ≤ N" and can reconcile it against `transition_tx` records
   with `sequence_no > N`. Today `DigitalTwinCar` carries no version → staleness is
   invisible.
2. **Prefer the transition ledger over polling for verification.** Polling `GetStatus`
   races with in-flight events; the ledger is exact. Use `GetStatus` for *settled*
   assertions (after draining) and for live UIs.
3. **Keep it read-only.** A `RefreshStatus` that *forces recomputation* would break the
   documented contract that `GetStatus` "does not call `transition`." Recomputation
   belongs to events, not to a query. If "refresh" means "give me the newest", the
   version stamp + draining the ledger already answers that.

Actorification angle: with child actors, a parent snapshot can only ever summarize the
parent's view; child state (in-flight actuation) is reflected later as feedback events.
The `as_of_seq` stamp is what lets a consumer say "snapshot is at N, but commands up to M
are still outstanding."

---

## Q4 — `transition_tx` should include the actions taken (names + params).

**Verdict: agree, and it's a small, high-value change. Record the *intended* actions
(deterministic, from the pure step), not execution outcomes.**

Today `TransitionRecord` = `{ at, event, old_state, next_state, old_ctx, current_ctx }`.
`StepResult` carries `actions: Vec<DomainAction>` *separately*, and the actor consumes
them in the loop **without** recording them. So a `transition_tx` consumer cannot see
what the FSM decided to do.

Proposed change: add the emitted actions to the record, e.g.
`actions: Vec<DomainAction>` (or a slim `Vec<ActionSummary { name, params }>`).
`DomainAction` already derives `Debug/Clone/PartialEq`, so it's cheap.

Important distinction this preserves:
- The record is produced **before** actions run, by a **pure** step. So `actions` in the
  record = **intended/emitted** actions (deterministic), *not* "succeeded/failed."
- Execution **outcomes** (ack/timeout/nack/failure) are separate facts. Today failures go
  to `diagnostic_tx`; successes come back as feedback events that generate their *own*
  transition records. The loop closes naturally.

Cleanups to fold in:
- Filter out `DomainAction::EnterMode(_)` from the recorded list (it's a runtime control
  hint, not a domain action) — or record it in a separate field.
- Consider embedding the `CorrelationId` of any actuation-producing action so the record
  becomes provenance: transition N → command(corr) → feedback event → transition M.
  (Best done *with* actorification; see Q9.)

**Naming hazard once both numbers live in one record (decided):** the record already
carries `RawTransitionRecord.sequence_no` (the **ledger** counter, Counter A — see Q7),
and the embedded action's `CorrelationId.sequence_no` is a **different** counter (Counter
B, the command counter). The moment both coexist in one struct, two unrelated 1-based
`u64` series share the field name `sequence_no`. **Disambiguate before they coexist:**
rename the ledger field to `record_seq` (or `ledger_seq`), and keep `CorrelationId`'s as
the command axis (e.g. read as `correlation_id.command_seq`). A reader of the
transition/actuation log must never be able to confuse "ledger position" with "command
position." Naming is the contract here.

This single change also resolves Q5 and feeds Q6/Q9.

---

## Q5 — Why aren't `StartBuzzer`/`StopBuzzer`/`PublishStateSync`/`LogWarning` already in `transition_tx`?

**Verdict: because the record carries no actions at all yet (Q4). Do Q4 and these become
observable for free — which is exactly the right home for the still-no-op ones.**

These four are `DomainAction`s minted in `step`/`output` and routed to the
`ActuationManager`, which currently **no-ops** them (the `TODO(actuation-*)` arms). They
have no connector wired, so their *only* meaningful observable today **is their presence
in the action list**. Recording the emitted actions (Q4) means the harness/diagnostics
can already assert e.g. "Driving → ExtremeOperationWarning emitted `StartBuzzer` +
`LogWarning(...)`" without the actuation side doing anything. The no-op `execute()` is
then fine: the *fact* is captured at the ledger, and the *effect* is filled in later
(actuation child actor / egress connector) without changing the ledger contract.

Flagged smell: **`LogWarning` is really observability, not actuation.** It is generated in
two places (`step` for `RejectedPowerOff`, `output` for threshold/extreme messages),
routed as a `DomainAction` the manager no-ops, while a parallel diagnostic stream already
exists. After Q4 it will show up in the record; separately it would be cleaner to route
`LogWarning` **directly to the diagnostic sink** rather than through the actuation path.
Candidate reclassification (small).

---

## Q6 — Run `verify_all_invariants()` over every cut in the journey; tests call checkers as library functions.

**Verdict: correct goal. Expose a *pure, public* state-law entry point that takes
`(&FsmState, &VehicleContext)`; keep `verify_all_invariants()` as the snapshot-level
wrapper. Then tests fold the laws over each captured cut.**

Current shape:
- `DigitalTwinCar::verify_all_invariants()` needs a full twin (identity + state + ctx).
- The individual laws in `car_behaviour_checker.rs` are `pub(super)` — invisible outside
  the `digital_twin` module — and already pure on `(&FsmState, &VehicleContext)`.

Every cut is reconstructable from a `RawTransitionRecord`: it carries
`old_state/next_state` and `old_ctx/current_ctx`. So a journey check iterates records and
evaluates the laws at each `(state, ctx)` cut — non-invasively, with no actor access.

Two frictions to remove:
1. The laws are not reachable from tests → **bump visibility** and add a free function,
   e.g. `pub fn verify_state_laws(state, ctx) -> Result<(), Vec<LawViolation>>`. Keep it
   as a **catalog of named laws** so the harness can report *which* law failed at *which*
   `sequence_no`.
2. `verify_all_invariants()` should become a thin wrapper: identity/health checks +
   `verify_state_laws(...)`. The state-law subset is what runs over the journey (identity
   is constant; health is part of ctx).

Note: `ExtremeOperationWarning(Instant)` carries a non-deterministic instant, but the laws
only read ctx, so cut-checking is unaffected. (See Q7 for equality.)

This is an easy, do-it-now change (mostly a visibility bump + one pure entry point + a
test fold helper).

---

## Q7 — Non-deterministic payloads: `TransitionRecord.at = Instant::now()` and `ExtremeOperationWarning(Instant)`.

**Verdict: agree — keep the `Instant` as elemental data (durations are derived, don't
pre-fold). But separate "elemental capture" from "ordering" and from "equality":**

- **Ordering: use `sequence_no`, not `at`.** `RawTransitionRecord.sequence_no` is a
  monotonic, total, clock-independent order. Within one actor, mailbox order ==
  `sequence_no` order, which is *stronger* than timestamp order (timestamps can tie). I'd
  gently amend the framing "order is determined by the message timestamp": prefer
  `sequence_no` for ordering; use `at`/durations only for *temporal* properties (cooldown
  elapsed, command latency).
- **Capture: keep `at` and keep `began_at` in `ExtremeOperationWarning`.** They are the
  raw material for duration/aberration analysis (e.g. the 5 s cooldown is literally
  `now - began_at`). Don't discard.
- **Determinism: inject the clock (Q8).** The non-determinism enters only at the call
  sites `fsm::step(..., Instant::now())` and `at: now` *inside the actor*. `step` itself
  is already pure w.r.t. time (it takes `now`). A clock seam makes the records and state
  instants reproducible in tests.
- **Equality in tests: compare on the discriminant / projected `VehicleState`, not the
  raw `Instant`.** Tests already do `matches!(s, ExtremeOperationWarning(_))` and avoid
  asserting on `at`. Formalize that: a helper that compares states ignoring the instant,
  or compare via `VehicleState::from(&state)` which drops the instant.

### Q7 addendum — there are TWO `sequence_no` counters; `seq` is per-source, not global

Inspection (whole tree) found two **physically independent** counters that happen to
share the name `sequence_no` and the same 1-based `u64` value space:

| | Counter A — ledger | Counter B — correlation |
|---|---|---|
| Field | `RawTransitionRecord.sequence_no` | `CorrelationId.sequence_no` |
| Stored in | `VirtualCarRuntimeState.next_sequence_no: u64` | `DefaultActuationManager.next_sequence_no: AtomicU64` |
| Bumped | `try_emit_transition_record` (`saturating_add(1)`) | `next_correlation_id` (`fetch_add(1, Relaxed)`) |
| Cadence | **every FSM event** (one per `step`) | **only when a correlated actuation command is emitted** |
| Scope | `car_identity` | `(source_id, session_id)` — and `source_id == car_identity` |
| Leaves process? | no | yes — packed onto CAN, **narrowed to `u32`** in `vehicle_device_bus` codec/can |

(Table uses *current* type names. Per recommendation 7 these rename to:
`TransitionRecord` → `RawTransitionRecord`, current `RawTransitionRecord` →
`PublishedTransitionRecord`, and its `sequence_no` → `record_seq`.)

Findings:
- **No shared state, no aliasing, no cross-assignment.** Bumping one never affects the
  other. There is no bug today: nothing compares them.
- They overlap only in **value range** and **field name**. They count different things on
  different axes and drift apart immediately (e.g. ledger `3` ↔ correlation `1` once the
  first headlamp command is emitted). They must never be cross-referenced.
- This is the concrete evidence for the earlier point: **`seq` is meaningful only
  per-source.** Counter B is *already* namespaced by `(source_id, session_id, seq)` on
  purpose. Counter A is namespaced only by `car_identity` today and has no session.

**Decisions (agreed):**
1. **Naming is the contract.** Rename Counter A's field to `record_seq` / `ledger_seq`;
   keep Counter B as the command axis (`correlation_id.command_seq`). Do this *before* Q4
   puts both in one record. A reader must never confuse ledger position with command
   position.
2. **More actuators → more child actors tomorrow.** Each child is its own source. For the
   **ledger** keep a single writer so `record_seq` stays a true total order — favour a
   dedicated journal/ledger actor that owns Counter A; children *message* it rather than
   minting ledger numbers. For **correlation**, the opposite is correct: each
   actuator/child owns its own Counter B, namespaced by its own `source_id`
   (+`session_id`), so command/feedback uniqueness holds without a central counter.
3. Counter A should gain a session/epoch concept too (mirroring B) so restarts don't
   reuse `record_seq` ambiguously — ties to the `as_of_seq` snapshot stamp (Q3).

### Q7 addendum — timestamp coverage (confirmed)

**Every `transition_tx` record carries the `Instant`.** `TransitionRecord` is constructed
in exactly one place — `fsm::step` (`step.rs:120`) — and always sets `at: now`
unconditionally. The actor always calls `step(.., Instant::now())` and always wraps
`result.transition_record` into the emitted `RawTransitionRecord`. There is no
constructor path that omits `at`. So the instant is a guaranteed field on every ledger
record (subject to Q8: replace the hardcoded `Instant::now()` with the injectable clock so
that guaranteed instant is also deterministic in tests).

---

## Q8 — Time-based behavior (ACK timeout, 5 s cooldown): pure-step layer vs clock seam. ("Not clear.")

**Clarification: the pure step is already time-as-input (good — keep it). The only gap is
that the *actor* hardcodes `Instant::now()`. Add a small `Clock` seam at the actor so that
single call site becomes injectable. Do both, layered.**

What "time-based behavior" means here:
- **5 s cooldown**: `operational_warning_recovery_ready(began_at, now, ctx)` in
  `transition_map.rs` compares `now - began_at` against `RPM_STRESS_DURATION_THRESHOLD_SECS`.
- **ACK timeout**: the headlamp assembly marks `TimedOut` on `TimerTick` using a deadline
  vs `now`.

Both ultimately depend on the `now` that the **actor** supplies via
`fsm::step(&state, &ctx, &evt, std::time::Instant::now())`. The *pure* layer never reads
a clock — it receives `now`. So:

- **"Keep at the pure-step layer"** = never read the clock inside pure code; the caller
  passes `now`. Already true for `step`. This is the functional core; preserve it.
- **"Add a clock seam"** = introduce `trait Clock { fn now(&self) -> Instant; }`, inject
  it via `VehicleControllerRuntimeOptions` (default = real monotonic clock; tests = a
  manually-advanceable fake). The actor calls `self.clock.now()` instead of
  `Instant::now()` — feeding both `step`'s `now` and the record's `at`.

Recommendation: do both. Keep the pure core as-is; add the seam at the imperative shell.
Payoff: the 5 s cooldown and ACK timeout become testable by *advancing a fake clock +
sending `TimerTick`*, instead of `tokio::time::sleep(FRONT_HEADLAMP_ON_ACK_WAIT + ...)` as
the current timeout e2e does. It also makes `at` deterministic (ties to Q7).

Actorification angle: when actuation/timeout move to child actors driven by timers, a
shared injectable clock keeps timeout logic deterministic across the refactor. Do this
seam *now*; it pays off immediately and survives the split.

---

## Q9 — "Once per FSM event (after state persist, before actions run)": actions must not change current state. Are we ready for message-to-self? Should the journey capture it?

**Verdict: the current ordering is correct and deliberate — the pure `step` is the *only*
place state changes; actions are effects, not mutators. You are actually *more* ready for
message-to-self than the question implies: a self-sent event is just another mailbox
message that produces its own cut. What's missing is *causality*, not the cut itself.**

Actor loop ordering today (`virtual_car_actor.rs::handle`):
1. `step()` (pure) →
2. **persist** `current_state` / `context` →
3. **emit transition record** →
4. **run actions** (`actuation_manager.execute`, async) →
5. emit diagnostics.

Two things follow:
- Because the record is emitted at step 3 reflecting the persisted state, an action at
  step 4 must **not** mutate `current_state` in place — that would desync from the
  already-emitted record. The design *structurally prevents* this: `execute()` takes
  `&DigitalTwinCar` (immutable); only the `EnterMode` hint is handled inline and it merely
  sets a local `mode` (currently `let _ = mode;`). Good — keep this invariant.
- **Message-to-self is compatible with the model.** A self-sent (or feedback) event is a
  *new* mailbox message → a *new* `handle()` call → a *new* `step` → a *new* record with a
  *new* `sequence_no`. The natural FSM progression via actuation feedback
  (`FrontHeadlampOnAck`, etc.) already works this way today. What you are correctly *not*
  doing is **synchronous in-handler re-entrancy** (an action recursively re-running the
  FSM for the same event). That distinction is the real content of "we are not ready for
  message-to-self": you are ready for *asynchronous* self-messages; you are (rightly)
  avoiding *synchronous* re-entrancy.

Does the journey capture it? **The *consequence* yes, the *causality* no.** Each resulting
event already gets its own cut. What's not captured is the link "transition N's action
emitted command C, whose feedback caused transition M." To capture that, thread a
`CorrelationId` from the recorded action (Q4) through the `ActuationCommand` and back via
the feedback event, and record it on both ends. Then the journey becomes a **causal DAG**
rather than a flat list — and that's exactly the property you want once actuation lives in
a separate, concurrently-running child actor (feedback may interleave;
`sequence_no` + `correlation_id` keep it sortable and causally linkable).

Rule to enshrine for actorification: **child actors never mutate parent state; they only
feed results back as new events.** The parent's pure `step` stays the sole state mutator.

---

## Work-item ledger (traceability)

Stable IDs so any future change can be mapped back to the decision that drove it. One
work-item = one focused commit, subject prefixed with the ID (e.g.
`WI-7a: rename TransitionRecord -> RawTransitionRecord ...`). Keep unrelated edits
(`.gitignore`, `README.md`, plan files) out of work-item commits.

| ID | Title | Questions | Status | Depends on |
|----|-------|-----------|--------|------------|
| WI-1 | Record emitted actions in `RawTransitionRecord` | Q4, Q5, Q9 | pending | WI-7a |
| WI-2 | Public pure state-law checker + journey fold | Q6 | pending | — |
| WI-3 | `Clock` seam in runtime options | Q7, Q8 | pending | — |
| WI-4 | `as_of_seq` snapshot stamp + Counter-A session/epoch | Q3, Q7 | pending | WI-7b |
| WI-5 | Reclassify `LogWarning` toward diagnostic sink | Q5 | pending | — |
| WI-6 | Test helper `(controller, actuation_rx)` + ack-injection | Q2 | pending | — |
| WI-7a | Type renames `TransitionRecord`→`RawTransitionRecord`, old `RawTransitionRecord`→`PublishedTransitionRecord` | Q4, Q7 | **DONE (uncommitted)** | — |
| WI-7b | Field rename `PublishedTransitionRecord.sequence_no`→`record_seq` (a1: leave `CorrelationId` as-is) | Q4, Q7 | pending | WI-7a |
| WI-8 | Single-writer ledger actor owns `record_seq` | Q7 | actorification | WI-7b |
| WI-9 | Correlation IDs end-to-end (action→command→feedback→record) | Q4, Q9 | actorification | WI-1, WI-7b |
| WI-10 | State-transition diagnostics as a projection of the ledger | Q1 | actorification | — |
| WI-11 | Move buzzer/egress I/O into actuation child actor | Q5 | actorification | WI-1 |

### WI-7b scope (next, agreed a1)

Rename **only** the ledger field; do not touch `CorrelationId` or any
`vehicle_device_bus` wire `sequence_no` (that's the command/wire axis).
- `transition_sink.rs` — `PublishedTransitionRecord.sequence_no` → `record_seq`.
- `virtual_car_actor.rs` — generator `next_sequence_no` → `next_record_seq`; struct init uses `record_seq`.
- `gateway_runtime.rs:59` — `record.sequence_no` → `record.record_seq` (leave `:194` `payload.sequence_no`, that's CAN wire).
- `test/actor_contract.rs:45,51` — `.sequence_no` → `.record_seq`.
- Acceptance: workspace builds; `cargo test -p common` green.

Deferred to WI-4 (per (b)): Counter-A session/epoch.

## Consolidated recommendation, ordered by "do now" vs "do with actorification"

Cheap & high-value **now** (independent of the actor split, and they make the split safer):

1. **Record emitted actions in `TransitionRecord`** (Q4) — unlocks Q5, feeds Q6/Q9.
   Filter/relocate `EnterMode`.
2. **Expose pure, public state-law checker** + journey-fold helper (Q6). Visibility bump +
   one entry point.
3. **Add a `Clock` seam** to `VehicleControllerRuntimeOptions`; replace the lone
   `Instant::now()` in the actor (Q7, Q8). Default real clock; fake for tests.
4. **Stamp snapshots (and keep `sequence_no` on records) with `as_of_seq`** (Q3, Q7) so
   staleness and ordering are explicit and reconcilable.
5. **Reclassify `DomainAction::LogWarning` toward the diagnostic sink** (Q5) — small.
6. Add a **test helper** `(controller, actuation_rx)` + ack-injection (Q2).
7. **Rename for clarity** — must land *before* actions+correlation share one record
   (Q4, Q7).
   - [DONE] Type: `TransitionRecord` (step.rs, the pure step output) →
     **`RawTransitionRecord`**.
   - [DONE] Type: old `RawTransitionRecord` (transition_sink.rs, the identity+seq wrapper
     emitted to the sink) → **`PublishedTransitionRecord`**. The channel/options types
     followed (`transition_tx: Sender<PublishedTransitionRecord>`); the sink trait keeps
     its name `TransitionRecordSink` but now takes `PublishedTransitionRecord`. Workspace
     builds; all 71 `common` tests green.

   Resulting scheme (now in code): `RawTransitionRecord` = raw pure-step fact (`at`,
   event, states, ctx); `PublishedTransitionRecord` = `{ car_identity, sequence_no,
   transition: RawTransitionRecord }` published to the sink.

   **Still to finalize (TODO 7):**
   - [ ] Field: `PublishedTransitionRecord.sequence_no` → **`record_seq`** (`ledger_seq`),
     leaving `CorrelationId` as the command axis (`command_seq`).
   - [ ] Give Counter A a **session/epoch** (mirrors `CorrelationId.session_id`) so
     restarts don't reuse `record_seq` ambiguously (ties to `as_of_seq`, Q3).

Better done **with actorification** (shape becomes clear once children exist):

8. **Single-writer ledger actor** owning Counter A (`record_seq`); child actors *message*
   it rather than minting ledger numbers, preserving a true total order as more actuators
   appear (Q7). Correlation counters stay per-source on each child.
9. **Correlation IDs end-to-end**: recorded action → `ActuationCommand` → feedback event →
   resulting record (Q4, Q9). Turns the journey into a causal DAG across actor boundaries.
10. **Make state-transition diagnostics a projection** of the transition ledger via an
    observer/telemetry actor, instead of the parent emitting them directly (Q1). Parent
    keeps emitting only non-transition diagnostics; child actors share the diagnostic bus.
11. **Move connector I/O for buzzer/egress into the actuation child actor** (the existing
    `TODO(actuation-child-actor)` / `TODO(actuation-egress)` arms), now backed by the
    recorded-action ledger so behavior stays observable through the transition.
