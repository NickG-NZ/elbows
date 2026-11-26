# elbows_rt

Real-time robot runtime in Rust. Design: [`docs/middleware.md`](../docs/middleware.md).

| Crate | What |
|---|---|
| `rt-core` | Messages and topic declarations, monotonic time, `ClockTrigger`, RT thread setup, cycle stats |
| `rt-node` | `Node`: the API every process uses for topics and services, on the robot (`Link::Local`) or off it (`Link::Remote`) |
| `rt-ipc` | Backend: topics over iceoryx2 (zero-copy shared-memory pub/sub) |
| `rt-devices` | Setup files, serial-port discovery, `CyclicDevice`/`StreamDevice`/`Transport` traits. No drivers yet |
| `robotd` | The robot process: supervisor + RT loop. `plan-demo` is a stand-in policy |
| `rt-net` | Backend: Zenoh services, node presence, topics that leave the host |
| `robotctl` | Operator CLI: `status`/`enable`/`hold`/`stop` over Zenoh; `devices list`/`port` |

```bash
cd elbows_rt
cargo test
cargo build --release
./target/release/robotd --robot so101 --rate-hz 200       # terminal 1: starts disabled
./target/release/robotctl --robot so101 enable            # → hold
./target/release/plan-demo --robot so101                  # terminal 2: local policy → running
./target/release/plan-demo --robot so101 --remote         # or: a remote policy over Zenoh
./target/release/robotctl --robot so101 status            # mode, plans, timing, nodes
./target/release/robotctl --robot so101 stop              # → disabled
```

Stop `plan-demo` and `robotd` holds the last target. With no devices yet, `robotd` treats measured =
commanded. RT scheduling settings go in the `[robotd]` table of the setup file; see `robotd/src/config.rs`.
On macOS everything runs, without RT scheduling. iceoryx2's CLI (`cargo install iceoryx2-cli`, then
`iox2 service list`) shows live topics.
