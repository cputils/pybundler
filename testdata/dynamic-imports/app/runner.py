import importlib as il
from builtins import __import__ as builtin_import
from importlib import import_module as load_module

TARGET = __package__ + ".helper1"


def run():
	first = il.import_module(TARGET)
	second = load_module(".helper2", package=__spec__.parent)
	third = builtin_import("helper3", globals(), locals(), ["VALUE"], 1)
	print(first.VALUE + ":" + second.VALUE + ":" + third.VALUE)
