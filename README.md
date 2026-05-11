# SDV simulation (draft)

Rust workspace that simulates a small **vehicle data path** inspired by **VSS (Vehicle Signal Specification)** ideas: telemetry is modeled as named signals, encoded on a **SocketCAN** bus, and consumed by a **gateway** that runs a simple **finite-state machine** over asynchronous events.

This is a hands-on learning / demo project, not production software.

## Requirements

- **Linux** with SocketCAN (typical for `vcan` or real CAN hardware).
- **Rust** toolchain compatible with the workspace (edition 2024).

## How To Run (Linux quick start)

`vcan0` is the default and currently preferred interface because both binaries are wired to it in code.

Run these setup commands first (requires `sudo`):

```bash
sudo modprobe vcan
sudo ip link add dev vcan0 type vcan
sudo ip link set up vcan0
```

Then start the apps in two terminals (no `sudo` needed):

```bash
# Terminal A — producer
cargo run -p emulator
```

```bash
# Terminal B — consumer / gateway
cargo run -p gateway
```

When done, stop both with `Ctrl+C`, then tear down `vcan0` (requires `sudo`):

```bash
sudo ip link del vcan0
```

If you use a different interface name, update the hardcoded interface strings in `crates/emulator/src/main.rs` and `crates/gateway/src/gateway_runtime.rs` (see `DEFAULT_CAN_INTERFACE`).

## What You Should See (outputs)

- **Terminal A (`emulator`)** prints startup and continuous debug lines with speed, RPM target tracking, and ambient lux while publishing all three as CAN telemetry.
- **Terminal B (`gateway`)** prints startup output, state transition logs, and timestamped action/alert logs from the controller runtime while consuming CAN frames from `vcan0`. When lighting requests are emitted, after a short simulated delay you may also see `[actuation-ingress @ corr …]` lines for corner-light ON/OFF acknowledgements (see Known Demo Behaviors).
- Heartbeat (`TimerTick`) log lines are **off by default** and can be enabled with `--print-timer-tick`.
- Both processes are long-running by design; stop with `Ctrl+C` when done.

### Paste Terminal A output here (`bash`)

```bash
# (Milestone-1)
cargo run -p emulator
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.17s
     Running `target/debug/emulator`
🚀 Emulator active on vcan0. Simulating VSS telemetry...
DEBUG: Time=33s | SpeedKph=40.46 | RPM=6388 (Target=6500) | AmbientLux=136
DEBUG: Time=33s | SpeedKph=40.24 | RPM=6399 (Target=6500) | AmbientLux=0
DEBUG: Time=34s | SpeedKph=39.98 | RPM=6405 (Target=6500) | AmbientLux=0
DEBUG: Time=34s | SpeedKph=39.71 | RPM=6410 (Target=6500) | AmbientLux=17
```

### Paste Terminal B output here (`bash`)

```bash
# (Milestone-1)
cargo run -p gateway
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.05s
     Running `target/debug/gateway`
Physical Car name: NASHIK-VC-001, initializing its Digital Twin ...
[ACTION @ 14:19:28 103912443]: 📡 Publishing to Cloud: Idle
⚡ Gateway on vcan0 — CAN → VehicleEvent → PhysicalCarVocabulary → DigitalTwinCarVocabulary → VirtualCarActor
[NASHIK-VC-001]: Transitioned to Idle
[ACTION @ 14:19:30 210212284]: 📡 Publishing to Cloud: Driving
[NASHIK-VC-001]: Transitioned to Driving
[ACTION @ 14:19:32 210389826]: 🔊 BUZZER ON - High Stress Detected!
[ALERT @ 14:19:32 210429121]: Overspeed detected!
[NASHIK-VC-001]: Transitioned to Warning(Instant { tv_sec: 56863, tv_nsec: 499413717 })
[ACTION @ 14:19:34 013528967]: 💡 Requesting front corner lights ON.
[actuation-ingress @ corr CorrelationId { source_id: "...", session_id: 0, sequence_no: 1 }]: corner lights ON acknowledged (non-CAN path)
[ACTION @ 14:19:45 355467066]: 🔇 BUZZER OFF - System Normal.
[NASHIK-VC-001]: Transitioned to Driving
```

*(The `CorrelationId` fields in the actuation line above are illustrative; the runtime prints the real `Debug` value from the controller.)*

