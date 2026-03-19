def a_elem(i, j):
    return ((i + j) * (i + j + 1)) // 2 + i + 1

n = 1000
s = 0
for _iter in range(500):
    for i in range(n):
        acc = 0
        for j in range(n):
            acc += a_elem(i, j) * (j + 1)
        s += acc % 1000000
print(s)
