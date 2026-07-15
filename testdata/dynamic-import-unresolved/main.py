import importlib


def load_module(module_name):
	return importlib.import_module(module_name)


mod = load_module("pkg.helper")
print(mod.VALUE)
