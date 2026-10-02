import time
import subprocess
import sys
import os

# Import Python benchmarks
sys.path.insert(0, os.path.join(os.path.dirname(__file__), 'python'))
import arithmetic_comparison
import control_flow
import function_calls
import objects

def time_python(fn, iterations):
    start = time.perf_counter()
    for _ in range(iterations):
        fn()
    end = time.perf_counter()
    total_sec = end - start
    avg_us = (total_sec / iterations) * 1e6
    return total_sec, avg_us

def main():
    print('========================================================================')
    print('          Mote VM vs CPython Baseline Benchmarking Report               ')
    print('========================================================================')
    print()
    print('Measuring CPython 3.x execution times across workloads...')
    
    iters_arith = 100_000
    iters_ctrl = 100_000
    iters_call = 10_000
    iters_obj = 100_000

    t_py_arith, avg_py_arith = time_python(arithmetic_comparison.run, iters_arith)
    t_py_ctrl, avg_py_ctrl = time_python(control_flow.run, iters_ctrl)
    t_py_call, avg_py_call = time_python(function_calls.run, iters_call)
    t_py_obj, avg_py_obj = time_python(objects.run, iters_obj)

    print(f'CPython - Arithmetic & Comparison:  {avg_py_arith:8.3f} us/iter ({iters_arith} iters)')
    print(f'CPython - Control Flow (Loop):      {avg_py_ctrl:8.3f} us/iter ({iters_ctrl} iters)')
    print(f'CPython - Function Calls (Fib 10):  {avg_py_call:8.3f} us/iter ({iters_call} iters)')
    print(f'CPython - Objects (Alloc & Walk):   {avg_py_obj:8.3f} us/iter ({iters_obj} iters)')
    print()

if __name__ == '__main__':
    main()
