"""Find a revolute joint's physical end stops from a URDF's meshes.

Rotates everything downstream of the joint about it (other joints at 0, or --pose) and measures
the overlap volume between each moving and each fixed mesh, using the exact meshes (voxelised and
filled), not collision hulls.

Mating parts (a servo and the bracket bolted to it) overlap at every angle, and voxelisation makes
that overlap wobble by a few mm^3. So each mesh pair gets its own reference: its largest overlap in
a window around 0 deg, assumed collision-free. Contact is overlap clearly above that. A coarse pass
(1 mm voxels) over all pairs finds where contact starts; a fine pass (--pitch) on the pairs involved
pins down the angle.

    uv run python elbows_sim/scripts/hard_stop_sweep.py --joint wrist_roll
    uv run python elbows_sim/scripts/hard_stop_sweep.py --urdf ~/so101_6dof.urdf --joint elbow_rotate

Meshes must be watertight. Record results in a hard_stops.toml next to the URDF (see assets/so101/).
"""

from __future__ import annotations

import argparse
import itertools
import warnings

import mujoco
import numpy as np
import trimesh
from elbows_sim.backends.mujoco_cpu import MujocoSim
from elbows_sim.core.spec import SceneSpec
from elbows_sim.formats import urdf_joints
from elbows_sim.robots.so101 import SCENE, so101_robot

warnings.filterwarnings("ignore", category=RuntimeWarning)  # spurious matmul warnings from macOS Accelerate


