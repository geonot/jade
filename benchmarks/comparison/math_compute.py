total = 0
for i in range(1, 10001):
    for j in range(1, 10001):
        total += i * j + i - j
print(total)
