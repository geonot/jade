total = 0
for i in range(10000000):
    arr = [i, i + 1, i + 2, i + 3, i + 4]
    total += arr[0] + arr[1] + arr[2] + arr[3] + arr[4]
print(total)
