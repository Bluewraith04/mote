# Core libraries

The libraries that ship with Mote and are not in the runtime. Each is a Mote package in `mote_std/<name>/` with `mote.toml`, `src/`, `tests/` and `README.md`, and a shared library in `lib/` when it has one.

## Rules

* A requirement is met when its `Done when` command passes. Each library's command is the proof of the slice named in its section, in `tasks.md`.
* The runtime holds the language's needs only (rule 9 of `tasks.md`). Anything that is not language-specific is a library here.
* An external library that only calls the system (`fs`, `env`, `process`, `random`, `signal`, `term`, `net.socket`) is Zig in `lib/` and makes system calls directly. One that wraps a library of formats or protocols (`compress`, `regex`, `net.tls`, `crypto`, `sqlite`) is a `rust/` crate of the library's own and may link the Rust standard library. <!-- test: core_fs_the_tests_of_the_library_pass -->
* A Zig library is built with `zig build-lib` without a panic handler, stack probes or threads, so its archive links into an executable with nothing beyond libc. It reports a failure as the negated `errno` and writes results into an array the caller gives, because a string result is copied once and never freed. <!-- test: core_fs_a_built_program_links_the_archive_and_reads_a_file -->
* An external library reaches the runtime through `abi/mote.h`, generated from the host table (S121): port push and close, and `fd_wait` on the calling thread, so a call that waits is `@blocking`. It does not allocate Mote strings or arrays; they cross by copy. <!-- test: host_header_file_is_the_generated_text, host_header_compiles_and_matches_the_layout_of_the_table -->
* A library with external functions has `lib/build.sh`, which writes `lib<name>.so` (and `lib<name>.a`) to `$MOTE_LIB_OUT`, or a `rust/` crate that builds both; `tools/build-std-libs.sh` runs it and `--check` lists a library that declares `extern` and has no builder (S122). Built libraries are never committed. <!-- test: core_tools_check_passes_when_each_external_library_has_a_build_script, core_tools_check_names_a_library_whose_external_library_has_no_build_script, core_tools_build_writes_the_shared_library_and_the_archive -->
* A library is imported by its name with no manifest entry, and the name is reserved (S123). `tools/test-core.sh NAME` tests it, after building the shared libraries of NAME and of every library it imports. <!-- test: core_embed_an_import_of_a_core_name_adds_the_library_with_no_manifest_entry, core_tools_test_core_builds_the_library_then_runs_the_tests, core_tools_test_core_builds_the_external_libraries_it_imports -->
* A program that imports a library finds its shared library in `<mote_std>/<name>/lib/<arch>-linux/`, populated by `./compile-std.sh` and copied to `~/.mote/mote_std/` by `./install.sh` (S166). <!-- test: core_a_custom_home_with_mote_std_provides_its_libraries_from_disk, lib_finds_the_library_in_the_lib_directory_of_the_package_that_declares_it -->
* A library's `src/lib.mote` holds its module doc and `pub import` lines only; the public items live in files named for what they are. <!-- test: core_layout_lib_mote_holds_only_reexports -->
* External `extern` declarations live in `_lib.mote` or any module file; when placed in `_lib.mote` (which starts with `_` and is private to its package), it holds external declarations and constants only (S161). <!-- test: core_layout_externs_follow_lib_convention -->
* A source file is at most 450 lines; a longer one is split by topic. <!-- test: core_layout_no_file_is_longer_than_450_lines -->
* `core/` stays in this repository until the language is stable, then moves to its own repository with its history.
* A new library needs the user's go for its scope.

## Order

Libraries are built in the order of this file, which is the order of slices S127 to S156.

## path

Text operations on `/`-separated paths.

| Provides | Join, split, normalize, parent, file name, extension, relative-to |
|---|---|
| Native | none |
| Slice | S127 |

* A path is text: nothing here reads the file system, and a name is whatever lies between `/`. <!-- test: core_path_the_tests_of_the_library_pass -->
* `join(a, b)` is `b` when it is absolute; `split` starts an absolute path's list with `/`; `normalize` cancels `..` where it can and gives `.` for nothing. <!-- test: core_path_the_tests_of_the_library_pass -->
* `parent`, `file_name`, `extension` and `relative_to` give `null` when there is no answer; `relative_to` never writes `..`. <!-- test: core_path_the_tests_of_the_library_pass -->
* A program imports it as `import { join } from path`, with no manifest entry. <!-- test: core_path_a_program_imports_it_with_no_manifest_entry -->

Done when: `tools/test-core.sh path` passes.

## date

Calendar dates and times of day. There is no clock and no time zone; `std.time` has the scheduler clock.

| Provides | `Date`, `TimeOfDay` and `DateTime`; days between dates, weekday, formatting and parsing of ISO 8601 |
|---|---|
| Native | none |
| Slice | S128 |

* `Date.new(year, month, day)` is `null` for a day that does not exist; dates are proleptic Gregorian and count days from 1970-01-01 (`days`, `from_days`, `add_days`, `days_since`). <!-- test: core_date_the_tests_of_the_library_pass -->
* `weekday()` is a `Weekday` (`number()` is 1 for Monday to 7 for Sunday). <!-- test: core_date_the_tests_of_the_library_pass -->
* `TimeOfDay` is hour, minute and second with no leap second; `DateTime` is a date with a time and counts seconds from 1970-01-01T00:00:00 (`seconds`, `from_seconds`, `add_seconds`). <!-- test: core_date_the_tests_of_the_library_pass -->
* Each prints and `parse`s as `YYYY-MM-DD`, `HH:MM:SS` and `YYYY-MM-DDTHH:MM:SS`; `parse` is an `Err` for any other text. <!-- test: core_date_the_tests_of_the_library_pass -->

Done when: `tools/test-core.sh date` passes.

## log

Leveled, structured logging on one logger.

| Provides | Levels, key-value fields, a sink per logger, a stream sink for `std.io` |
|---|---|
| Native | none |
| Slice | S129 |

* A `Logger` has a name, a `Level` (`Debug`, `Info`, `Warn`, `Error`; `Info` at first) and a sink, a function that takes a `Record`; `debug`, `info`, `warn`, `error` and `log` pass a record to it when the level is at least the logger's. <!-- test: core_log_the_tests_of_the_library_pass -->
* A `Record` has the level, the logger's name, the message and the key-value fields; `format()` is `LEVEL name: message key=value`, with a value quoted when it is empty or holds a space, `=` or `"`. <!-- test: core_log_the_tests_of_the_library_pass -->
* `Logger.to_stdout(name)` and `Logger.to_stderr(name)` have a sink that prints the formatted record with `println` or `eprintln`. <!-- test: core_log_the_tests_of_the_library_pass -->
* A record has no time, because there is no clock (see `date`). <!-- test: core_log_the_tests_of_the_library_pass -->

Done when: `tools/test-core.sh log` passes.

## fs

