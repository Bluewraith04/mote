def square(x):
    return x * x

def sum_of_squares(a, b):
    return square(a) + square(b)

def fib(n):
    if n <= 1:
        return n
    return fib(n - 1) + fib(n - 2)

def run():
    r2 = sum_of_squares(3, 4)
    r4 = fib(10)
    return (r2, r4)

if __name__ == '__main__':
    for _ in range(10_000):
        run()
