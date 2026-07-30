import inspect

import helper

print(helper.__doc__)
print(helper.annotation_name())
print(helper.__file__.endswith("helper.py"))
print(helper.__spec__.origin.endswith("helper.py"))
print("def annotated" in inspect.getsource(helper))
