# Mote — A Minimal Interpreted Language

> *"Write fast, think clean, build small."*

Mote (formerly Dust) is a small interpreted programming language written in Python. It's designed to be understandable, hackable, and expressive with minimal syntax and maximum readability. Mote employs Python-style significant indentation rather than curly brace blocks.

---

## Why Mote?

Mote is a toy language built for:

- Learning how interpreters and parsers work
- Experimenting with language design
- Creating small, readable programs without boilerplate
- Exploring implementation tradeoffs

---

## Features

- Easy-to-understand, clean syntax (Python-style whitespace blocks)
- Variables, functions, and conditionals
- While loops and scoping
- A working REPL with multiline support
- Simple interpreter backend in Python
- Managed elegantly via `uv`

---

## Installation

Mote uses `uv` for lightning-fast Python dependency management. Make sure you have `uv` installed.

```bash
git clone https://github.com/Bluewraith04/mote.git
cd mote
uv sync
```

---

## Usage

Start the REPL:

```bash
uv run mote
```

Run a script file:

```bash
uv run mote path/to/file.mote
```

---

## Language Overview

```mote
// Hello world
print("Hello, Mote!");

// Variables
let x = 10;
let y = x + 5;

// Functions
fn add(a, b)
    return a + b;

print(add(3, 4));

// Conditionals
if x > 5
    print("x is large");
else
    print("x is small");

// Loops
let i = 0;
while i < 5
    print(i);
    i = i + 1;
```

---

## Project Structure

```text
mote/
├── mote/             # Core interpreter code
│   ├── ast_nodes.py
│   ├── lexer.py      # Contains the custom IndentationLexer
│   ├── parser.py
│   ├── env.py
│   ├── utils.py
│   ├── stdlib.py
│   ├── interpreter.py
│   ├── repl.py
│   └── main.py
├── README.md
└── pyproject.toml    # uv-managed packaging config
```


## Contributing

If you want to extend Mote, hack the parser, or build your own features, start by reading the code in the `mote/` directory. Contributions, forks, and experiments are all welcome.

---

## License

MIT License. Mote is free to use, learn from, or tear apart for your own language ideas.
