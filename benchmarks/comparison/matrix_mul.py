n = 800
total = 0
for i in range(n):
    for j in range(n):
        s = 0
        for k in range(n):
            s += (i * n + k) * (k * n + j)
        total += s
print(total)
