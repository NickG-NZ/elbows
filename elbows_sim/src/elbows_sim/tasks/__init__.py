from ..core.task import Task
from .reach import Reach

TASKS: dict[str, type[Task]] = {
    "reach": Reach,
}
