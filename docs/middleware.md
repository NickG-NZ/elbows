# Robot middleware

Design for our own real-time robot runtime, which will replace lerobot's control loop. It targets
embedded Linux (PREEMPT_RT). The first slice is built in `elbows_rt/` (see Status). It builds on
[`realtime-control.md`](realtime-control.md), which defines the plan format, validation and watchdog.
This doc covers the processes, threads, shared memory, pub/sub and drivers that carry it.

Language: **Rust.** It is memory-safe in a long-running real-time process, cross-compiles to aarch64
with cargo, catches more agent mistakes at compile time, and can be called from Python through PyO3.
Model inference stays in a separate process (Python first, then ONNX Runtime/TensorRT), so the
language choice does not limit which models we can run.

## Processes

```
┌─────────────────────────── robotd (owns all hardware) ───────────────────────────┐
│                                                                                  │
│  driver threads (one per blocking fd)          RT thread (SCHED_FIFO, own core)  │
│  ┌──────────────┐                              every tick:                       │
│  │ camera (later)│──publish──▶ [topics] ──────▶ 1. read inputs (latest / drain)  │
│  │ IMU (later)   │──publish──▶ [topics] ──────▶ 2. state estimation              │
│  └──────────────┘                              3. sample active plan at now      │
│                                                4. low-level control + limits     │
│  cyclic devices (exchanged inside the tick) ◀─▶ 5. bus I/O: servos read/write    │
│  Feetech bus                                   6. publish state, timing          │
│                                                                                  │
│  supervisor thread: setup, health, logging, bridges to other processes           │
└──────────────▲──────────────────────────────────────────────────┬────────────────┘
               │ plan topic (iceoryx2)                            │ state, frames (iceoryx2)
┌──────────────┴──────────────────────────────────────────────────▼────────────────┐
│ policy process (non-RT): obs → action chunk → Plan.  Python first; may be remote. │
└──────────────────────────────────────────────────────────────────────────────────┘
```

- **`robotd` is the only process that touches hardware.** If the policy crashes, restarts or stalls,
  the RT thread holds position and then stops, as `realtime-control.md` describes.
- **The policy process can be replaced.** It reads state and frames and writes plans. It may be
  Python (via bindings to the topic API), Rust with `ort`, or a bridge to a remote GPU.
- Later, drivers we don't trust (vendor camera SDKs) can move to their own processes behind the same
  topic API, without changing the controller.

## The RT thread

### Trigger

```rust
trait Trigger {
    /// Block until the next cycle. Returns the cycle's monotonic timestamp and whether we were late.
    fn wait(&mut self) -> Tick;
}
```

- **`ClockTrigger` is the default.** It uses `clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME)` on an
  absolute deadline (`t_next += period`), so error never accumulates. It never uses wall-clock time.
- **`DeviceTrigger` comes later.** It blocks on an fd such as an INS data-ready GPIO line
  (`gpio-cdev` line events) or a sensor's sample stream, with a timeout of about 1.5 periods. A
  missed trigger is counted, and the cycle falls back to the clock, so a dead sensor can't freeze
  the controller.
- The loop only sees `Tick`. Changing the trigger changes nothing else.

### One cycle

1. **Read inputs** from in-process topics. It takes the latest value for most data and drains every
   queued sample for sensors such as the IMU, because the estimator needs all of them.
2. **State estimation.** This starts as joint positions/velocities from the servos. IMU fusion comes later.
3. **Sample the plan** at the tick time (`t0 + k·dt` interpolation, blending, expiry).
4. **Low-level control**, then limits, then the safety check (watchdog, joint/velocity limits).
5. **Cyclic bus I/O:** write commands and read servo state.
6. **Publish** state, commands and cycle timing (start, end, overrun) without blocking.

### Real-time rules

- PREEMPT_RT kernel (mainline since 6.12), `isolcpus`/`nohz_full` on the RT core, `SCHED_FIFO`,
  `mlockall(MCL_CURRENT | MCL_FUTURE)`, and a prefaulted stack.
- **The RT thread does not allocate, take locks, make syscalls other than the bus fd and the
  trigger, or log.** Enforce this in debug builds with an allocator that panics on allocation in the
  RT thread (e.g. `assert_no_alloc`).