Files, directories and metadata. A file is `Closeable`.

| Provides | Read, write, append, open, list, create, remove, rename, copy, metadata |
|---|---|
| Native | Zig, system calls |
| Slice | S130 |

* `read_string`, `read_bytes`, `write_string`, `write_bytes`, `append_string` and `append_bytes` take a path and finish the whole file; text that is not UTF-8 is an `InvalidData` `Err`. <!-- test: core_fs_the_tests_of_the_library_pass -->
* `File.open`, `create`, `create_new` and `append` give a `File`, which reads, writes, seeks, truncates, gives its metadata and is `Closeable`; a second `close` is an `Err` and never closes another file. <!-- test: core_fs_the_tests_of_the_library_pass -->
* `metadata` and `link_metadata` give the kind, size, change time in nanoseconds and permission bits; `exists`, `is_file` and `is_dir` answer for a path. <!-- test: core_fs_the_tests_of_the_library_pass -->
* `list` gives the names of a directory sorted, without `.` and `..`; `create_dir`, `create_dir_all`, `remove_file`, `remove_dir`, `remove_all`, `rename` and `copy` change it. <!-- test: core_fs_the_tests_of_the_library_pass -->
* A failure is an `Err` whose kind follows the `errno` (`NotFound`, `PermissionDenied`, `AlreadyExists`, `WouldBlock`, `Interrupted`, else `Other`) and whose message is `path: reason`. <!-- test: core_fs_the_tests_of_the_library_pass -->
* A program that is built links the archive and runs with no file beside it. <!-- test: core_fs_a_built_program_links_the_archive_and_reads_a_file -->

Done when: `tools/test-core.sh fs` passes.

## env

Environment variables and the working directory. Arguments and `exit` are `std.io`.

| Provides | Get, set, remove, list variables; current directory |
|---|---|
| Native | Zig, the `environ` of libc and system calls |
| Slice | S131 |

* `get(name)` is the value or `null` when the variable is not set or its value is not UTF-8; `get_or` takes a fallback and `has` is true for a variable set to an empty value. <!-- test: core_env_the_tests_of_the_library_pass -->
* `set` and `remove` change this process and the programs it starts; a name that is empty or has `=` in it is an `Err`, and removing a variable that is not set is not. <!-- test: core_env_the_tests_of_the_library_pass -->
* `vars()` gives every variable as a name and a value, leaving out one that is not UTF-8. <!-- test: core_env_the_tests_of_the_library_pass -->
* `current_dir()` and `set_current_dir(path)` read and change the working directory of the process. <!-- test: core_env_the_tests_of_the_library_pass -->
* A program that is built links the archive and reads the environment of its process. <!-- test: core_env_a_built_program_links_the_archive_and_reads_the_environment -->

Done when: `tools/test-core.sh env` passes.

## process

Run another program.

| Provides | Spawn with arguments, pipes for the three streams, wait, exit status |
|---|---|
| Native | Zig, `posix_spawnp` from libc and system calls |
| Slice | S132 |

* `command(program)` is a `Command`; `arg`, `with_args`, `set_env`, `without_inherited_env`, `in_dir`, `with_stdin`, `with_stdout` and `with_stderr` each give a changed copy. A stream is `Stdio.Inherit`, `Piped`, `Null` or, for standard error only, `Merged` into standard output. <!-- test: core_process_the_tests_of_the_library_pass -->
* `start()` runs the program (found on `PATH` when its name has no `/`) with the environment of this program, extended or replaced by `set_env`, and gives a `Child`; a missing program, directory or permission is an `Err` of kind `NotFound` or `PermissionDenied` with the program's name. <!-- test: core_process_the_tests_of_the_library_pass -->
* A `Child` writes (`write`, `write_string`, `close_stdin`) and reads (`read`, `read_all`, `read_stderr`, `read_all_stderr`; none at the end) through its pipes, waits (`wait`, which gives the same `Status` again; `try_wait`, which is `null` while the process runs), is signalled (`signal`, `kill`) and is `Closeable`: closing closes the pipes and does not wait. <!-- test: core_process_the_tests_of_the_library_pass -->
* A `Status` is the exit `code()` or the `signal()` that ended the process, and `success()` for code 0. <!-- test: core_process_the_tests_of_the_library_pass -->
* `output()` and `output_with(input)` run a program to its end and give its `Status` with the bytes of standard output and error, reading both and writing the input at the same time, so large streams do not stall; `status()` runs it with its streams as they are. <!-- test: core_process_the_tests_of_the_library_pass -->
* A call that waits (`wait`, `read`, `write`) parks its task while a helper thread waits, so as many children as tasks run together. A write to a child that has closed its input is an `Err`, and the program is not stopped by `SIGPIPE`, which `start` sets to ignored for this program and leaves at its default in the child. <!-- test: core_process_the_tests_of_the_library_pass -->
* A program that is built links the archive and runs a child. <!-- test: core_process_a_built_program_links_the_archive_and_runs_a_child -->

Done when: `tools/test-core.sh process` passes.

## random

A seeded generator and the system source.

| Provides | A generator from a seed, integers in a range, floats, shuffles, system bytes |
|---|---|
| Native | Zig, `getrandom`; the generator is Mote |
| Slice | S133 |

* `Random.seeded(seed)` is xoshiro256++ with its state from splitmix64 of the seed, so a seed gives the numbers the Zig standard library's generator gives; it is not for secrets. <!-- test: core_random_the_tests_of_the_library_pass -->
* `below(limit)` and `between(low, high)` give every value in the range with equal chance, by rejection; `float()` is in `[0, 1)` with 53 bits. <!-- test: core_random_the_tests_of_the_library_pass -->
* `shuffled(items)` is a copy in a random order, `choose(items)` is one item or `null`, `bool()` and `bytes(count)` draw from the same generator. <!-- test: core_random_the_tests_of_the_library_pass -->
* `system_bytes(count)`, `system_int()` and `Random.system()` read the system's source, which is the one for keys and seeds. <!-- test: core_random_the_tests_of_the_library_pass -->
* A program that is built links the archive and draws from the system's source. <!-- test: core_random_a_built_program_links_the_archive_and_draws_bytes -->

Done when: `tools/test-core.sh random` passes.

## signal

Signals delivered as a `Receiver`.

| Provides | Subscribe to a signal, receive it as a value |
|---|---|
| Native | Zig: a handler that writes the signal number into a pipe, and system calls |
| Slice | S134 |

