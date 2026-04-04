def gcd(a, b):
    while b != 0:
        a, b = b, a % b
    return a

total = 0
for i in range(1, 10000):
    j = i + 1
    while j < 10000:
        total += gcd(i, j)
        j += 10
print(total)
