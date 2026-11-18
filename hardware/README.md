# Hardware

Setup notes and local diagnostics for physical robot experiments, starting with the SO101 arm.

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
- AVFoundation enumeration mapped the BRIO to camera index **1** during this run.
- `lerobot-find-cameras opencv` opened that camera and repeatedly saved frames successfully.
- Reported default profile: **1920 × 1080 at 15 FPS**, using **AVFOUNDATION**.
- The final test frame is at `outputs/captured_images/opencv_1.png` (local only).

The initial scan encountered a macOS camera permission request; a subsequent scan succeeded.
This verifies discovery and frame capture, not sustained frame rate or robot integration.
Camera indices can change after reconnecting devices or rebooting; rerun discovery before
configuring the SO101 camera. The robot arm has not yet been configured or tested.
