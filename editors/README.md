# Editors

## Tree-sitter grammar

`tree-sitter-mote/` is the grammar of `syntax/Syntax.md`; `src/` is generated from `grammar.js` and `src/scanner.c` is the external scanner.

| Task | Command |
|---|---|
| Regenerate the parser | `tree-sitter generate` in `tree-sitter-mote/` |
| Run the corpus | `tree-sitter test -p editors/tree-sitter-mote` |
| Parse every `.mote` file and `mote` block of the repository | `tools/check-grammar.sh` |

`test/corpus/accepted.txt` and `rejected.txt` are the node-rule cases of `syntax/grammar.cases` that the grammar agrees on; a rejected input is marked `:error`. After a grammar change, rebuild the trees with `tree-sitter test --update` and read the diff.

## Zed extension

`zed/` is the extension: `extension.toml`, and `languages/mote/` with `config.toml` and the queries `highlights`, `brackets`, `indents`, `outline` and `overrides`. `tools/check-zed.sh` checks that each query compiles against the grammar and matches a real file. To try it, run `zed: install dev extension` and pick `editors/zed`.

Zed builds the grammar from the `repository` and `rev` of `[grammars.mote]` in `extension.toml`; `rev` is a commit of the public repository, so after the grammar changes, export to the public repository and pin the new commit.

## Known gaps

The grammar accepts six inputs the compiler's parser refuses; the compiler still refuses each.

| Input | Tree-sitter | Why |
|---|---|---|
| `a.fn`, `let if = 1`, `-> self` | parsed as an identifier | a keyword lexes as `identifier` where only an identifier is valid |
| `@` then a newline then `fn f() {}` | an attribute named by the next line | newlines are whitespace |
| `let a = if x { 1 }` | an `if_expression` | statement and value `if` are one node, so a missing `else` is not an error |
| `[1, 2,]` after a line at top level | an index expression | the grammar has no newline token; `(` is told apart by the scanner, `[` is not |
