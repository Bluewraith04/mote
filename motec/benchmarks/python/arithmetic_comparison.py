def run():
    r0 = 100
    r1 = 30
    r2 = r0 + r1
    r3 = r0 - r1
    r4 = r0 * r1
    r5 = r0 // r1
    r6 = r0 % r1
    r7 = -r0

    r8 = 0x0F
    r9 = 0x33
    r10 = r8 & r9
    r11 = r8 | r9
    r12 = r8 ^ r9
    r13 = ~r8
    r14 = 2
    r15 = r8 << r14
    r16 = r8 >> r14

    r17 = (r0 == r1)
    r18 = (r0 != r1)
    r19 = (r1 < r0)
    r20 = (r0 <= r0)
    r21 = (r0 > r1)
    r22 = (r1 >= r0)

    r23 = (r18 and r19)
    r24 = (r17 or r18)
    r25 = (r18 ^ r19)
    r26 = (not r17)

    r27 = r2
    r28 = 3.14159
    r29 = 2.71828
    r30 = r28 + r29

if __name__ == '__main__':
    for _ in range(100_000):
        run()