* `subscribe(signals)` watches `Hangup`, `Interrupt`, `Quit`, `User1`, `User2`, `Alarm`, `Terminate`, `Child`, `Continue` or `WindowChange` and gives a `Subscription` whose `signals` is a `Receiver<Signal>`; `next()` waits for one. An empty list is an `Err`. <!-- test: core_signal_the_tests_of_the_library_pass -->
* Signals arrive in the order they were caught, and every subscription that watches a signal receives it. <!-- test: core_signal_the_tests_of_the_library_pass -->
* `close()` stops a subscription: the receiver gives what was already caught and then ends, a second close is an `Err`, and a signal goes back to its default when no subscription watches it. <!-- test: core_signal_the_tests_of_the_library_pass -->
* A task that waits for a signal parks and does not hold a worker. <!-- test: core_signal_the_tests_of_the_library_pass -->
* `pid()` is the process id and `send(pid, signal)` sends a signal, to this program too, and the handler of a signal to this program has run when `send` returns, so such signals are never reordered or merged; a process that does not exist is an `Err`. <!-- test: core_signal_the_tests_of_the_library_pass -->
* A program that is built links the archive and receives a signal. <!-- test: core_signal_a_built_program_links_the_archive_and_receives_a_signal -->

Done when: `tools/test-core.sh signal` passes.

## term

The terminal under `tile`.

| Provides | Size, raw mode, escape output, key and mouse input |
|---|---|
| Native | Zig, `ioctl`, `poll`, `read` and `write` on descriptors |
| Slice | S135 |

* A `Terminal` is two descriptors, input and output (`Terminal.stdio()` is 0 and 1); `is_tty()` and `size()` ask the kernel, and `raw()` stops echo, line editing and the signals keys raise until `close()` gives back the settings it replaced. <!-- test: core_term_the_tests_of_the_library_pass -->
* `read_event(timeout_ms)` is the next key or mouse event, `null` after the time, and an `Err` at the end of the input; a lone Escape is told from the start of a sequence by waiting 30 ms for more. <!-- test: core_term_the_tests_of_the_library_pass -->
* Keys decode from plain bytes, UTF-8, control bytes, `CSI` and `SS3` sequences with the xterm modifier parameter (`Shift`, `Alt`, `Ctrl`), and an escape before a key as `Alt`; mouse reports decode from the SGR form with the cell counted from 0 at the top left. <!-- test: core_term_the_tests_of_the_library_pass -->
* `decode_input(bytes)` is the first event and the bytes it took, 0 bytes when the sequence stops short and no event for bytes that mean nothing. <!-- test: core_term_the_tests_of_the_library_pass -->
* The escape output functions give the text for clearing, moving the cursor, hiding it, the alternate screen, mouse reporting, and the colours and attributes of the text. <!-- test: core_term_the_tests_of_the_library_pass -->
* A `Terminal` writes the same sequences as methods: `print(span)`, `clear()`, `goto(row, col)`, `cursor(visible)`, `alternate_screen(on)` and `mouse(on)`. <!-- test: core_term_the_tests_of_the_library_pass -->
* `paint(text)` is a `Span` that chains `fg`, `bg`, `bold`, `dim`, `italic`, `underline` and `reverse`; in a string it is the text between the codes that set its `Style` and the code that resets it. `Style` is the colours and attributes as data, which `tile` draws with. <!-- test: core_term_the_tests_of_the_library_pass -->
* A `Pty` is a pseudo-terminal: `terminal()` is its terminal side, `send` types at it, `read` takes what was written to it and `resize` gives it a size, so a terminal program is tested with no terminal. <!-- test: core_term_the_tests_of_the_library_pass -->
* A program that is built links the archive and reads keys from a pseudo-terminal. <!-- test: core_term_a_built_program_links_the_archive_and_reads_a_key -->

Done when: `tools/test-core.sh term` passes.

## net

All networking in one library: sockets, TLS, and the HTTP and WebSocket layer above them. A blocked read, write or accept waits in a `@blocking` external function on a helper thread, so other tasks run. It is built in three slices, each adding a module to the same package.

| Provides | `net.socket`: connect, listen, accept, read, write, close, UDP send and receive (S136); `net.tls`: a client and a server stream over a socket, certificate checking (S137); `net.http`: `Server<S>` with handlers over shared state, an HTTP and HTTPS client, WebSocket frames (S153) |
|---|---|
| External library | Zig `libnet` for `net.socket`, system calls and the resolver of libc; Rust `libnettls` (rustls, ring, bundled roots) in `rust/` for `net.tls`; `lib/build.sh` builds both |
| Slice | S136, S137, S153 |

