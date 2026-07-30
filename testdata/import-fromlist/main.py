module = __import__("pkg", fromlist=("sub",))

print(module.sub.VALUE)
