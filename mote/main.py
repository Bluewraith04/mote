import sys
from .interpreter import Interpreter
from .repl import REPL

def run_file(path):
    try:
        with open(path, 'r') as file:
            source = file.read()
        runtime = Interpreter()
        runtime.execute(source)
    except FileNotFoundError:
        print(f"Error: File '{path}' not found.")
        sys.exit(1)
    except Exception as e:
        print(e)
        sys.exit(1)

def main():
    args = sys.argv[1:]
    if len(args) == 0:
        r = REPL()
        r.loop()
    elif len(args) == 1:
        run_file(args[0])
    else:
        print("Usage: mote [script]")
        sys.exit(64)
    