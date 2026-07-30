import sys

import helper

sys.modules.pop("time", None)
import time

print(hasattr(time, "monotonic"))
