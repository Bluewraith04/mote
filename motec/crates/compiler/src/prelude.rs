//! The implicitly available prelude: `Result` is a lang-item enum, so every compilation gets it without an `import`.

use crate::ast::Item;
use crate::lexer::Lexer;
use crate::parser::Parser;

/// The prelude declarations as source; `Option` is built in.
pub(crate) const SYNTHESIZED_PRELUDE_SRC: &str = "\
enum Result { Ok(__Any), Err(__Any) }
trait Display { fn to_string(self) -> String }
trait Debug { fn debug(self) -> String }
trait Eq { fn eq(self, other: Self) -> Bool }
trait Ord { fn compare(self, other: Self) -> Int }
trait Closeable { fn close(self) -> Result<Null, Error> }
";

/// Parses [`SYNTHESIZED_PRELUDE_SRC`] into items to prepend to a single-file program.
pub(crate) fn synthesized_items() -> Vec<Item> {
    let tokens = Lexer::new(SYNTHESIZED_PRELUDE_SRC)
        .tokenize()
        .expect("prelude source lexes");
    Parser::new(tokens)
        .parse()
        .expect("prelude source parses")
        .items
}

/// Name of the hidden generator behind `.stream()` on a list or channel.
pub(crate) const STREAM_OF_FN: &str = "__stream_of";

/// The hidden `__stream_of` function codegen appends to every program: a stream over any iterable.
pub(crate) fn stream_of_item() -> Item {
    let src = "fn __stream_of(src: __Any) -> Stream<__Any> { for x in src { yield x } }";
    let tokens = Lexer::new(src).tokenize().expect("stream_of source lexes");
    Parser::new(tokens)
        .parse()
        .expect("stream_of source parses")
        .items
        .remove(0)
}
