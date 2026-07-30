import sys
import types

virtual = types.ModuleType(__name__ + ".virtual")
virtual.VALUE = 42
sys.modules[virtual.__name__] = virtual