* `TcpStream.connect(host, port)` resolves a name and tries each address in turn, and `connect_within` gives up on each after a time; a refused connection, a name that does not resolve (`NotFound`), a time that runs out (`TimedOut`) and an address in use (`AlreadyExists`) are an `Err` that names what was asked. <!-- test: core_net_the_tests_of_the_library_pass -->
* A stream reads as soon as some bytes arrive and gives 0 bytes at the end of the stream; `read_all` reads to the end, `write` writes all of its bytes, `shutdown_write` tells the peer no more come, and `set_timeout` limits each read and write. <!-- test: core_net_the_tests_of_the_library_pass -->
* `TcpListener.bind(host, port)` listens, port 0 picks a free port that `port()` tells, and `accept()` waits for a connection, up to the listener's time. <!-- test: core_net_the_tests_of_the_library_pass -->
* A wait in one task does not stop the others, even when more tasks wait than the program has workers, and a transfer larger than the socket buffers arrives whole while the other side reads. <!-- test: core_net_the_tests_of_the_library_pass -->
* `UdpSocket.bind` and `send_to` send a datagram to a name and a port, and `recv_from` puts the next one in a buffer and tells its size and sender as `address:port`. <!-- test: core_net_the_tests_of_the_library_pass -->
* A program that is built links the archive and the resolver of libc, and talks over a socket. <!-- test: core_net_a_built_program_links_the_archive_and_talks_over_a_socket -->
* `TlsStream.connect(host, port)` connects and shakes hands over TLS 1.2 or 1.3, checking the certificate against the bundled public roots and against `host` (a name or an address); `connect_trusting` adds the PEM certificates it is given, and `client` runs the handshake over a stream that is already connected. <!-- test: core_net_the_tests_of_the_library_pass -->
* A certificate that no trusted root vouches for, one that does not carry the name asked, and a peer that does not speak TLS are an `Err` that starts `tls: `, and the stream is closed. <!-- test: core_net_the_tests_of_the_library_pass -->
* `TlsAcceptor.new(chain, key)` takes PEM text, is an `Err` when either is not usable, and `accept(stream)` runs the server handshake over an accepted stream. <!-- test: core_net_the_tests_of_the_library_pass -->
* A `TlsStream` reads, writes, times out and closes like a `TcpStream`; closing sends the close notice, and a peer that closes without one reads as the end of the stream. <!-- test: core_net_the_tests_of_the_library_pass -->
* The external library holds only the TLS state and does no input or output: it takes the bytes that arrived and gives the bytes to send, and Mote moves them over the socket. Its archive keeps only its `tls_` symbols global, so the Rust standard library inside it cannot clash with the runtime's. <!-- test: core_net_a_built_program_links_the_tls_archive_and_shakes_hands -->
* `get`, `post` and `send(method, address, headers, body, options)` read a whole HTTP/1.1 response from an `http://` or `https://` address into a `Response` of the status, headers (`(name, value)` pairs, names in lower case) and body; a body with a length, a chunked body and one that ends with the connection are all read, and a response to `HEAD`, a 204 and a 304 have none. <!-- test: core_net_the_tests_of_the_library_pass -->
* `options` is the last parameter of `get`, `post`, `send` and `connect_websocket` and defaults to `Options()`; the `headers` and `body` of `send` default to none. The error builder of the sockets and the handshake answer are in `_` files, so only the package reaches them. <!-- test: core_net_the_tests_of_the_library_pass -->
* `Options` gives the time for connecting and for each read and write (an `Err` of `TimedOut` past it), the redirects followed (a 303, and a 301 or 302 after a POST, become a GET), extra trusted PEM roots, and the largest body read. <!-- test: core_net_the_tests_of_the_library_pass -->
* `parse_url` splits `http`, `https`, `ws` and `wss` addresses into secure flag, host (an IPv6 address in brackets), port and target, and `url_encode` and `url_decode` convert `%XX` escapes. <!-- test: core_net_the_tests_of_the_library_pass -->
* `Server.new(handler)` serves stateless requests, and `Server.with_state(state, handler, sockets = null)` keeps `state` in a `Mutex` and gives each request, in a task of its own for each connection, to `handler(state, request)`; `serve(listener)` accepts until `stop`, and `with_tls`, `with_sockets` and `with_limits` give the server TLS, a WebSocket handler and its time and body limits. <!-- test: core_net_the_tests_of_the_library_pass -->
* A `Request` has the method, target, path, query, version, headers, body and peer; `header`, `query_param`, `segments`, `route(method, "/users/:id")` (the captured parts, or null), `body_text` and `body_json` read it. A `Response` is made by `text`, `html`, `json`, `bytes`, `empty` or `redirect` and takes `with_header`; the server adds `content-length` and `connection`. <!-- test: core_net_the_tests_of_the_library_pass -->
* A connection stays open for the next request unless either side asks to close; a request body with a length or chunked is read, `expect: 100-continue` is answered, a `HEAD` runs the handler as a `GET` and sends no body, and a malformed request, a body over the limit and a line or header count over the limit are answered 400, 413 and 431. <!-- test: core_net_the_tests_of_the_library_pass -->
* A handler that answers `Response.upgrade_to_websocket()` to a valid upgrade request turns the connection into a `WebSocket` for the handler given to `with_sockets`; any other request for an upgrade is answered 400. <!-- test: core_net_the_tests_of_the_library_pass -->
* `connect_websocket(address, options)` shakes hands over `ws://` or `wss://` and checks the server's accept key; a `WebSocket` sends text, binary, ping and close, and `recv` gives the next text or binary message (fragments joined), answers pings, skips pongs, and gives null after the peer's close. A frame with reserved bits, the wrong masking, an unknown opcode or text that is not UTF-8 closes the socket with an `Err`. <!-- test: core_net_the_tests_of_the_library_pass -->

Done when: `tools/test-core.sh net` passes.

## regex

Regular expressions.

| Provides | Compile, match, find, captures, replace |
|---|---|
| Native | Rust `libregex` (`rust/`): the `regex` crate |
| Slice | S138 |

* `Regex.new(pattern)` compiles a pattern in the syntax of the Rust `regex` crate (Unicode classes, `(?i)` and other flags, named groups, no look-around and no back-references) or gives an `Err` with the crate's message; `close()` frees it and a second close is an `Err`. <!-- test: core_regex_the_tests_of_the_library_pass -->
* A search takes time proportional to the length of the text, whatever the pattern. <!-- test: core_regex_the_tests_of_the_library_pass -->
* `is_match`, `find`, `find_all`, `captures` and `captures_all` search the text; a `Match` has its text and its `start` and `end` in bytes of the UTF-8 text. <!-- test: core_regex_the_tests_of_the_library_pass -->
* A `Captures` has the groups by number (`get`, `group`, group 0 is the whole match) and by name (`named`); a group that did not take part is `null`. <!-- test: core_regex_the_tests_of_the_library_pass -->
* Matches do not overlap, and an empty match next to the one before it is skipped. <!-- test: core_regex_the_tests_of_the_library_pass -->
* `replace` and `replace_all` expand `$1`, `${1}`, `$name` and `${name}` in the template (a missing group is empty) and `$$` as `$`; `split` gives the pieces between the matches, leaving out an empty match at either end of the text. <!-- test: core_regex_the_tests_of_the_library_pass -->
* A program that is built links the archive and matches. <!-- test: core_regex_a_built_program_links_the_archive_and_matches -->

Done when: `tools/test-core.sh regex` passes.

## base64

Base64 codecs.

| Provides | Standard and URL-safe alphabets, with and without padding |
|---|---|
| Native | none |
| Slice | S139 |

* `encode(bytes, alphabet, padded)` is the text of the bytes and `decode(text, alphabet)` the bytes of a text; `encode_string` and `decode_string` do the same for the UTF-8 bytes of a string. <!-- test: core_base64_the_tests_of_the_library_pass -->
* `Alphabet.Standard` is the `+` and `/` alphabet and `Alphabet.UrlSafe` the `-` and `_` one; `padded` is true by default, and `decode` takes text with or without its padding. <!-- test: core_base64_the_tests_of_the_library_pass -->
* `decode` is an `Err` for a character outside the alphabet, a lone last character and padding that is misplaced. <!-- test: core_base64_the_tests_of_the_library_pass -->

Done when: `tools/test-core.sh base64` passes.

## uuid

Generation and parsing.

| Provides | Versions 4 and 7, parse, format, compare |
|---|---|
| Native | none |
| Slice | S140 |

* `Uuid.v4()` reads the system's random source and `v4_from(random)` a seeded generator; the version is 4 and the variant bits are `10`. <!-- test: core_uuid_the_tests_of_the_library_pass -->
* `Uuid.v7(unix_millis)` and `v7_from(unix_millis, random)` put the 48-bit time first, so identifiers sort by it; `unix_millis()` reads it back and is `null` for another version. There is no wall clock, so the caller gives the time. <!-- test: core_uuid_the_tests_of_the_library_pass -->
* `parse` takes the hyphenated form or 32 hex digits in either case and is an `Err` for other text; `to_string()` is lowercase hyphenated and `to_simple()` has no hyphens. <!-- test: core_uuid_the_tests_of_the_library_pass -->
* `bytes()`, `from_bytes`, `to_base64()` (22 URL-safe characters) and `from_base64` round trip; `compare` orders as the bytes do. <!-- test: core_uuid_the_tests_of_the_library_pass -->

Done when: `tools/test-core.sh uuid` passes.

## crypto

SHA-2 hashes, HMAC, HKDF and a constant-time comparison.