- Overruns are counted and published. N consecutive overruns trigger a protective stop.
- The rate is set in config. USB serial limits us to about 100–500 Hz to start (see Drivers).
- On macOS/dev machines, the same code runs with normal scheduling, for testing and simulation.

## The supervisor thread

The supervisor is `robotd`'s non-RT control plane. It is one normal-priority thread running an
`epoll` loop over a `signalfd`, the session socket, a `timerfd` for health checks and the `eventfd`s of
the topics it watches. Anything that blocks for a long time (disk, network) gets its own thread,
which the supervisor starts and watches.

- **Bring-up, in order:** load the setup, find devices, open transports, ping and configure them
  (limits, gains, servo return delay, torque off), create topics, start driver threads, set up
  the RT thread, then start the loop.
- **Modes:** Init → Idle (torque off) → Hold → Running → Stopping → Fault. Requests arrive on a
  command topic. The supervisor validates them and the RT thread applies them at a cycle boundary.
- **Policy sessions** over a Unix socket (see Liveness below). It hands out the single plan-writer
  role and frees it on disconnect.
- **Health:** heartbeats from each driver thread, device status flags (overtemperature, overload,
  voltage), cycle timing, queue drop counters, USB unplug events, board temperature and throttling.
  It escalates in stages: warn, then degrade, then fault.
- **Driver lifecycle:** it restarts failed `StreamDevice`s with backoff. A failed `CyclicDevice`
  means Fault, because it can't be swapped out under the running loop.
- **Recording:** a logger thread drains the telemetry and state queues to disk (MCAP, which
  Foxglove can open). This is also the source of episodes for `elbows_learn`.
- **Bridges:** start and watch the Zenoh bridge that sends selected topics off the machine.
- **Shutdown:** on `SIGINT`/`SIGTERM` it requests a stop, waits for Hold, turns torque off, joins
  threads and releases its topics.
- **systemd watchdog:** it sends `WATCHDOG=1` only while the RT heartbeat is advancing, so systemd
  restarts a hung `robotd`.
- **Operator CLI:** `robotctl` talks to it over the same socket (status, enable, stop).

**The supervisor requests and the RT thread decides.** Safety never depends on the supervisor. If it
hangs or dies, the RT thread still enforces limits and holds, then stops when plans go stale. The
RT thread can always enter Stop by itself.

## Messaging

**Decision: iceoryx2 carries all messaging on the robot's host, including in the RT loop.** These
are research robots, so a well-designed dependency in the RT path is acceptable. In exchange we get
zero-copy pub/sub with fan-out to several subscribers, discovery, events and dead-process cleanup,
instead of writing them ourselves.

- **Between processes:** the `ipc` service variant (shared memory).
- **Between threads in `robotd`:** also `ipc` for now, so tools like `robotctl` can read the same
  topics. iceoryx2's `local` variant (process memory, same API) is there if we need it.
- **Off the host:** Zenoh (see below). This is planned: remote inference, monitoring and teleop will
  need it.

### Our layer on top

A thin `Topic` wrapper (`rt-ipc`). Drivers, the controller and policies use only this API and never
import iceoryx2 directly, so the backend can be swapped.

```rust
// Declared once in code, next to the message types. Name, type and pattern travel together.
pub const PLAN:   Topic<Plan>     = Topic::latest("plan");
pub const STATE:  Topic<ArmState> = Topic::latest("state");
pub const EVENTS: Topic<Event>    = Topic::queue("events", 64);

let pub_ = node.publisher(&STATE)?;           // created at setup: all memory allocated here
pub_.publish(&state)?;                        // in the loop: no allocation, never blocks
let plans = node.subscriber(&PLAN)?;
if let Some(p) = plans.latest()? { ... }      // RT side: poll once per cycle
sub.drain(|sample| est.update(sample))?;      // queues: every unread sample, in order
```

- **Topics are defined in code** (a `topics` module beside the messages), so a wrong name or type
  is a compile error. The Python bindings expose the same constants.
- **The setup file holds the deployment:**
  - the robot namespace (`so101/plan`), so two robots on one host never collide;
  - device instance names (`[imu_wrist]` gives `imu_wrist/sample`; the type is fixed by the driver);
  - rates and buffer depths;
  - which topics are bridged off the host and which are recorded.
- No runtime remapping: we control both ends.
- If a language other than Rust or Python ever needs the messages, move them to a schema file and
  generate code per language (as PX4 does with `.msg` files).

### Patterns

