import importlib

helper = importlib.import_module(__spec__.name + ".helper")
print(helper.VALUE)