### Runtime notes

- Gateway heartbeat lines are optional:
  - default: `cargo run -p gateway` (quiet timer ticks)
  - verbose: `cargo run -p gateway -- --print-timer-tick`
- Action/alert logs include timestamps (`HH:mm:ss nsec`) to measure gaps between transitions and effects.

### Known Demo Behaviors (must-keep section for each demo milestone)

**Maintainers:** whenever this milestone adds or changes **user-visible** gateway or emulator output (new log lines, flags, or timing), update **this subsection** and, if the canned terminal samples above no longer match reality, refresh the **“Paste Terminal A/B output”** blocks in the same README revision.

- Lighting actions are usually less frequent than RPM/warning transitions because ambient-lux tunnel events are probabilistic while RPM target flips are periodic.
- Corner-light ON/OFF request emission is idempotent; repeated ON/OFF request lines are intentionally suppressed while in pending lighting states until an ACK is applied.
- **In-process actuation ACK (this milestone):** the gateway does **not** receive lighting ACKs from `vcan0`. Two Tokio tasks simulate a minimal plant and a second ingress path: commands go out on an internal `ActuationCommand` channel; after a fixed delay (~150 ms, see `gateway_runtime`), “confirmed” feedback is turned into `PhysicalCarVocabulary::{CornerLightsOnConfirmed, CornerLightsOffConfirmed}` and submitted through the same `submit_physical_car_event` path as CAN-derived telemetry. Console lines prefixed with `[actuation-ingress @ corr …]` are that path; wiring lives in `crates/gateway/src/actuation_scaffold.rs` and `gateway_runtime.rs`.
- Gateway output is action/transition-centric by default; it does not print every incoming telemetry frame unless explicitly instrumented.
- TimerTick heartbeat logs are disabled by default and only shown when gateway is started with `--print-timer-tick`.

## Current Architecture (milestones)

- **Scope of this workstream** (`localized_AmbientLight_actuator_facility`): ambient-lux-driven corner-light **intents**, a **localized** in-process **actuator simulation** (command → delayed feedback → `PhysicalCarVocabulary` ACK → same controller ingress as telemetry), and a **thin gateway** layout (`main` → `gateway_runtime` + `actuation_scaffold`). It is **not** a repo-wide “ingression facility” rewrite; CAN→`ingress` mapping for speed/RPM/lux stays the primary bus path, and other future ingress can reuse the same `submit_physical_car_event` boundary.
- Current milestone demonstrates that ambient-lux path plus lighting command emission on top of the established FSM + controller runtime split.
- **Gateway step (3) — done (checked in), within the scope above:** `crates/gateway/src/main.rs` is CLI-only (e.g. `--print-timer-tick`). `gateway_runtime.rs` owns controller install, demo `send_power_on`, the `TimerTick` loop, and the CAN read loop. `actuation_scaffold.rs` holds the small helpers that spawn the in-process ACK **plant** (command → delayed feedback) and **feedback ingress** (feedback → `PhysicalCarVocabulary` → `submit_physical_car_event`).

## Architecture And Design

### Gateway behavior (high level)

- **Context** (`VehicleContext`) holds latest RPM/speed/ambient-lux and health flags used by FSM guards.
- **Events**: `PowerOn`, `PowerOff`, `UpdateRpm`, `UpdateSpeed`, `UpdateAmbientLux`, and periodic `TimerTick`.
- **FSM spec**: `transition(...)` + `output(...)` in `common::engine::op_strategy::transition_map` are the canonical transition/action rules.
- **Execution wrapper**: `step(...)` in `common::fsm::step` derives context from event payload, calls `transition/output`, and returns `StepResult`.
- **Time handling**: `transition(...)` takes `now` explicitly (no hidden clock calls), which keeps time-based behavior deterministic in tests.
- **Warning recovery**: `Warning(began_at)` is recovered on `TimerTick` only when cooldown elapsed and RPM is at/below recovery threshold; recovers to `Driving` or `Idle` based on speed.
- **Lighting behavior**: lux hysteresis (`LUX_ON_THRESHOLD` / `LUX_OFF_THRESHOLD`) emits `RequestCornerLightsOn/Off` intents from `step(...)` based on `LightingState`.
- **Transition sink**: actor can emit raw transition records through `TransitionRecordSink` (best-effort, warn-and-continue on sink full/closed).
- **Actuation boundary**: runtime executes `DomainAction` through `DefaultActuationManager`; FSM remains intent-only.
- **Layered ingress path**: gateway maps `VehicleEvent` to `PhysicalCarVocabulary`, then projects through `PhysicalToDigitalProjector` before sending to the runtime controller.

