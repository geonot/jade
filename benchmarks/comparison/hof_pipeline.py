def double_val(x):
    return x * 2

def add_one(x):
    return x + 1

def apply(f, x):
    return f(x)

total = 0
for i in range(10000000):
    total += apply(double_val, i)
    total += add_one(double_val(i))
print(total)