| Data | Pattern | Why |
|---|---|---|
| Plan (policy → RT) | Latest | Only the newest matters; it is time-indexed |
| Robot state (RT → policy, logger, viewer) | Latest | Each subscriber wants the current state |
| IMU / INS samples (driver → RT) | Queue (oldest dropped when full) | The estimator needs every sample in order |
| Camera frames | Latest, zero-copy | Large; stale frames are worthless |
| Commands/events (enable, mode, stop request) | Queue, never silently dropped | Each one must be handled |
| Telemetry, logs, cycle timing | Queue, overflow counted | Must never block the RT thread |

How `rt-ipc` maps these onto iceoryx2 (0.10), checked against its source and tested:

- Every topic: safe overflow on (a full subscriber buffer drops its oldest sample), publisher
  backpressure `DiscardData`, so **publishing never blocks** on a stalled subscriber. One publisher
  (`max_publishers(1)` enforces single writer), up to 8 subscribers, each with its own buffer.
- "Latest": subscriber buffer of 1 plus history of 1, so a late subscriber gets the last value.
- "Queue": subscriber buffer of `depth`, no history. Drops are not counted yet.
- Ports on the `ipc` variant use no locks (single-threaded policy), so they are `!Send`: each thread
  creates its own node and ports during setup. The RT thread creates its own before the loop.
- `publish` and `receive` don't allocate. They may make syscalls when a peer connects or
  disconnects. Accepted for research robots; measure on the target.

### Plans are latest-value, not a queue

A queue is the wrong shape for plans. It delivers every chunk in order, but a newer plan always
supersedes an older one. If the policy produces two plans while the controller is busy, we want the
second, not both.

- **Plans expire by time, not because a buffer fills up.** Each plan covers `[t0, t0 + n·dt]`. Past
  waypoints are skipped. Past the end, the controller holds. A plan is also rejected if its
  `obs_stamp_ns` is too old, because it was computed from stale state.
- **Plan contents are untrusted input.** The RT side copies each plan out and validates it
  (`n ≤ N_MAX`, finite values, limits, jumps) before using it, as `realtime-control.md` describes.
- Messages are `#[repr(C)]` with fixed-size arrays and no pointers, `Vec` or `String`, and
  implement iceoryx2's zero-copy marker trait.

### Liveness

- **Message timestamps decide safety.** Every message carries a `CLOCK_MONOTONIC` stamp. The RT
  thread judges the policy only by whether fresh, valid plans arrive. It doesn't care whether the
  policy crashed, hung or is slow: it holds either way.
- **iceoryx2 cleans up dead nodes** when another node is created or a topic is opened. Tested: after
  a policy is SIGKILLed, a new one can take the single plan-publisher slot. A second `robotd` for the
  same robot is refused (the state topic already has its publisher). A Unix socket session (with
  `robotctl`) remains the plan for prompt disconnect events. Not yet tested: `robotd` restarting
  under a running policy.
- **The RT thread creates its ports and locks memory before the loop starts.** Nothing is created or
  connected inside the loop.

### Off the host: Zenoh

Remote inference on a GPU machine (or Modal), monitoring and teleop from a laptop, recording
elsewhere, and multiple robots all need traffic to leave the robot. A bridge process republishes
selected topics (listed in the setup file) over Zenoh, using the per-stream link choices in
`realtime-control.md`. The bridge sits behind the same topic API, so nothing in `robotd` changes. To
evaluate when we build it: iceoryx2's own Zenoh tunnel versus a bridge of our own.

### First prototype

Before this decision, `rt-ipc` was a hand-written latest-value triple buffer and SPSC ring in shared
memory, one writer and one reader each. It was replaced before being committed, because it had no
fan-out, discovery or dead-process cleanup.

## Drivers

Two kinds of device, because they relate to the loop differently:

```rust
/// On the control bus; exchanged inside the RT cycle, at a fixed phase each tick.
trait CyclicDevice {
    fn read(&mut self, now: Tick) -> Result<(), BusError>;   // fill state
    fn write(&mut self, now: Tick) -> Result<(), BusError>;  // send commands
}

/// Streams on its own; owns a thread blocking on its fd and publishes to topics.
trait StreamDevice: Send {
    fn run(&mut self, bus: &Bus, stop: &StopFlag) -> Result<(), DeviceError>;
}
```