### What it does

1. **Emulator (`emulator`)** — Acts like a minimal “virtual ECU”: model components update speed/RPM/ambient-lux, encode them as `VssSignal`, and **write standard CAN frames** to Linux CAN (`vcan0` by default).
2. **Gateway (`gateway`)** — Opens the same interface, **reads CAN frames**, decodes known frames into `VssSignal`, maps ingress to `PhysicalCarVocabulary`, projects to `DigitalTwinCarVocabulary`, and sends to controller runtime. A Tokio loop sends periodic `TimerTick` heartbeat events. A separate in-process pair of tasks simulates actuator ACK delivery on channels (not on CAN); see Known Demo Behaviors.
3. **Common library (`common`)** — Shared types and behavior: VSS-style signals, physical/digital vocabulary contracts, projection adapters, strategy (`transition/output`), step contract (`step` + `StepResult`), and controller runtime with optional transition sink.

Together, the crates demonstrate: **encode → CAN → decode → domain events → stateful logic → runtime actuation intents**, mirroring core SDV control-path patterns.

### Crates

| Crate      | Role                                                                                                                                                                                         |
| ---------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `common`   | `VssSignal`, physical/digital vocabularies, projector adapters, op strategy (`transition/output`), step contract, controller runtime, default actuation manager, transition sink abstraction |
| `emulator` | Sends simulated speed/RPM/ambient-lux telemetry at ~10 Hz                                                                                                                                    |
| `gateway`  | CAN ingress + physical mapping + projection; `gateway_runtime` + `actuation_scaffold` worker wiring; thin `main`                                                                             |

### CAN mapping (concrete protocol in code)

Signals use **11-bit standard IDs** and **2-byte big-endian** payloads:

| Signal (concept) | CAN ID  | Payload                                           |
| ---------------- | ------- | ------------------------------------------------- |
| Vehicle speed    | `0x101` | `u16`, scaled: km/h × 100 (decode divides by 100) |
| Engine RPM       | `0x102` | `u16`, RPM as integer                             |
| Ambient lux      | `0x103` | `u16`, lux as integer                             |

Unknown IDs or non-standard frames are ignored by the ingress path (unless and until the decoder
is extended).

## FSM + Lighting Contract (current implementation and limits)

Corner-light actuation based on ambient light (`lux`) is implemented as an orthogonal context concern while preserving the top-level FSM (`Off`, `Idle`, `Driving`, `Warning`).

### Ambient Light Sensing and Acting in this version

#### Current goal coverage

The current code provides a **lighting sub-state** in context so that:

- low ambient light requests front corner lights ON,
- the system remains in a **pending** sub-state until an actuator acknowledgment event is received,
- repeated sensor updates do not spam actuator commands.

This keeps the current machine as an **extended FSM** (top-level state + orthogonal context), not a
full hierarchical state machine yet.

#### Scope and Non-Goals (current)

- Scope: sensor-driven corner-light control with pending-state safety and command idempotency.
- Non-goal: replacing the existing primary FSM state model.
- Non-goal: introducing a full multi-region/hierarchical statechart runtime.

#### Context Extension

- `lighting_state: LightingState`
- `ambient_lux: u16` (or equivalent normalized representation)

`LightingState`:

- `Off`
- `OnRequested`
- `On`
- `OffRequested`

#### Event Vocabulary

- `UpdateAmbientLux(u16)` — ambient sensor update from ingress path.
- `CornerLightsOnConfirmed` — actuator/body-controller ACK for ON.
- `CornerLightsOffConfirmed` — actuator/body-controller ACK for OFF.
- `CornerLightsActuationFailed` is not implemented yet.

#### Domain Actions

- `RequestCornerLightsOn`
- `RequestCornerLightsOff`
- `LogLightingInfo`/`LogLightingFault` are deferred.

#### Threshold Contract (hysteresis)

Use separate thresholds:

