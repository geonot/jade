def collatz_steps(n):
    steps = 0
    while n != 1:
        if n % 2 == 0:
            n = n // 2
        else:
            n = 3 * n + 1
        steps += 1
    return steps

max_steps = 0
max_n = 0
for n in range(1, 1000000):
    s = collatz_steps(n)
    if s > max_steps:
        max_steps = s
        max_n = n
print(max_n)
print(max_steps)
