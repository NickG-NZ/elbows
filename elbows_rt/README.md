# elbows_rt

Real-time robot runtime in Rust. Design: [`docs/middleware.md`](../docs/middleware.md).

| Crate | What |
|---|---|
| `rt-core` | Messages and topic declarations, monotonic time, `ClockTrigger`, RT thread setup, cycle stats |
| `rt-ipc` | Typed topics over iceoryx2 (zero-copy shared-memory pub/sub) |
| `rt-devices` | `CyclicDevice`, `StreamDevice` and `Transport` traits. No drivers yet |
| `robotd` | The robot process: supervisor + RT loop. `plan-demo` is a stand-in policy |

```bash
cd elbows_rt
cargo test
cargo run --release --bin robotd -- --robot so101 --rate-hz 200   # terminal 1
cargo run --release --bin plan-demo -- --robot so101              # terminal 2: robot goes Running
```

Stop `plan-demo` and `robotd` holds the last target. With no devices yet, `robotd` treats measured =
commanded. RT scheduling settings go in the `[robotd]` table of the setup file; see `robotd/src/config.rs`.
On macOS everything runs, without RT scheduling. iceoryx2's CLI (`cargo install iceoryx2-cli`, then
`iox2 service list`) shows live topics.