| Provides | `hash`, `Hasher`, `hmac`, `hkdf`, `equal`, hex |
|---|---|
| Native | yes (Rust: `sha2`, `hmac`, `subtle`) |
| Slice | S141 |

* `hash(algorithm, bytes)` and `hash_text` give the SHA-256, SHA-384 or SHA-512 digest (`Algorithm.size()` bytes). <!-- test: core_crypto_the_tests_of_the_library_pass -->
* A `Hasher` takes the input in pieces with `update` and gives the same digest from `finish`; closing it unfinished frees it. <!-- test: core_crypto_the_tests_of_the_library_pass -->
* `hmac(algorithm, key, data)` accepts a key of any length. <!-- test: core_crypto_the_tests_of_the_library_pass -->
* `hkdf(algorithm, secret, salt, info, length)` is RFC 5869 and is an `Err` for a length above 255 digests. <!-- test: core_crypto_the_tests_of_the_library_pass -->
* `equal(a, b)` compares in a time that depends on the lengths only. <!-- test: core_crypto_the_tests_of_the_library_pass -->
* `to_hex` writes lowercase digits and `from_hex` reads either case. <!-- test: core_crypto_the_tests_of_the_library_pass -->

Done when: `tools/test-core.sh crypto` passes.

## compress

gzip, zlib and raw deflate.

| Provides | `compress` and `decompress` of a buffer; `Encoder` and `Decoder` for a stream pushed in pieces |
|---|---|
| Native | yes (Rust: `flate2` with the pure-Rust backend) |
| Slice | S142 |

* A `Format` is `Gzip`, `Zlib` or `Deflate` (raw), and `compress(format, data, level)` followed by `decompress(format, packed)` returns the data; the level is 0 (stored) to 9, and another level is an `Err`. <!-- test: core_compress_the_tests_of_the_library_pass -->
* `Encoder.push` and `Decoder.push` return the output ready so far and `finish` the rest, so a stream works in pieces of any size. <!-- test: core_compress_the_tests_of_the_library_pass -->
* A decoder gives an `Err` for corrupt data, a failed checksum, a cut stream and data after the end of a zlib or deflate stream; gzip members one after the other are read as one stream. <!-- test: core_compress_the_tests_of_the_library_pass -->
* A decoder's `limit` stops output past that many bytes with an `Err`, for input that cannot be trusted. <!-- test: core_compress_the_tests_of_the_library_pass -->

Done when: `tools/test-core.sh compress` passes.

## archive

tar and zip.

| Provides | `read` and `write` of the entries of a tar, tar.gz or zip; `extract` and `pack` against a directory |
|---|---|
| Native | yes (Rust: `tar`, `zip`; gzip is `compress`) |
| Slice | S143 |

* An `Entry` has a path, a kind (`File`, `Directory`, `Symlink`, `Hardlink`), permission bits, a modification time in seconds, a link target and contents. <!-- test: core_archive_the_tests_of_the_library_pass -->
* `write(format, entries)` and `read(format, bytes)` round trip the entries of a `Tar`, a `TarGz` and a `Zip`, including paths over 100 bytes in a tar. <!-- test: core_archive_the_tests_of_the_library_pass -->
* `read` gives an `Err` for bytes that are not the format, and for contents over its `limit`. <!-- test: core_archive_the_tests_of_the_library_pass -->
* `read` reads a zip made by the `zip` tool. <!-- test: core_archive_the_tests_of_the_library_pass -->
* `pack(format, directory)` archives the files and directories under it in sorted order, and `extract` writes them back under another directory, returning the number of files; an entry path that is absolute or has `..` is an `Err` and nothing is written. <!-- test: core_archive_the_tests_of_the_library_pass -->

Done when: `tools/test-core.sh archive` passes.

## sqlite

SQLite databases.

| Provides | Open, execute, query, bind parameters, transactions |
|---|---|
| Native | yes (Rust: `rusqlite` with the bundled SQLite amalgamation) |
| Slice | S144 |

* `Database.open(path)` opens or makes a file, `Database.memory()` a private database in memory; `close` frees it. <!-- test: core_sqlite_the_tests_of_the_library_pass -->
* `execute(sql, params)` runs one statement with `Value`s bound to `?` in order and returns the rows it changed; `execute_batch(sql)` runs several without parameters. <!-- test: core_sqlite_the_tests_of_the_library_pass -->
* A `Value` is `Null`, `Integer`, `Real`, `Text` or `Blob`; `query(sql, params)` returns every row as a `Row` read by position or by column name. <!-- test: core_sqlite_the_tests_of_the_library_pass -->
* `begin`, `commit` and `rollback` are a transaction by hand, and `transaction(body)` commits when the body returns `Ok` and rolls back when it returns `Err`. <!-- test: core_sqlite_the_tests_of_the_library_pass -->
* An error from SQLite (a missing table, a constraint) is an `Err` with SQLite's message. <!-- test: core_sqlite_the_tests_of_the_library_pass -->
* Tasks may share one `Database`; the external library serializes their calls. <!-- test: core_sqlite_the_tests_of_the_library_pass -->

Done when: `tools/test-core.sh sqlite` passes.

## json

JSON values, and encoding and decoding of Mote types by reflection.

| Provides | A value type, parse, print, and `to_json` and `from_json` for any type that lists the traits |
|---|---|
| Native | none |
| Slice | S145 |

* `Json` is `Null`, `Boolean`, `Integer`, `Real`, `Text`, `Array` or `Object` (a list of name and value pairs, in order); `is_null`, `as_bool`, `as_int`, `as_float`, `as_text`, `at(i)` and `get(name)` read it. <!-- test: core_json_the_tests_of_the_library_pass -->
* `parse(text)` is the value or an `Err` that names the place of the first mistake; it takes the whole of RFC 8259, reads `\u` escapes (surrogate pairs too) and an integer too large for `Int` as a `Real`, and stops at 256 levels. <!-- test: core_json_the_tests_of_the_library_pass -->
* `to_string()` is the compact text and `pretty()` puts one element or member on a line, indented by two spaces. <!-- test: core_json_the_tests_of_the_library_pass -->
* `encode(value)` and `to_json()` of a type that lists `ToJson` give a struct or class as an object of its fields, a tuple, list or array as an array, an optional as its value or null, a unit variant as its name and any other variant as an object with its name as the one key. <!-- test: core_json_the_tests_of_the_library_pass -->
* A map with `String` keys is an object and a set is an array. A map with other keys, a union, a function, a handle, a `UInt64` and a float that is not finite are an `Err` to encode. <!-- test: core_json_the_tests_of_the_library_pass -->
* `decode<T>(json)` and `T.from_json(json)` of a type that lists `FromJson` read what `encode` writes: a record takes its fields from the members of an object by name, extra members are ignored, an optional field may be left out, and an integer is checked against the width it is read into. <!-- test: core_json_the_tests_of_the_library_pass -->
* A decode `Err` names the place of the first mismatch as `$.members[1].x`. A field with a default is not filled from it, so it is missing like any other. An object decodes into a map with `String` keys and an array into a set, a repeated key or member keeping the last; a map with other keys, a union, a fixed-length array, a `Null`, a function and a handle are an `Err` to decode. <!-- test: core_json_the_tests_of_the_library_pass -->