- `LUX_ON_THRESHOLD`
- `LUX_OFF_THRESHOLD` where `LUX_OFF_THRESHOLD > LUX_ON_THRESHOLD`

Reason: avoid rapid ON/OFF toggling near one boundary.

#### Transition Contract (lighting sub-state)

Given `lighting_state` and incoming event:

1. `Off` + `UpdateAmbientLux(lux <= LUX_ON_THRESHOLD)`  
   -> `OnRequested` + emit `RequestCornerLightsOn`
2. `OnRequested` + `CornerLightsOnConfirmed`  
   -> `On`
3. `On` + `UpdateAmbientLux(lux >= LUX_OFF_THRESHOLD)`  
   -> `OffRequested` + emit `RequestCornerLightsOff`
4. `OffRequested` + `CornerLightsOffConfirmed`  
   -> `Off`
5. `OnRequested` + repeated low-lux updates  
   -> stay `OnRequested` (no duplicate ON command)
6. `OffRequested` + repeated high-lux updates  
   -> stay `OffRequested` (no duplicate OFF command)
7. Failure/timeout retry policy is deferred to a later milestone.

#### Main FSM Interaction Policy

Lighting remains orthogonal to primary drive state:

- primary FSM (`Off`, `Idle`, `Driving`, `Warning`) continues to be the authoritative operational state;
- lighting logic runs in context as a secondary concern;
- when primary state is `Off`, effective lighting should be forced/kept `Off` (or ON requests blocked).

#### Behavioral Guarantees (contract-level invariants)

- ON request emits only from `LightingState::Off`.
- OFF request emits only from `LightingState::On`.
- Pending states resolve only via ACK/failure/timeout events.
- Duplicate sensor updates do not cause duplicate actuator requests.
- Existing warning/buzzer logic remains independent from lighting actuation.

#### Architecture Mapping (where this belongs)

- Signal encode/decode: `common::signals` (`VssSignal`)
- Ingress mapping to physical vocabulary: `gateway/src/ingress/mapping.rs`
- Physical to digital projector: `common::engine::connectors::PhysicalToDigitalProjector`
- FSM vocabulary/context/actions: `common::fsm::machineries`
- Transition and output rules: `common::engine::op_strategy::transition_map`
- Step boundary for context mutation + domain actions: `common::fsm::step`
- Side-effect execution and ACK ingestion path: `common::engine::controller::VehicleController` (currently aliased to `VirtualCarActor`)

#### Limitations (current and expected)

- Actuator ACK is simulated in-process (channels + Tokio tasks), not as real CAN ACK frames on `vcan0`; replacing that with bus-backed ACKs is future work.
- Timing/timeout policy is deliberately simple for simulation clarity.
- No formal concurrent-region statechart runtime yet; orthogonal behavior is represented through context.
- Determinism depends on explicit event ordering and `now` handling at the step boundary.
- Gateway does not print every incoming telemetry frame by default (focuses on transitions/actions).

## Dependencies (not exhaustive)

- **`socketcan`** — CAN sockets and frames on Linux  
- **`tokio`** (gateway) — async runtime, channels, timers  
- **`anyhow`** — convenient error handling in binaries  
- **`rand`** (common) — lightweight randomness for the virtual car  

## Future work (ideas)

- Configurable CAN interface via CLI or env  
- `spawn_blocking` (or dedicated thread) for blocking socket reads without stalling the async runtime  
- Richer VSS coverage, diagnostics, or recording  
- Standards-driven signal ingress contract (DBC/AUTOSAR-style source of truth for CAN IDs, payload scaling, and signal semantics), then project from that contract into `PhysicalCarVocabulary`  
- Emulator modeling notes/tutorial index (starting with `docs/rpm-model-tutorial.md`) for quick onboarding and review  
- Handcrafted emulator profile injection (test/demo/realistic) so scenario behavior is intentional and reproducible  
- Add a shared trait/template for device policies so new actuated devices (for example brake lights) follow a fixed implementation checklist  
- Add structured observability counters/log fields for ignored device responses (reason, device, correlation) to improve diagnostics and trend analysis  
- Add follow-on transport adapters (Zenoh/uProtocol) that reuse the existing per-device policy core and keep Twin/Controller contracts unchanged  

---

*This README is a **draft**; we will extend it as the codebase grows.*