- **Transports are separate from protocols.** Use a `Transport` trait (USB serial now, SocketCAN,
  RS-485 or EtherCAT later) beneath a protocol driver (Feetech now). Swapping the bus adapter then
  doesn't mean rewriting the servo driver.
- **Feetech servos (first driver, not yet written)** are a `CyclicDevice`. Each tick sends one
  sync-write for all goal positions and one sync-read for positions, velocities and loads. See
  First hardware below.
- **Cameras and IMUs (later)** are `StreamDevice`s. Each gets one thread per blocking fd at a
  priority set in config. They stamp each sample with `CLOCK_MONOTONIC` at capture (or with the
  device's hardware timestamp, mapped to it) and publish. An INS can also provide the
  `DeviceTrigger`.
- **Configuration** comes from the robot's `hardware/setups/<setup>/setup.toml` (one table per device),
  shared by `robotd`, `robotctl` and calibration tools. `rt-devices` finds a device's port from its
  USB serial number (`robotctl devices list/port`).
- **A sim device** (later) implements `CyclicDevice` by stepping MuJoCo. The same loop, timing and
  policy interface can then run against `elbows_sim` before touching hardware.

## First hardware: SO-101 on the Waveshare bus adapter

The SO-101 uses Feetech STS3215 servos on a Waveshare Bus Servo Adapter (A). These specs come from
memory and must be checked on the hardware.

- **The adapter is a plain bridge with no MCU.** It has a WCH CH343 USB-serial chip (USB-C, the
  `cdc_acm` driver, `/dev/ttyACM*`) and a half-duplex TTL line driver, at up to 1 Mbaud. All
  control runs in `robotd`.
- **Timing:** on the wire, a 6-servo sync-write plus a sync-read takes well under 1 ms at 1 Mbaud.
  USB full-speed transfers dominate, at roughly 1 ms or more each way, which puts a realistic
  ceiling around 200–300 Hz. lerobot runs at 30 Hz. Measure the round trip on the target board
  before fixing the rate.
- **Bring-up sets the STS return delay to 0.** The default is about 500 µs per reply.
- **No safety below `robotd`.** As far as we know, neither the adapter nor the STS3215 has a
  communication timeout, so if `robotd` dies the servos hold their last goal with torque on. **The
  e-stop must cut servo power** (a switch or relay on the adapter's DC input). Longer term, put our
  own MCU board between the PC and the bus. It can run a comms watchdog and a deterministic inner
  loop, and talk to the host over CAN or Ethernet.

## Proposed layout

```
elbows_rt/                 Cargo workspace
  crates/
    rt-core/               Tick, Trigger, RT thread setup, timing stats
    rt-ipc/                Node/Publisher/Subscriber over iceoryx2 (topics are declared in rt-core)
    rt-devices/            CyclicDevice/StreamDevice/Transport traits; drivers added per device
    robotd/                the main binary: config → devices → RT loop → supervisor
    rt-py/                 PyO3 bindings: attach to topics, write plans, read state
```

## Status

Built in `elbows_rt/` and tested on macOS (no RT scheduling there), not yet on the robot:

- `rt-ipc`: typed topics over iceoryx2 (fan-out, history, drop-oldest, single publisher; tested).
- `rt-core`: messages and topic declarations, `ClockTrigger`, RT thread setup, cycle stats, plan
  validation and sampling.
- `rt-devices`: the traits only.
- `robotd`: supervisor (signals, stall detection, status) plus the RT loop following plans with no
  devices. `plan-demo` stands in for a policy.
- End to end at 500 Hz on a laptop: plans followed with none rejected; Hold when the policy is
  killed; a replacement policy takes over; a second `robotd` is refused. Wake-up latency p99
  < 0.5 ms, max ~2 ms, without RT scheduling.

Next: the Feetech driver, the session socket and `robotctl`, Python bindings (`rt-py`), plan
blending, recording, the device trigger and the Zenoh bridge.

## Open questions

- **Servo-side timeout.** Confirm whether the STS3215 firmware has any communication timeout.
- **Own MCU board** between the PC and the servo bus: when, and which host link (CAN, Ethernet,
  USB).
- **First target board** (Jetson Orin vs a Pi-class board), which sets the kernel and toolchain
  setup.
- **Control mode** for Feetech: position-only to start, or current/torque where the servo supports it.
- **Bring-up order:** whether we build the RT loop against the sim device before the Feetech driver.