Done when: `tools/test-core.sh json` passes.

## args

Command-line parsing of a type by reflection.

| Provides | The `Args` trait; flags from field names and the type's `flags` method; help text |
|---|---|
| Native | none |
| Slice | S146 |

* A struct or class that lists `Args` is read by `parse_args<T>(words)`, or by `from_env<T>()` from the program's own arguments; each field is a flag named after it, with `_` written `-` (`max_depth` is `--max-depth`). <!-- test: core_args_the_tests_of_the_library_pass -->
* A `Bool` is a switch (`--verbose`, or `--verbose=false`); an `Int`, `Float`, `String` or an enum of unit variants (read by its kebab name) takes a value as `--port 80`, `--port=80`, `-p 80`, `-p=80` or `-p80`; an optional may be left out; a list repeats the flag. <!-- test: core_args_the_tests_of_the_library_pass -->
* `fn flags() -> List<Flag>` on the type adds details by field name: `short`, `help`, `default_to` and `positional`; a positional field is read from the words that are not flags, in field order, and a positional list takes the rest. <!-- test: core_args_the_tests_of_the_library_pass -->
* Words after `--` are positional, and a negative number is a word and a value, not a flag. <!-- test: core_args_the_tests_of_the_library_pass -->
* A missing value, an unknown flag, a flag given twice, a word that no field takes and a value of the wrong kind or too wide for its integer type are an `Err` that names the flag. <!-- test: core_args_the_tests_of_the_library_pass -->
* `usage_text<T>(program)` writes the synopsis, the doc of the type, the positional arguments and the options in columns, with the doc of each field (or its `help`) and its default; `help_requested(words)` says whether `--help` or `-h` came before a `--`. <!-- test: core_args_the_tests_of_the_library_pass -->
* A program that is built reads its arguments and writes its usage. <!-- test: core_args_a_built_program_reads_its_arguments_and_writes_its_usage -->

Done when: `tools/test-core.sh args` passes.

## doc

One small in-memory representation of a document, like HTML in shape. `markdown` and `pane` are frontends of it; `toml` and `yaml` are when they fit.

| Representation | A tree of nodes in one arena: a kind, attributes, text, children; ids in place of pointers; strings interned once |
|---|---|
| Byte form | One buffer that crosses the FFI boundary in one call and reads back equal |
| Vocabulary | Five node kinds: document, element, text, comment, raw HTML. An element has a name and attributes; their names are free text |
| Provides | Build, query, walk, edit, write as HTML text |
| External library | None (pure Mote nodes and `MDOC` codec; `Doc` kept as a thin optional wrapper) |
| Slices | S147 (library), S148 (Mote API) |

* A document has the document node (id 0) and nodes made by `element`, `text`, `comment` and `raw`; a node is in the tree once it is appended to the document node or to an element, and appending moves it out of its old place. A node cannot go inside itself or its own descendants, a text or comment node cannot have children, and only an element has a name and attributes. <!-- test: core_doc_the_tests_of_the_library_pass -->
* `children`, `child`, `parent`, `next`, `previous`, `descendants`, `find`, `find_all` and `text_content` read the tree in document order; `attr`, `attrs` (in the order set), `set_attr`, `remove_attr`, `rename`, `set_text`, `insert_before`, `detach` and `copy` edit it. <!-- test: core_doc_the_tests_of_the_library_pass -->
* `to_html` writes a node, or the children of the document node, with text and attribute values escaped, no text escaping inside `script` and `style`, comments as `<!-- -->`, raw nodes as they are, void elements as `<br />`, and nothing added between nodes. <!-- test: core_doc_the_tests_of_the_library_pass -->
* `bytes` is the byte form (a header, the strings, then each node with its attributes and child ids) and `Doc.from_bytes` reads it back equal; bytes with a wrong header, a cut or extra tail, an id out of range, a node with two parents or a cycle are an `Err`. <!-- test: core_doc_the_tests_of_the_library_pass -->
* `adopt` copies a subtree from this or another document; text of any length crosses the FFI boundary whole. <!-- test: core_doc_the_tests_of_the_library_pass -->
* A program that is built links the archive and writes HTML. <!-- test: core_doc_a_built_program_links_the_archive_and_writes_html -->
* Size per node and the cost of a build are recorded in the benchmark results file (S112); this page cites a number only from there.
* `toml` and `yaml` use `doc` only if their conformance suites load into it with no loss and no slower than a parser of their own; the result is recorded in their sections.

Done when: `tools/test-core.sh doc` passes.

## markdown

Markdown as a frontend of `doc`.

| Provides | CommonMark text to a `doc` tree, a `doc` tree to Markdown text and to HTML |
|---|---|
| Native | Rust `libmarkdown` (`rust/`): the `pulldown-cmark` parser building the byte form of a `doc` tree; it contains its own copy of the `doc` model |
| Slice | S149 |

* `parse(text)` and `parse_with(text, extensions)` give a `doc` tree and `render` and `render_with` its HTML; text that is not UTF-8 is an `Err`. <!-- test: core_markdown_the_tests_of_the_library_pass -->
* Every one of the 652 examples of the CommonMark 0.31 specification renders to its HTML; a newline between two tags or at the ends is not compared, because the tree holds no node for it. <!-- test: core_markdown_the_tests_of_the_library_pass -->
* The tree is block elements (`p`, `h1`–`h6`, `blockquote`, `pre` with a `code` child and a `language-` class, `ul`, `ol` with `start`, `li`, `hr`) and inline ones (`em`, `strong`, `code`, `a` with `href` and `title`, `img` with `src`, `alt` and `title`, `br`), with text in text nodes and raw HTML, block or inline, in raw nodes; a link target is percent-encoded. <!-- test: core_markdown_the_tests_of_the_library_pass -->
* `github()` turns on tables (`table`, `thead`, `tbody`, `th` and `td` with a `style` for the alignment), strikethrough (`del`) and task lists (an `input` checkbox first in the item); `Extensions.smart_punctuation` curls quotes and dashes; `commonmark()` has none of them. <!-- test: core_markdown_the_tests_of_the_library_pass -->
* `to_markdown` writes the document node or an element as Markdown ending in a newline: characters that would read as syntax are escaped, lists keep numbering, nesting and looseness, a code fence is longer than any backtick run inside, a heading of several lines is a setext heading, and an element with no Markdown form (or emphasis at the edge of emphasis) is written as HTML. <!-- test: core_markdown_the_tests_of_the_library_pass -->
* Every specification example read, written with `to_markdown` and read again gives the same HTML. <!-- test: core_markdown_the_tests_of_the_library_pass -->
* A program that is built links both archives and renders text. <!-- test: core_markdown_a_built_program_links_both_archives_and_renders_text -->
Done when: `tools/test-core.sh markdown` passes.

