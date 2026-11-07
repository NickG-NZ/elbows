"""MuJoCo CPU backend: one MjModel, one MjData per env, stepped in a Python loop.

This is the reference backend. It runs anywhere (including macOS) and is fast enough for small
state-based tasks with tens to hundreds of envs. For thousands of envs on a GPU, add an MJWarp
backend implementing the same `Sim` interface. Tasks will not need to change.
"""

from __future__ import annotations

import mujoco
import numpy as np
import torch

from ..core.sim import Sim
from ..core.spec import SceneSpec


class MujocoSim(Sim):
    REQUIRES = "mujoco"

    def __init__(self, scene: SceneSpec, num_envs: int, device: str | torch.device | None = None):
        super().__init__(scene, num_envs, torch.device(device or "cpu"))
        if "mjcf" not in scene.assets:
            raise ValueError(f"scene {scene.name!r} has no 'mjcf' asset; MuJoCo needs one")
        m = self.model = mujoco.MjModel.from_xml_path(str(scene.assets["mjcf"]))
        m.opt.timestep = scene.physics_dt
        self.datas = [mujoco.MjData(m) for _ in range(num_envs)]

        jids = [self._id(mujoco.mjtObj.mjOBJ_JOINT, j) for j in scene.actuated_joints]
        for name, jid in zip(scene.actuated_joints, jids):
            if int(m.jnt_type[jid]) not in (int(mujoco.mjtJoint.mjJNT_HINGE), int(mujoco.mjtJoint.mjJNT_SLIDE)):
                raise ValueError(f"actuated joint {name!r} must be a hinge or slide joint")
        self._qpos_adr = np.array([m.jnt_qposadr[j] for j in jids])
        self._qvel_adr = np.array([m.jnt_dofadr[j] for j in jids])

        self._act_ids = np.array([self._actuator_for(jid, name) for jid, name in zip(jids, scene.actuated_joints)])
        lo, hi = m.actuator_ctrlrange[self._act_ids].T
        if not np.all(m.actuator_ctrllimited[self._act_ids]):
            raise ValueError("every actuator needs a ctrlrange so normalised actions can be mapped onto it")
        self._ctrl_mid, self._ctrl_half = (hi + lo) / 2, (hi - lo) / 2

        limited = m.jnt_limited[jids].astype(bool)
        rng = m.jnt_range[jids]
        self._limits = torch.tensor(
            np.where(limited[:, None], rng, [-np.inf, np.inf]), dtype=torch.float32, device=self.device
        )

        self._frames: dict[str, tuple[str, int]] = {}
        for name in scene.frames:
            sid = mujoco.mj_name2id(m, mujoco.mjtObj.mjOBJ_SITE, name)
            self._frames[name] = ("site", sid) if sid >= 0 else ("body", self._id(mujoco.mjtObj.mjOBJ_BODY, name))
        self._markers = {}
        for name in scene.markers:
            mocap_id = m.body_mocapid[self._id(mujoco.mjtObj.mjOBJ_BODY, name)]
            if mocap_id < 0:
                raise ValueError(f"marker {name!r} must be a mocap body")
            self._markers[name] = mocap_id
        for cam in scene.cameras:
            self._id(mujoco.mjtObj.mjOBJ_CAMERA, cam)

        self._renderers: dict[tuple[int, int], mujoco.Renderer] = {}
        self._dirty: set[int] = set()
        for d in self.datas:
            mujoco.mj_forward(m, d)

    # --- helpers ---------------------------------------------------------------------------------
    def _id(self, kind: mujoco.mjtObj, name: str) -> int:
        i = mujoco.mj_name2id(self.model, kind, name)
        if i < 0:
            raise KeyError(f"{kind.name} {name!r} not found in {self.scene.name}")
        return i

    def _actuator_for(self, jid: int, name: str) -> int:
        m = self.model
        ids = [
            a
            for a in range(m.nu)
            if int(m.actuator_trntype[a]) == int(mujoco.mjtTrn.mjTRN_JOINT) and m.actuator_trnid[a, 0] == jid
        ]
        if len(ids) != 1:
            raise ValueError(f"joint {name!r} needs exactly one actuator, found {len(ids)}")
        return ids[0]

    def _sync(self) -> None:
        for i in self._dirty:
            mujoco.mj_forward(self.model, self.datas[i])
        self._dirty.clear()

    def _tensor(self, arr: np.ndarray) -> torch.Tensor:
        return torch.as_tensor(arr, dtype=torch.float32, device=self.device)

    @staticmethod
    def _np(x: torch.Tensor) -> np.ndarray:
        return x.detach().to("cpu", torch.float64).numpy()

    # --- Sim interface ---------------------------------------------------------------------------
    def set_action(self, action: torch.Tensor) -> None:
        ctrl = self._ctrl_mid + self._ctrl_half * self._np(action.clamp(-1, 1))
        for d, c in zip(self.datas, ctrl):
            d.ctrl[self._act_ids] = c

    def step(self) -> None:
        self._sync()
        m = self.model
        for d in self.datas:
            for _ in range(self.scene.decimation):
                mujoco.mj_step(m, d)
            mujoco.mj_kinematics(m, d)  # mj_step leaves positions one substep stale; refresh frames

    def joint_pos(self) -> torch.Tensor:
        return self._tensor(np.stack([d.qpos[self._qpos_adr] for d in self.datas]))

    def joint_vel(self) -> torch.Tensor:
        return self._tensor(np.stack([d.qvel[self._qvel_adr] for d in self.datas]))

    def frame_pos(self, name: str) -> torch.Tensor:
        self._sync()
        kind, i = self._frames[name]
        attr = "site_xpos" if kind == "site" else "xpos"
        return self._tensor(np.stack([getattr(d, attr)[i] for d in self.datas]))

    def joint_limits(self) -> torch.Tensor:
        return self._limits

    def reset(self, env_ids: torch.Tensor) -> None:
        for i in env_ids.tolist():
            mujoco.mj_resetData(self.model, self.datas[i])
            self._dirty.add(i)

    def set_joint_state(self, env_ids: torch.Tensor, pos: torch.Tensor, vel: torch.Tensor) -> None:
        pos, vel = self._np(pos), self._np(vel)
        for k, i in enumerate(env_ids.tolist()):
            self.datas[i].qpos[self._qpos_adr] = pos[k]
            self.datas[i].qvel[self._qvel_adr] = vel[k]
            self._dirty.add(i)

    def set_marker_pos(self, name: str, env_ids: torch.Tensor, pos: torch.Tensor) -> None:
        mocap_id, pos = self._markers[name], self._np(pos)
        for k, i in enumerate(env_ids.tolist()):
            self.datas[i].mocap_pos[mocap_id] = pos[k]
            self._dirty.add(i)

    def render(
        self, camera: str, env_ids: torch.Tensor | None = None, width: int = 256, height: int = 256
    ) -> torch.Tensor:
        self._sync()
        key = (width, height)
        if key not in self._renderers:
            self._renderers[key] = mujoco.Renderer(self.model, height=height, width=width)
        r = self._renderers[key]
        ids = range(self.num_envs) if env_ids is None else env_ids.tolist()
        frames = []
        for i in ids:
            r.update_scene(self.datas[i], camera=camera)
            frames.append(r.render())
        return torch.as_tensor(np.stack(frames), device=self.device)

    def close(self) -> None:
        for r in self._renderers.values():
            r.close()
        self._renderers.clear()
