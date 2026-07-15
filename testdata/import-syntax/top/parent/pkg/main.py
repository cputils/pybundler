import math  # module import
import numpy as np
import os, sys, time  # multiple import
import numpy as np, pandas as pd  # multiple with alias

from math import pi
from math import pi as PI
from math import pi, cos, sin
from math import pi as PI, cos as cosine
from math import pi, cos as cosine, sin
from math import *  # wildcard

from math import (
    pi,  # comment
    cos as cosine,  # comment
    sin,
    tan,
)

from math import (
    pi as PI,
    cos as cosine,  # comment
)

from math import (
    pi,
    cos,
)

from math import pi, \
    cos, \
    sin  # comment

from . import sibling_module  # relative
from .sibling_module import my_function
from .sibling_module import (
    my_function,
    other_function,  # comment
)
from .. import parent_module
from ... import grand_parent_module
from ..other_package import their_module