class Sweep:
    def __init__(self, urdf: str | None, joint: str, pose: str | None):
        robot = so101_robot(urdf, hard_stops=None, self_collision=False)
        names = [j.name for j in urdf_joints(robot.urdf)]
        sim = MujocoSim(SceneSpec("sweep", {"mjcf": SCENE}, tuple(names), robot=robot), num_envs=1)
        self.m, self.d = m, _ = sim.model, sim.datas[0]
        jid = mujoco.mj_name2id(m, mujoco.mjtObj.mjOBJ_JOINT, joint)
        if jid < 0:
            raise SystemExit(f"no joint {joint!r}; have {names}")
        self.qadr = m.jnt_qposadr[jid]
        self.limits = np.degrees(m.jnt_range[jid])
        self.pose = np.zeros(m.nq)
        if pose:
            for name, deg in zip(names, map(float, pose.split(","))):
                self.pose[m.jnt_qposadr[mujoco.mj_name2id(m, mujoco.mjtObj.mjOBJ_JOINT, name)]] = np.radians(deg)

        moving_bodies = {b for b in range(m.nbody) if _is_descendant(m, b, m.jnt_bodyid[jid])}
        self.meshes: dict[int, trimesh.Trimesh] = {}
        self.moving: dict[int, bool] = {}
        for g in range(m.ngeom):
            if m.geom_type[g] != mujoco.mjtGeom.mjGEOM_MESH or m.geom_bodyid[g] == 0:
                continue
            mid = m.geom_dataid[g]
            v = m.mesh_vert[m.mesh_vertadr[mid] : m.mesh_vertadr[mid] + m.mesh_vertnum[mid]]
            f = m.mesh_face[m.mesh_faceadr[mid] : m.mesh_faceadr[mid] + m.mesh_facenum[mid]]
            self.meshes[g] = tm = trimesh.Trimesh(v.copy(), f.copy())
            self.moving[g] = m.geom_bodyid[g] in moving_bodies
            if not tm.is_watertight:
                print(f"warning: {self.label(g)} is not watertight; its overlap may be wrong")
        self.pairs = [
            (i, j) for i, j in itertools.permutations(self.meshes, 2) if self.moving[i] and not self.moving[j]
        ]
        self._vox: dict[tuple[int, float], trimesh.voxel.VoxelGrid] = {}

    def label(self, g: int) -> str:
        return f"{self.m.body(self.m.geom_bodyid[g]).name}:{self.m.mesh(self.m.geom_dataid[g]).name}"

    def vox(self, g: int, pitch: float) -> trimesh.voxel.VoxelGrid:
        if (g, pitch) not in self._vox:
            self._vox[g, pitch] = self.meshes[g].voxelized(pitch / 1000).fill()
        return self._vox[g, pitch]

    def overlaps(self, deg: float, pairs: list[tuple[int, int]], pitch: float) -> np.ndarray:
        """mm^3 of each moving mesh inside its fixed partner, at joint angle `deg`."""
        m, d = self.m, self.d
        d.qpos[:] = self.pose
        d.qpos[self.qadr] = np.radians(deg)
        mujoco.mj_kinematics(m, d)
        out = np.zeros(len(pairs))
        for k, (i, j) in enumerate(pairs):
            pi, Ri = d.geom_xpos[i], d.geom_xmat[i].reshape(3, 3)
            pj, Rj = d.geom_xpos[j], d.geom_xmat[j].reshape(3, 3)
            world = self.vox(i, pitch).points @ Ri.T + pi
            out[k] = self.vox(j, pitch).is_filled((world - pj) @ Rj).sum() * (pitch**3)
        return out

    def reference(
        self, pairs: list[tuple[int, int]], pitch: float, window: float, step: float
    ) -> tuple[np.ndarray, np.ndarray]:
        """Each pair's largest overlap within +-window deg (assumed collision-free), and its spread there."""
        table = np.array([self.overlaps(a, pairs, pitch) for a in np.arange(-window, window + step / 2, step)])
        return table.max(axis=0), table.max(axis=0) - table.min(axis=0)


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--urdf", default=None, help="default: stock SO-101")
    p.add_argument("--joint", required=True)
    p.add_argument("--pose", default=None, help="comma-separated angles (deg) for all joints, in tree order")
    p.add_argument("--pitch", type=float, default=0.3, help="fine voxel size, mm")
    p.add_argument("--step", type=float, default=2.0, help="coarse sweep step, deg")
    p.add_argument("--window", type=float, default=45.0, help="+-deg around 0 assumed collision-free")
    p.add_argument("--threshold", type=float, default=3.0, help="overlap above reference that counts as contact, mm^3")
    a = p.parse_args()

    s = Sweep(a.urdf, a.joint, a.pose)
    print(
        f"{a.joint}: {sum(s.moving.values())} moving meshes, {len(s.pairs)} pairs; URDF limits {s.limits.round(1)} deg"
    )

    coarse = 1.0
    ref, spread = s.reference(s.pairs, coarse, a.window, a.step)
    coarse_threshold = np.maximum(a.threshold, spread)  # 1 mm voxels are noisy; allow each pair its own wobble
    for sign in (-1, 1):
        name = "-" if sign < 0 else "+"
        stop = None
        for deg in np.arange(a.window + a.step, 180.0 + a.step / 2, a.step):
            over = s.overlaps(sign * deg, s.pairs, coarse) - ref
            if not (over > coarse_threshold).any():
                continue
            # Fine pass on the pairs that hit, between the last clear coarse angle and this one.
            hit_pairs = [s.pairs[k] for k in np.flatnonzero(over > coarse_threshold)]
            fine_ref, _ = s.reference(hit_pairs, a.pitch, a.window, 5.0)
            for x in np.arange(deg - 2 * a.step, deg + a.step, 0.25):
                fine = s.overlaps(sign * x, hit_pairs, a.pitch) - fine_ref
                if (fine > a.threshold).any():
                    i, j = hit_pairs[int(np.argmax(fine))]
                    stop = sign * (x - 0.125)
                    print(f"{name}: stop at {stop:+.1f} deg ({s.label(i)} hits {s.label(j)})")
                    break
            if stop is not None:
                break  # otherwise the coarse hit was voxel noise; keep sweeping
        if stop is None:
            print(f"{name}: no contact up to {sign * 180:+.0f} deg")


def _is_descendant(m: mujoco.MjModel, b: int, root: int) -> bool:
    while b > 0:
        if b == root:
            return True
        b = m.body_parentid[b]
    return False


if __name__ == "__main__":
    main()
