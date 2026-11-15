# Real-time control: policy to robot

Design notes for running learned policies (VLAs such as π0.5, RL policies) on real hardware.
Nothing here is implemented yet. The simulation pieces at the end are the next step.

## Layers

A learned policy never sits inside the fast control loop. Each layer runs at its own rate and hands
its output to the layer below.

| Layer | Rate | Runs on | Timing |
|---|---|---|---|
| Motor current/torque servo | 10–40 kHz | Drive microcontroller/FPGA | Hard real-time, firmware |
| Joint/arm controller | 0.5–1 kHz | PREEMPT_RT Linux or embedded controller | Hard real-time |
| Policy | 1–50 Hz | GPU, Python, possibly another machine | Best effort |

The policy outputs a **plan**, which is a timed chunk of future targets. The real-time controller
interpolates the plan, enforces limits, and falls back to a safe stop if no new plan arrives in time.
Network, Python and GPU jitter only affect when a plan arrives. They never affect the 1 kHz loop.

```
 GPU machine (non-real-time)               Robot controller machine
┌────────────────────────────┐           ┌────────────────────────────────────────────┐
│ policy: obs → action chunk │  network  │ bridge thread      │ real-time thread       │
│                            │──────────▶│ recv, check, stamp │ every 1 ms:            │
│                            │           │        │           │  read latest plan      │
│                            │           │        ▼           │  sample at current time│
│                            │           │   [latest-value]──▶│  limits → command      │
│                            │◀──────────│   [latest-value]◀──│  write current state   │
└────────────────────────────┘ state, ts └────────────────────────────────────────────┘
```

The real-time thread never touches a socket, the GPU or the allocator, and never blocks on a lock.

## The plan

```c
struct Plan {             // fixed size, no pointers
  uint64_t seq;           // increases with each plan
  int64_t  t0_ns;         // robot-clock time at which waypoint 0 applies
  uint32_t dt_ns;         // waypoint spacing
  uint16_t n;             // valid waypoints (≤ N_MAX)
  uint64_t obs_stamp_ns;  // robot timestamp of the observation the plan was computed from
  float    q[N_MAX][DOF]; // joint or end-effector targets, plus gripper
};
```

- **Plans are tied to time, not a queue position.** Waypoint *k* applies at `t0 + k·dt`, and the
  controller samples the plan at the current time. Waypoints that have already passed when a plan
  arrives are skipped, so a late plan costs responsiveness, not correctness.
- **Clocks are not synchronized across machines.** The robot stamps each observation with its own
  clock, and the policy echoes that stamp back in `obs_stamp_ns`. Only the robot's clock is used.
- **The controller validates each plan.** It checks for finite values, joint/velocity/workspace
  limits, and a bounded jump from the current target. It rejects bad plans and keeps the old one
  running. It blends to a new plan over about 50–100 ms, unless the policy already guarantees
  continuity (real-time chunking).
- **The controller has a watchdog.** Near the end of the active plan, it slows to a hold. Past a
  timeout, it triggers a protective stop. The hardware safety circuit (e-stop) is separate and is
  the main protection.

## Decision: how plans cross into the real-time thread

**Small data, such as plans and robot state, uses a latest-value slot protected by a short lock and
copy. The real-time side only ever calls `try_lock`.** If it can't get the lock in a cycle, it reuses
the previous plan for that cycle. Large data, such as camera frames, uses a lock-free triple buffer
or zero-copy shared memory (iceoryx, Zenoh SHM).

Reasons:

- **This is how mainstream stacks do it.** PX4's uORB copies each message inside a tiny critical
  section. ArduPilot uses semaphores. ROS 2 `realtime_tools::RealtimeBuffer` is a mutex-protected
  double buffer read with `try_lock`. True lock-free latest-value objects exist (Orocos RTT
  `DataObjectLockFree`), but they are the exception.
- **It's cheap and predictable.** Copying a few hundred bytes under a lock takes well under a
  microsecond, and `try_lock` means the real-time side never waits.
