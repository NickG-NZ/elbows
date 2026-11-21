"""MJX backend: MuJoCo in JAX. All envs live in one batched `mjx.Data` and step under `jit(vmap(...))`.

Same MJCF, names and conventions as `mujoco_cpu.py`, so tasks and policies run unchanged. State stays
in JAX on the default JAX device (GPU when `jax[cuda12]` finds one, else CPU). Tensors cross to torch
through DLPack, which avoids copies when both libraries share a device. Rendering copies the
requested envs into a CPU `MjData` and uses MuJoCo's renderer, since MJX has none.
"""

from __future__ import annotations

import contextlib
import io
from functools import partial

import jax
import jax.numpy as jnp
import numpy as np
import torch

with contextlib.redirect_stdout(io.StringIO()):  # MJX prints a warning when the optional warp impl is missing
    from mujoco import mjx

from ..core.sim import Sim
from ..core.spec import SceneSpec
from .mujoco_cpu import MujocoSim


# Module-level and taking the model as an argument, so envs with the same scene and env count share
# one compilation.
@partial(jax.jit, static_argnums=2)
def _step(mx: mjx.Model, d: mjx.Data, decimation: int) -> mjx.Data:
    def one(d):
        d = jax.lax.fori_loop(0, decimation, lambda _, d: mjx.step(mx, d), d)
        return mjx.kinematics(mx, d)  # mjx.step leaves positions one substep stale; refresh frames

    return jax.vmap(one)(d)


@jax.jit
def _forward(mx: mjx.Model, d: mjx.Data) -> mjx.Data:
    return jax.vmap(lambda d: mjx.forward(mx, d))(d)


@jax.jit
def _reset(d: mjx.Data, d0: mjx.Data, mask: jax.Array) -> mjx.Data:
    return jax.tree.map(lambda x, x0: jnp.where(mask.reshape(-1, *([1] * x0.ndim)), x0, x), d, d0)


class MjxSim(Sim):
    REQUIRES = "mujoco.mjx"

    def __init__(self, scene: SceneSpec, num_envs: int, device: str | torch.device | None = None):
        jax_dev = jax.devices()[0]
        default = "cuda" if jax_dev.platform == "gpu" else "cpu"
        super().__init__(scene, num_envs, torch.device(device or default))
        # Reuse the CPU backend for MJCF loading, name lookup and validation, and later for rendering.
        self._cpu = MujocoSim(scene, num_envs=1)
        m = self.model = self._cpu.model
        self._limits = self._cpu._limits.to(self.device)
        self._ctrl_mid = jnp.asarray(self._cpu._ctrl_mid, jnp.float32)
        self._ctrl_half = jnp.asarray(self._cpu._ctrl_half, jnp.float32)
        self._qpos_adr = jnp.asarray(self._cpu._qpos_adr)
        self._qvel_adr = jnp.asarray(self._cpu._qvel_adr)
        self._act_ids = jnp.asarray(self._cpu._act_ids)

        self.mx = mjx.put_model(m, device=jax_dev)
        d0 = mjx.forward(self.mx, mjx.make_data(m, device=jax_dev))
        self._d0 = d0
        self.data = jax.tree.map(lambda x: jnp.broadcast_to(x, (num_envs, *x.shape)), d0)

        self._dirty = False

    # --- helpers ---------------------------------------------------------------------------------
    def _sync(self) -> None:
        if self._dirty:
            self.data = _forward(self.mx, self.data)
            self._dirty = False

    def _torch(self, x: jax.Array) -> torch.Tensor:
        return torch.from_dlpack(x).to(self.device, torch.float32)

    def _jax(self, x: torch.Tensor) -> jax.Array:
        dev = jax.devices()[0]
        x = x.detach().to(torch.float32).contiguous()
        if dev.platform == "cpu" or x.device.type != "cuda":
            return jax.device_put(jnp.asarray(x.cpu().numpy()), dev)
        return jax.dlpack.from_dlpack(x)

    def _ids(self, env_ids: torch.Tensor) -> jax.Array:
        return jnp.asarray(env_ids.detach().cpu().numpy())

    def _set(self, **fields: jax.Array) -> None:
        self.data = self.data.replace(**fields)
        self._dirty = True

    # --- Sim interface ---------------------------------------------------------------------------
    def set_action(self, action: torch.Tensor) -> None:
        ctrl = self._ctrl_mid + self._ctrl_half * jnp.clip(self._jax(action), -1, 1)
        self.data = self.data.replace(ctrl=self.data.ctrl.at[:, self._act_ids].set(ctrl))

    def step(self) -> None:
        self._sync()
        self.data = _step(self.mx, self.data, self.scene.decimation)

    def joint_pos(self) -> torch.Tensor:
        return self._torch(self.data.qpos[:, self._qpos_adr])

    def joint_vel(self) -> torch.Tensor:
        return self._torch(self.data.qvel[:, self._qvel_adr])

    def frame_pos(self, name: str) -> torch.Tensor:
        self._sync()
        kind, i = self._cpu._frames[name]
        arr = self.data.site_xpos if kind == "site" else self.data.xpos
        return self._torch(arr[:, i])

    def joint_limits(self) -> torch.Tensor:
        return self._limits

    def reset(self, env_ids: torch.Tensor) -> None:
        mask = jnp.zeros(self.num_envs, bool).at[self._ids(env_ids)].set(True)
        self.data = _reset(self.data, self._d0, mask)
        self._dirty = True

    def set_joint_state(self, env_ids: torch.Tensor, pos: torch.Tensor, vel: torch.Tensor) -> None:
        ids = self._ids(env_ids)[:, None]
        d = self.data
        self._set(
            qpos=d.qpos.at[ids, self._qpos_adr].set(self._jax(pos)),
            qvel=d.qvel.at[ids, self._qvel_adr].set(self._jax(vel)),
        )

    def set_marker_pos(self, name: str, env_ids: torch.Tensor, pos: torch.Tensor) -> None:
        mocap_id = int(self._cpu._markers[name])
        self._set(mocap_pos=self.data.mocap_pos.at[self._ids(env_ids), mocap_id].set(self._jax(pos)))

    def render(
        self, camera: str, env_ids: torch.Tensor | None = None, width: int = 256, height: int = 256
    ) -> torch.Tensor:
        ids = np.arange(self.num_envs) if env_ids is None else env_ids.cpu().numpy()
        qpos, qvel, mocap = (np.asarray(getattr(self.data, f))[ids] for f in ("qpos", "qvel", "mocap_pos"))
        cpu, d = self._cpu, self._cpu.datas[0]
        frames = []
        for k in range(len(ids)):
            d.qpos[:], d.qvel[:], d.mocap_pos[:] = qpos[k], qvel[k], mocap[k]
            cpu._dirty.add(0)
            frames.append(cpu.render(camera, torch.tensor([0]), width, height)[0])
        return torch.stack(frames).to(self.device)

    def close(self) -> None:
        self._cpu.close()
