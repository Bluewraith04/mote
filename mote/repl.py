from .lexer import lexer
from .interpreter import Interpreter
from .parser import parser
from .utils import BlockTracker

class REPL:
    def __init__(self):
        self.runtime = Interpreter()
        self.tracker = BlockTracker()
        self.code = ''
        self.awaiting_more_input = False
        self.is_active = True

    def init(self):
        print("Mote 0.2.0 REPL (type 'exit()' to quit)")

    def loop(self):
        self.init()
        while self.is_active:
            self.read_input()
            if not self.awaiting_more_input and self.is_active:
                self.evaluate_code()

    def read_input(self):
        try:
            prompt = ">>> " if not self.awaiting_more_input else "... "
            line = input(prompt)

            if line.strip().lower() == "exit()":
                self.shutdown()
                return

            self.code += line + '\n'
            self.awaiting_more_input = self.tracker.check_string(self.code)

        except (EOFError, KeyboardInterrupt):
            self.shutdown()

    def evaluate_code(self):
        if not self.code.strip():
            self.code = ''
            return

        try:
            program = parser.parse(self.code, lexer=lexer)
            if not program: return
            result = self.runtime.eval(program.declarations[0])
            self.display_result(result)
        except Exception as e:
            # import traceback
            # traceback.print_exc()
            print(e)
        finally:
            self.code = ''
            self.awaiting_more_input = False

    def display_result(self, result):
        if result is not None:
            print(result)

    def shutdown(self):
        self.is_active = False
        print("Exiting Mote REPL...")
        
            
                
if __name__ == "__main__":
    r = REPL()
    r.loop()
    