## toml

TOML documents.

| Provides | Parse to a value tree, print, typed access |
|---|---|
| Native | Rust `libtoml` (`rust/`): the `toml` crate (TOML 1.1) and `serde_json`, trading a tagged JSON tree for the text |
| On `doc` | No. The fit test fails: `doc` has no integer, float or date-time, so a typed read would parse text again, and it finds a name by scanning the children, so a key lookup would be linear. The value tree is a Mote enum |
| Slice | S150 |

* `Toml` is `Table` (a list of key and value pairs, in the order of the text), `Array`, `Text`, `Integer`, `Real`, `Boolean` or `Datetime` (the text of a date, a time or both); `get(key)`, `at(i)`, `as_text`, `as_int`, `as_float`, `as_bool` and `as_datetime` read it. <!-- test: core_toml_the_tests_of_the_library_pass -->
* `parse(text)` is the table or an `Err` that names the line and column of the first mistake; it reads the whole of TOML 1.1, with integers in 64 bits, hexadecimal, octal and binary forms, `inf` and `nan`, and rejects a repeated key or table. <!-- test: core_toml_the_tests_of_the_library_pass -->
* `to_text()` writes a table as TOML text that `parse` reads back equal (keys in order, a key with a space quoted, a table of tables as `[a.b]` and a list of tables as `[[a]]`), and `pretty()` puts the items of an array on lines of their own; a value that is not a table is an `Err`. <!-- test: core_toml_the_tests_of_the_library_pass -->
* `encode(value)` and `to_toml()` give the table of a record as `json.encode` reads it, with a null member left out and a null anywhere else an `Err`; `decode<T>(value)` and `T.from_toml(value)` read a table as `json.decode` does, a date-time as text. <!-- test: core_toml_the_tests_of_the_library_pass -->
* Text of any length crosses the FFI boundary whole. <!-- test: core_toml_the_tests_of_the_library_pass -->
* A program that is built links the archive, reads and writes TOML. <!-- test: core_toml_a_built_program_links_the_archive_and_reads_toml -->

Done when: `tools/test-core.sh toml` passes.

## yaml

YAML documents.

| Provides | Parse to a value tree, print, typed access |
|---|---|
| Native | Rust `libyaml` (`rust/`): the `serde_yaml_ng` crate and `serde_json`, trading JSON text for the YAML text |
| On `doc` | No. The fit test fails as it does for `toml`; and the data model of a YAML configuration is the JSON one, so the value tree is `Json` of the `json` library, with its reflection |
| Slice | S151 |

* `parse(text)` is the one document of the text as a `Json` value, `Null` for a text of only blanks and comments, and an `Err` for more than one document or a mistake, which names the line and column; `parse_all(text)` is every document of a stream. <!-- test: core_yaml_the_tests_of_the_library_pass -->
* A plain scalar is typed by its form (`1`, `0x1F`, `-2.5`, `1e3`, `true`, `null`, `~`, an empty value), a quoted one is text, and block scalars (`|`, `>`, with their chomping marks) keep or fold their lines. <!-- test: core_yaml_the_tests_of_the_library_pass -->
* A comment is dropped, a tag is dropped, an anchor and its aliases are expanded, a merge key `<<` is applied, a mapping keeps the order of its keys and a scalar key is read as its text; a key that is not a scalar is an `Err`. <!-- test: core_yaml_the_tests_of_the_library_pass -->
* A float that is not finite is read as the text `.inf`, `-.inf` or `.nan`. <!-- test: core_yaml_the_tests_of_the_library_pass -->
* `to_text(value)` is the YAML text of one value and `to_text_all(values)` of several, each after a `---` line; text that would read as another type (`"true"`, `"123"`, `""`, `"null"`) is written quoted, so `parse` reads every value back equal. <!-- test: core_yaml_the_tests_of_the_library_pass -->
* `load<T>(text)` and `dump(value)` read and write a type as `json.decode` and `json.encode` do. <!-- test: core_yaml_the_tests_of_the_library_pass -->
* A program that is built links the archive, reads and writes YAML. <!-- test: core_yaml_a_built_program_links_the_archive_and_reads_yaml -->

Done when: `tools/test-core.sh yaml` passes.

## tile

A terminal UI as a function of state.

| Provides | A view tree, redraw of the cells that differ, keys, timers, mouse |
|---|---|
| Builders | `tile.vstack([...])`, `tile.button("+").on_click(f)` and chained modifiers make the view of a state; `App(state, view).run()` runs it |
| Native | none beyond `term` |
| Slices | S152, S164 (builders) |

* A `View` is text, a fill, a row or column of slots, a titled border or padding; a slot takes a fixed number of cells, what its content needs, or a weighted share of the rest, with the remainder going to the first shares. <!-- test: core_tile_the_tests_of_the_library_pass -->
* `render(view, rows, cols)` draws the view on a `Grid` of cells, cutting what does not fit and counting a wide glyph as two columns and a combining mark as none. <!-- test: core_tile_the_tests_of_the_library_pass -->
* `Grid.diff(before)` is the text that makes a terminal showing `before` show the grid: everything for the first drawing or a new size, else only the cells that differ, each run after one cursor move. <!-- test: core_tile_the_tests_of_the_library_pass -->
* `run(terminal, state, view, update, options)` draws `view(state)`, reads a key, a mouse event, a tick or a new size, and gives the state and the input to `update`, until it quits; the terminal is raw on the alternate screen with the cursor hidden, and is put back when the program ends or fails. <!-- test: core_tile_the_tests_of_the_library_pass -->
* `Options` give the milliseconds between ticks (0 for none), whether the mouse is reported, and the size to use when the terminal says none. <!-- test: core_tile_the_tests_of_the_library_pass -->
* An `Element` is made by `text`, `button` (the label reversed between spaces), `vstack`, `hstack`, `spacer` and `fill`, and every modifier (`pad`, `gap`, `width`, `height`, `grow`, `grow_by`, `border(title)`, `fg`, `bg`, `bold`, `dim`, `italic`, `underline`, `reverse`, `id`) answers a changed copy, so they chain; a style is passed down to the text under an element, and the text can set its own. <!-- test: core_tile_the_builder_tests_pass -->
* `lay_out(element, rows, cols)` draws an element tree on a grid and gives the area of each named element; `width` is along a row, `height` along a column, and `grow` takes shares of what the siblings leave. <!-- test: core_tile_the_builder_tests_pass -->
* `on_click(f)` takes a function over `var` state or a lambda that names the state type; a left press runs the handler of the element under the cell or the nearest one around it, and the view is built and drawn again. <!-- test: core_tile_the_builder_tests_pass -->
* `App(state, view)` carries `on_key("q", f)`, `quit_on("Escape")` (keys are written as `KeyEvent` shows them), `every(ms, f)`, `quit_when(done)` and `mouse(on)`; `run()` takes the terminal of the process and `run_on(terminal)` a given one, so a pseudo-terminal tests an app. <!-- test: core_tile_the_builder_tests_pass -->

