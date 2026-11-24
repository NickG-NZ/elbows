# Hardware

Setup notes, robot setup files (`setups/`) and diagnostics for physical robot experiments, starting
with the SO101 arm. Driver and discovery code lives in `elbows_devices/`.

## LeRobot (optional)

LeRobot handles SO101 motor setup, calibration, teleoperation and recording, so we can use the arm
before writing our own drivers. It is an optional extra (Python ≥ 3.12):

```bash
uv sync --extra lerobot          # a plain `uv sync` removes it again
uv run lerobot-info
```

SO101 bring-up, following the [LeRobot SO-101 guide](https://huggingface.co/docs/lerobot/so101).
We have a follower arm only (no leader, so no teleoperation).

```bash
PORT=$(uv run elbows-devices port --setup hardware/setups/so101.toml arm)   # see Serial ports
uv run lerobot-setup-motors --robot.type=so101_follower --robot.port=$PORT   # once, assigns motor IDs
uv run lerobot-calibrate --robot.type=so101_follower --robot.port=$PORT --robot.id=follower
```

### Serial ports

Each robot setup (a physical rig) has its own file, `hardware/setups/<setup>.toml`, listing its
devices and what their drivers need. Setup files are local, not committed; pass one with `--setup`
to point the same code at a different robot. Serial devices are found by USB serial number, which is
the same on every OS, USB socket and replug (the port path is not: `/dev/cu.usbmodem…` on macOS,
`/dev/ttyACM0`/`ACM1` in plug order on Linux).

```bash
cp hardware/setups/example.toml hardware/setups/so101.toml
uv run elbows-devices list                                           # connected ports and USB serials
uv run elbows-devices port --setup hardware/setups/so101.toml arm    # current port of a device
```

On Linux, add yourself to the `dialout` group to open serial ports without sudo
(`sudo usermod -aG dialout $USER`, then log in again).

Calibration is saved under `~/.cache/huggingface/lerobot/calibration/`, keyed by `id`. Reuse the
same `id` every time. Not yet tested on our arm.

## Camera discovery

From the repository root, run LeRobot's OpenCV discovery and capture tool:

```bash
uv tool run --from lerobot==0.7.0 lerobot-find-cameras opencv \
  --output-dir hardware/outputs/captured_images --record-time-s 2
```

This installs the tool in an isolated uv environment, discovers available cameras, and saves
test frames as `opencv_<index>.png`. It captures from all detected cameras. Outputs stay local
and are ignored by Git. See the [LeRobot camera documentation](https://huggingface.co/docs/lerobot/cameras).

On macOS, allow camera access when prompted. If the tool reports `not authorized to capture video`,
check System Settings → Privacy & Security → Camera for the application running the command,
then rerun it. Discovery may print errors for unused indices; check the detected-camera summary
and saved frames to determine success.

## Logitech BRIO 301 smoke test

Tested on 2026-11-18, on an Apple Silicon Mac with LeRobot 0.7.0:

- macOS identified the USB webcam as **BRIO 301** (Logitech).
- OpenCV's AVFoundation device ordering (sorted by unique ID) mapped the BRIO to index **0**.
- `lerobot-find-cameras opencv` opened that camera and repeatedly saved frames successfully.
- Reported default profile: **1920 × 1080 at 5 FPS**, using **AVFOUNDATION**.
- The discovery test frame is at `outputs/captured_images/opencv_0.png` (local only).

An earlier version of these notes incorrectly identified index 1 as the BRIO by using unsorted
AVFoundation enumeration. Index 1 was the MacBook Air camera; its 15 FPS default and initial
30 FPS measurements were not BRIO results.

Retested the BRIO at index 0 with OpenCV 4.13.0, explicitly requesting 30 FPS:

| Requested resolution | Measured frame delivery | Frames | Failed reads | Longest frame interval |
|---|---:|---:|---:|---:|
| 1920 × 1080 | 29.99 FPS | 300 | 0 | 66.17 ms |
| 1280 × 720 | 30.00 FPS | 300 | 0 | 64.62 ms |

Each run discarded 30 warmup frames, then read continuously for 10 seconds. Successful reads
were timestamped with `time.monotonic()`; FPS is `(frames - 1) / (last_time - first_time)`.
Returned frame dimensions matched the requests. This measures delivery to Python, not exposure
timing, unique exposures, or end-to-end latency. No video was saved during the FPS tests.

The initial scan encountered a macOS camera permission request; a subsequent scan succeeded.
This verifies discovery and short capture runs, not long-running reliability or robot integration.
Camera indices can change after reconnecting devices or rebooting; rerun discovery before
configuring the SO101 camera. The robot arm has not yet been configured or tested.
