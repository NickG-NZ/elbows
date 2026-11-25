# Robot middleware

Design for our own real-time robot runtime, which will replace lerobot's control loop. It targets
embedded Linux (PREEMPT_RT). Nothing here is implemented yet. It builds on
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
               │ plan slot (shared memory)                        │ state, frames (shared memory)
┌──────────────┴──────────────────────────────────────────────────▼────────────────┐
│ policy process (non-RT): obs → action chunk → Plan.  Python first; may be remote. │
└──────────────────────────────────────────────────────────────────────────────────┘
```

- **`robotd` is the only process that touches hardware.** If the policy crashes, restarts or stalls,
  the RT thread holds position and then stops, as `realtime-control.md` describes.
- **The policy process can be replaced.** It reads state and frames and writes plans. It may be
  Python (via PyO3 bindings to the shared-memory API), Rust with `ort`, or a bridge to a remote GPU.
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
  (limits, gains, servo return delay, torque off), create segments, start driver threads, set up
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
- **Bridges:** Zenoh threads that send selected topics off the machine.
- **Shutdown:** on `SIGINT`/`SIGTERM` it requests a stop, waits for Hold, turns torque off, joins
  threads and removes segments.
- **systemd watchdog:** it sends `WATCHDOG=1` only while the RT heartbeat is advancing, so systemd
  restarts a hung `robotd`.
- **Operator CLI:** `robotctl` talks to it over the same socket (status, enable, stop).

**The supervisor requests and the RT thread decides.** Safety never depends on the supervisor. If it
hangs or dies, the RT thread still enforces limits and holds, then stops when plans go stale. The
RT thread can always enter Stop by itself.

## Shared memory between processes

### Chunks are a latest-value slot, not a ring buffer

**A ring buffer is the wrong shape for plans.** A ring queues every chunk and delivers them in
order, but a newer plan always supersedes an older one. If the policy produces two plans while the
controller is busy, we want the second, not both.

- **Plans expire by time, not because a buffer fills up.** Each plan covers `[t0, t0 + n·dt]`. Past
  waypoints are skipped. Past the end, the controller slows to a hold and then stops if the timeout
  passes. A plan is also rejected if its `obs_stamp_ns` is too old, because it was computed from
  stale state.
- **The plan slot is a triple buffer in shared memory** with three slots and one atomic state word.
  The policy writes into its back slot and publishes with one atomic swap. The RT thread takes the
  newest complete slot with one atomic swap. Neither side ever waits, retries or sees a half-written
  plan. A plan is a few KB (e.g. 50 waypoints × 7 DoF × f32 ≈ 1.4 KB).
- Robot state going the other way uses the same type of slot. Telemetry and logs, where every
  message matters, use rings (see the table below).

### Why no locks across processes

`realtime-control.md` allows a short lock read with `try_lock` *within* a process. Across processes
we use **only wait-free atomics**:

- If a process dies while holding a lock in shared memory, the lock stays held. Robust mutexes
  (`EOWNERDEAD`) work around this, but recovery is complex.
- A lock shared with a non-RT process invites priority inversion.
- A seqlock is the other common choice. Its reader retries without bound while the writer is busy,
  and the racy copy is undefined behaviour in Rust unless it is done word by word with atomics. The
  triple buffer has neither problem.

### Rules for every shared-memory segment

1. **Plain data only.** Use `#[repr(C)]` with fixed-size arrays, and no pointers, `Vec`, `String`
   or references. Derive `zerocopy`/`bytemuck` traits so the compiler proves this.
2. **A header checked on attach**: magic, ABI version, layout hash (type name, size, alignment,
   fields), and capacity. A reader refuses a mismatch instead of misreading the data.
3. **`robotd` creates every segment and owns its lifetime.** Segments live under
   `/dev/shm/elbows/<robot>/<topic>` with mode `0600`. Policy processes attach. One writer per slot
   is enforced with an `flock` on the segment, so a second policy can't silently interleave plans.
4. **Contents are untrusted input.** The RT side validates every plan (`n ≤ N_MAX`, finite values,
   limits, jumps) before using it, as `realtime-control.md` describes.
5. **Liveness is explicit.** Each slot has a heartbeat (sequence number and publish time) that the
   watchdog reads. A frozen writer is detected even if its last plan was valid.
6. **The RT side maps and `mlock`s segments at startup**, never inside the loop.

### Creating segments

- Open `/dev/shm/elbows/<robot>/<topic>` as a file, `set_len`, then `mmap(MAP_SHARED)`, using std
  plus `memmap2`. `shm_open` can't create subdirectories, which this layout needs. `memfd` with
  seals (`F_SEAL_SHRINK | F_SEAL_GROW`) is the stronger option if shrinking a mapped file (which
  causes SIGBUS) ever becomes a real risk. It needs the fd passed over the session socket.
- At startup `robotd` deletes its own segment directory and recreates each segment with
  `create_new` (`O_EXCL`). It prefaults with `populate()` (`MAP_POPULATE`) and calls `mlock`.