Done when: `tools/test-core.sh tile` passes.

## pane

A window as a function of state, over `doc`.

| Layout | clay computes a draw list from a `doc` view tree |
|---|---|
| Drawing | raylib draws the list on a thread of its own; `Window.open(width, height, title)` opens the window and `window.present(layout)` hands it the list |
| Events | Input events reach the program through a port and `window.run` passes each to the step that updates the state |
| Pipelines | An event can run a declared list of steps on the state: set a field, show a screen, send to a channel, then continue when the answer arrives |
| Builders | `pane.vstack([...])`, `pane.button("+").on_click(f)` and chained modifiers make the view of a state; `App(title, state, view).run()` shows it and runs the handlers |
| Native | yes (C, built by `zig cc`: vendored clay, and raylib for the window) |
| Slices | S154 (layout), S155 (window and events), S156 (pipelines), S163 (builders) |

* The draw list for a view and a headless window are tested without a display. Seeing the window needs one.
* The state lives in Mote, as one struct or class value that `window.run` and `window.run_routes` carry; the C backend holds none. <!-- test: core_pane_the_window_tests_pass -->
* `layout(view, width, height)` lays out a view, the top element of a `doc` tree (or the elements at the top of the document node, as a column), in a window of that size, and gives the draw list and the area of each element with an `id`. <!-- test: core_pane_the_layout_tests_pass -->
* A view is made of `row`, `column`, `box` (a column), `spacer` and `text`, with the attributes `width` and `height` (`grow`, `fit`, pixels or a percent of the parent), `padding` (one, two or four numbers), `gap`, `align-x`, `align-y`, `background`, `radius`, `border` (a width and a colour), `clip` and `id`; the root grows to the window, and a `spacer` grows. <!-- test: core_pane_the_layout_tests_pass -->
* A text is `size` pixels high (16 unless set) and measured with the embedded DejaVu face's glyph advances (`font` is `sans`, `bold`, `serif`, or `mono`); `color`, `size` and `font` on an element are inherited by the text under it, `wrap` is `words` or `none`, and a bare text node is a text. <!-- test: core_pane_the_layout_tests_pass -->
* The draw list is `Rect`, `Text`, `Border`, `Clip` and `Unclip` commands in paint order with a position, size, colour `0xRRGGBBAA`, radius, border width, text and text size; `layout.at(x, y)` is the id of the element painted on top at a point. <!-- test: core_pane_the_layout_tests_pass -->
* An unknown element, a bad attribute value or an id used twice is an `Err` that names the element and the attribute. <!-- test: core_pane_the_layout_tests_pass -->
* A program that is built links the archive and lays out a view. <!-- test: core_pane_a_built_program_links_the_archive_and_lays_out_a_view -->
* `Window.headless(width, height)` has no screen and keeps every layout shown in `window.frames`; `window.present(layout)` shows one. <!-- test: core_pane_the_window_tests_pass -->
* A window gives the program `Event` values through a port: `Click(target, x, y, button)`, `Key(code)`, `Text(code point)`, `Wheel(x, y, dx, dy)` in hundredths of a notch, `Resized(w, h)` and `Close`; `window.inject(event)` sends one as if the user had, and every event survives the port as it was sent. <!-- test: core_pane_the_window_tests_pass -->
* `window.run(state, view, update)` builds the view of the state with a fresh `Doc`, lays it out in the window's size, shows it, and passes each event to `update`, which answers `Next.Continue(state)` or `Next.Quit(state)`; it returns the last state, and a headless window ends the run when it has no more events. <!-- test: core_pane_the_window_tests_pass -->
* A `Click` carries the `id` of the element painted on top at the point, or null; a `Resized` event changes the size the next view is laid out in. <!-- test: core_pane_the_window_tests_pass -->
* A view that is wrong ends the run with the layout's `Err`. <!-- test: core_pane_the_window_tests_pass -->
* A built program links the window library through the init of the linked archive and runs a headless window. <!-- test: core_pane_a_built_program_runs_a_headless_window_through_the_linked_init -->
* A pipeline is a list of `Step` values run by `run_steps(state, steps)`: `Set(field, text)`, `Show(screen)` (sets the field `screen`), `Send(sender, text)`, `Await(receiver, field)` (waits for one reply and sets the field from it) and `Quit`; it gives `Next.Continue(state)`, or `Next.Quit(state)` at a `Quit`. <!-- test: core_pane_the_pipeline_tests_pass -->
* `Set` and `Await` set a field of a struct or class state from text, for a `Bool`, `Int`, `Float` or `String` and for the name of a unit variant of an enum; the state given is left as it was, and a field that is missing, a text of the wrong form or a channel that closes with no reply is an `Err` that names the field. <!-- test: core_pane_the_pipeline_tests_pass -->
* `Routes` binds a pipeline to a click on an element (`on_click`), a key (`on_key`) or the close of the window (`on_close`); `window.run_routes(state, view, routes)` runs the pipeline of each event, an event with none changes nothing, and a `Close` with none quits. <!-- test: core_pane_the_pipeline_tests_pass -->
* A step that fails ends `run_routes` with its `Err`, and a pipeline can ask a task through a channel and show the answer in the next frame. <!-- test: core_pane_the_pipeline_tests_pass -->
* An `Element` is made by `text`, `h1` to `h4`, `button`, `vstack`, `hstack`, `box` and `spacer`, and every modifier (`pad`, `pad_xy`, `margin`, `gap`, `width`, `height`, `grow`, `bg`, `color`, `radius`, `border`, `size`, `font`, `center`, `middle`, `id`) answers a changed copy, so they chain; a colour is `0xRRGGBBAA`. <!-- test: core_pane_the_builder_tests_pass -->
* `lay_out(element, width, height)` lays an element tree out without a window; `grow` takes the room along the parent's axis (both at the top), and `margin` is space outside that a click in it misses. <!-- test: core_pane_the_builder_tests_pass -->
* `on_click(f)` takes a function over `var` state or a lambda that names the state type, `|var s: Counter| { s.count += 1 }`; a click runs the handler of the element under the point or the nearest one around it, and the view is built and shown again. <!-- test: core_pane_the_builder_tests_pass -->
* `App(title, state, view)` carries `size(w, h)`, `on_key(code, f)`, `quit_on(code)` and `quit_when(done)`; `run()` opens a window and `run_in(window)` runs in one that is open, so a headless window tests an app. A close ends the run with the last state. <!-- test: core_pane_the_builder_tests_pass -->

Done when: `tools/test-core.sh pane` passes.
