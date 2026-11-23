from ..core.task import Task
from .reach import Reach
from .so101_dance import So101Dance
from .so101_pose import So101Pose

TASKS: dict[str, type[Task]] = {
    "reach": Reach,
    "so101_dance": So101Dance,
    "so101_pose": So101Pose,
}