- The owner writes the whole header first and stores the **magic** (`b"ELBW"`) last, with release
  ordering. Attachers check the magic (acquire), then the version, layout hash and size, before
  touching the payload. Zeroing the magic on shutdown tells attachers to detach.
- Shared atomics must be lock-free and naturally aligned. Atomic values read from shared memory
  are untrusted: decode and range-check an index before using it. Slot data is copied out, then
  validated, and never used in place.
- On macOS (dev only), a file under `$TMPDIR` replaces `/dev/shm`.

### Liveness

The magic says a segment is ready. It says nothing about whether its creator is still alive.

- **Heartbeats decide safety.** Every writer bumps a sequence number and `CLOCK_MONOTONIC`
  timestamp in its slot. A reader that sees them stall treats the writer as gone. This catches
  hangs as well as crashes, and the RT thread can read it wait-free. The RT thread doesn't care
  why plans stopped arriving: it holds, then stops.
- **A Unix socket session handles events.** Each policy process connects to the supervisor. A
  disconnect (EOF/`POLLHUP`) is immediate in both directions and drives cleanup, logging and
  writer handoff. `flock` on the segment is the fallback for single-writer enforcement.
- **Attachers re-open after an owner restart.** A deleted segment stays alive while it is mapped,
  so after `robotd` restarts, an old policy would keep writing into an orphan without any error.
  Re-open by path and re-check the header when the session drops, the heartbeat stalls, or
  `stat(path)` and `fstat(fd)` report different inodes.

## Pub/sub

Every data stream is a **typed topic** with a declared delivery pattern. Drivers, the RT loop,
loggers and other processes all use the same API. The transport underneath depends on where the
subscriber is.

```rust
let imu = bus.publisher::<ImuSample>("imu/0", Pattern::Queue { cap: 64, overflow: DropOldest })?;
imu.publish(&sample);                         // small Copy types: one call, never blocks
frames.publish_with(|slot| cam.read_into(slot)); // large: fill in place, published on return

let imu_rx = bus.subscriber::<ImuSample>("imu/0")?;
for s in imu_rx.drain() { est.update(s) }     // RT side: wait-free, bounded
```

| Data | Pattern | Why |
|---|---|---|
| Plan (policy → RT) | Latest (triple buffer) | Only the newest matters; it is time-indexed |
| Robot state (RT → policy) | Latest | The policy wants the current state |
| IMU / INS samples (driver → RT) | Queue (SPSC ring), drop oldest | The estimator needs every sample in order |
| Camera frames | Latest, zero-copy slot pool | Large; stale frames are worthless |
| Commands/events (enable, mode, stop request) | Queue, never drop (sender gets an error when full) | Each one must be handled |
| Telemetry, logs, cycle timing | Queue, drop oldest, with a drop counter | Must never block the RT thread |

- **A ring is the right choice for queues**, which is where the instinct for ring buffers fits:
  single-producer single-consumer, a fixed capacity set at creation, wait-free. If there are several
  subscribers, each gets its own read cursor, or a fan-out thread copies to each.
- **Subscribers poll or wait on an fd.** The RT thread never sleeps on a topic; it polls at the top
  of each cycle. Non-RT subscribers (loggers, bridges) can wait on an `eventfd` that the publisher
  signals. Signalling is optional per topic, so RT publishers can skip the syscall.
- **Transports:**
  - In-process: plain atomics/rings in heap memory.
  - Same host: the same structures in shared memory (rules above).
  - Off-host: a bridge thread republishes selected topics over Zenoh, as planned in
    `realtime-control.md`.
- **The RT path has no third-party dependencies.** We write the in-process and shared-memory
  buffers ourselves, because they are small and must be auditable. iceoryx2 (Rust-native, zero-copy
  shared memory) is the alternative to evaluate for the non-RT, same-host transport, especially for
  camera frames, before we build a pool ourselves.

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
- **Configuration** comes from the existing `hardware/setups/<setup>.toml` (one table per device),
  so the Python tools in `elbows_devices` and `robotd` share one file. Device discovery (USB serial
  → port) stays as it is in Python for now.
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
    rt-ipc/                Latest slot, SPSC ring, shared-memory segments, topics/Bus
    rt-devices/            CyclicDevice/StreamDevice/Transport traits; drivers added per device
    robotd/                the main binary: config → devices → RT loop → supervisor
    rt-py/                 PyO3 bindings: attach to topics, write plans, read state
```

## Open questions

- **Servo-side timeout.** Confirm whether the STS3215 firmware has any communication timeout.
- **Own MCU board** between the PC and the servo bus: when, and which host link (CAN, Ethernet,
  USB).
- **First target board** (Jetson Orin vs a Pi-class board), which sets the kernel and toolchain
  setup.
- **Control mode** for Feetech: position-only to start, or current/torque where the servo supports it.
- **Bring-up order:** whether we build the RT loop against the sim device before the Feetech driver.
