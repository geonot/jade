def apply(f, x):
    return f(x)

base = 100
adder = lambda x: base + x

total = 0
for i in range(10000000):
    total += adder(i)
    total += apply(adder, i)
    total ^= i
print(total)
