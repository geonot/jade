total = 0
for i in range(10000000):
    x, y, z = i ^ total, i + 1, i + 2
    x2, y2, z2 = i + 3, i + 4, i + 5
    total += x * x2 + y * y2 + z * z2
print(total)
