# Editor support

| Directory | What |
|---|---|
| `tree-sitter-mote/` | Tree-sitter grammar (`grammar.js`, generated `src/`, corpus tests in `test/corpus/`) |
| `zed/` | Zed extension: `extension.toml` and `languages/mote/` queries |

## Install in Zed

1. Run `zed: install dev extension` and pick the `editors/zed` folder.
2. Zed clones the grammar from `extension.toml` (`repository` at `rev`) and compiles it.

Zed fetches the grammar from git, so `rev` must name a pushed commit. After changing `grammar.js`, regenerate `src/`, commit, push, and bump `rev` to that commit.

## Develop the grammar

```bash
cd editors/tree-sitter-mote
tree-sitter generate
tree-sitter test
```

Queries live in `zed/languages/mote/`; `tree-sitter.json` points the CLI at the same `highlights.scm`.

## Known gaps

- Block comments do not nest (the compiler's do).
- `x?` and `a ? b : c` are one ambiguity the compiler settles by a colon lookahead; here both readings run and the colon picks one. Tree shapes for `a + b?` can differ from the compiler's.
