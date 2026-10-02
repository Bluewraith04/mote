def run():
    r0 = 1
    r1 = 20
    r2 = 0
    r3 = 1
    r4 = 2
    r5 = 0

    while r0 <= r1:
        r7 = r0 % r4
        if r7 == r5:
            r2 += r0
        r0 += r3
    return r2

if __name__ == '__main__':
    for _ in range(100_000):
        run()
