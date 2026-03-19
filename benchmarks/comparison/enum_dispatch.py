def eval_op(tag, a, b):
    if tag == 0:
        return a + b
    elif tag == 1:
        return a * b
    else:
        return -a

total = 0
for i in range(5000000):
    total += eval_op(0, i, i + 1)
    total += eval_op(1, i, 2)
    total += eval_op(2, i, 0)
print(total)