- **A triple buffer pays off only when copying is expensive.** It has three slots (writer, reader,
  and the latest complete copy in between) and one atomic index. Neither side ever waits, and the
  writer fills its slot in place, so megabyte payloads are never copied under a lock.

API rule for any buffer we write: **writing is one call.** Use `write(const T&)` for small data. Use
`write(fill_fn)` for large data, where the buffer passes its slot to `fill_fn` and publishes when it
returns. Never expose a separate "get slot" and "publish", because it is easy to forget the publish
or keep writing after it. Single writer, single reader. Use acquire/release ordering on the index,
and put the two sides' state on separate cache lines.

## Threads and processes

- **Process boundaries isolate faults.** For example, a crashing camera driver must not take the
  controller down. Threads within one process share memory cheaply.
- **Every independent blocking input gets its own thread.** Each camera, serial sensor or socket is
  separate. Off-board, non-real-time streams such as plans and teleop can share one network session.
- **Cameras stream directly to the policy machine, not through the controller.** The policy matches
  frames to robot state by capture timestamp.

## Timing source

With EtherCAT, the Distributed Clocks (DC) bus time is the control timebase:

- Every drive and sensor samples and actuates at the hardware SYNC0 event, simultaneously, with
  sub-µs jitter.
- Linux only has to deliver each cycle's frame before SYNC0 minus a margin. That is easy on tuned
  PREEMPT_RT, where wake-up jitter is about 10–50 µs.
- We prefer that the Linux loop follows the bus reference clock. If instead the bus follows Linux,
  use `CLOCK_MONOTONIC`, never wall-clock time.
- Stamp sensor data with the DC time of its SYNC0, not the time Linux read it.
- Bridge DC time to PTP (IEEE 1588) so cameras and other machines share one timeline.

PX4 instead lets each new IMU sample trigger the controller (event-driven). That suits one dominant
sensor and drives that accept commands at any time. Fixed-cycle EtherCAT drives (CSP/CSV/CST modes)
need a time-triggered loop.

## ROS 2 mapping

`ros2_control` provides the controller side:

- `controller_manager` runs `read() → update() → write()` on a real-time thread.
- The EtherCAT master lives inside a `SystemInterface` hardware plugin, for example
  `ethercat_driver_ros2`. Bus data reaches controllers through in-process state and command
  interfaces, not topics.
- `JointTrajectoryController` follows timed trajectories with spline interpolation, replaces a
  running trajectory mid-motion, and enforces limits. A plan maps onto a
  `trajectory_msgs/JointTrajectory`.
- Topics are used only at the edges, through `RealtimeBuffer` and `RealtimePublisher`.
- Caveat: `controller_manager` times its loop from the system clock. Following the EtherCAT DC clock
  needs custom work.

## Transport

Zenoh is the planned transport between the policy and the robot. It has zero-copy shared memory on
one host, uses the same API locally and remotely, and offers priority and drop-vs-block QoS.
`rmw_zenoh` provides ROS 2 interop. Choose the link per stream:

| Situation | Link |
|---|---|
| Same host | Shared memory for payloads; Unix socket for control |
| Wired LAN | TCP |
| Wi-Fi sensor streams | UDP, best effort (latest wins) |
| Commands, plans, stops | Reliable (TCP/QUIC), high priority |
| Over the internet | QUIC or TLS over TCP |

## Next step: model this in simulation

LIBERO-style evaluation hides latency, because the simulator pauses while the policy thinks. Three
additions to `elbows_sim` would expose it before any hardware is involved:

- `Plan`: the timed action chunk above.
- `ChunkController`: samples the active plan at the current sim time, blends, applies limits, and
  holds position when the plan expires. It sits between `Policy` and `Sim`.
- `LatencyModel`: fixed or random delay between an observation and when its plan arrives, with the
  simulator running in real time while the policy computes.

Then measure π0.5 on LIBERO at 0, 50, 150 and 300 ms of delay, with and without blending.
