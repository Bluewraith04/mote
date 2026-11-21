# The `mote` command

## Executables

* `mote compile` writes the relocatable object file `target/<name>.o` of the package's `main` and its start-up. <!-- test: cli_compile_writes_the_object_file_below_target -->
* `mote build` links that object with the runtime into the executable `target/<name>`, which prints what `mote run` prints and ends with the same code. <!-- test: cli_build_writes_an_executable_that_runs_main, aot_every_corpus_group_shows_the_recorded_results -->
* `mote build` and `mote install` copy each shared external library the program opens (one with no archive to link) to `lib/` beside the executable, and the executable opens that copy before the path it was built with. <!-- test: core_tools_a_built_program_carries_the_shared_library_it_opens_beside_it, core_tools_a_built_program_with_a_linked_archive_needs_no_file_beside_it, beside_a_library_next_to_the_executable_replaces_the_built_in_path -->
* The same source gives the same object file and the same executable from any directory. <!-- test: aot_reproducible_the_same_source_gives_the_same_object_from_two_directories, aot_reproducible_the_same_source_links_the_same_executable_from_two_directories -->
* A program that does not build writes nothing and exits 1. <!-- test: cli_build_of_a_package_with_errors_exits_1_and_writes_nothing -->
* The link runs `$CC`, else `cc`, and asks it for `mold` or `lld` when one is on the `PATH`; the executable is a dynamic glibc executable, and the runtime is embedded in the `mote` binary. <!-- test: executable_the_link_uses_cc_from_the_environment_else_cc, executable_the_link_asks_for_mold_or_lld_when_one_is_on_the_path -->
* An executable takes `--seed N`, which runs it on one thread in the order the seed gives, and `--feed PORT MS BYTES` and `--close PORT MS`, which push to a port the program opens (the bytes are decimal numbers split by commas). <!-- test: args_the_command_line_takes_a_seed, args_the_command_line_takes_the_pushes_to_ports, aot_seeds_a_racing_program_shows_both_orders_and_a_seed_repeats -->
* The words after `--` are the program's own arguments, for an executable and for `mote run [file] -- words`. <!-- test: args_the_command_line_leaves_what_follows_two_dashes_to_the_program, std_rt_io_args_are_the_words_after_the_double_dash, std_rt_io_args_reach_a_program_started_by_run -->
* A fault prints `fault: <message>` and an error prints `error: <text>` to standard error, with exit code 1. <!-- test: aot_a_fault_and_an_error_end_the_executable_with_code_1 -->
* A `main` that returns an `Int` ends the program with that code. <!-- test: run_tests_main_that_returns_an_int_ends_with_that_code, std_rt_io_main_that_returns_an_int_exits_with_it -->
* A fault adds the place of the check as `  at <file>:<line>:<column>` and a line `  in <function> (<file>:<line>)` for each caller, innermost first; in an optimized build a function inlined into another is named by the other. <!-- test: aot_backtrace_a_fault_names_the_check_and_each_function_with_its_line, aot_backtrace_an_inlined_function_is_named_by_the_function_it_was_inlined_into -->
* The object and the executable carry DWARF line tables, so `gdb`, `perf` and `addr2line` map an address to a line of the source. <!-- test: object_the_line_table_has_a_sequence_for_each_function_placed_by_a_relocation_with_the_rows_of_the_debug_table -->
* `mote run` with `MOTE_PERF_MAP=1` writes `/tmp/perf-<pid>.map` for `perf`. <!-- test: jit_the_perf_map_lists_each_function_with_its_start_size_and_name -->

## Standard input and output

* `eprint` and `eprintln` write to standard error as `print` and `println` write to standard output; the result of `mote run` does not hold standard error. <!-- test: std_rt_io_eprint_writes_to_standard_error -->
* `read_line() -> String?` of `std.io` reads one line of standard input without its line end and is `null` at the end of the input; it blocks its worker until a line arrives. <!-- test: std_rt_io_read_line_reads_lines_then_null_at_the_end, std_rt_io_read_line_in_run_sees_the_end_of_input -->
* `args() -> [String]` of `std.io` is the program's own arguments. <!-- test: std_rt_io_args_are_the_words_after_the_double_dash -->
* `exit(code)` of `std.io` ends the program at once with that code. <!-- test: std_rt_io_exit_ends_the_program_with_its_code -->

## Packages

* `mote package` writes `target/<name>-<version>.zip` of the package and says how many files and bytes it holds. <!-- test: cli_package_writes_the_zip_below_target_and_says_what_it_holds, cli_package_of_a_directory_without_a_manifest_is_an_error -->
* `mote add <zip>` copies the zip into `deps/` of the project, unless it is already inside the project, and writes `NAME = { zip = "deps/..." }` under `[dependencies]`; an entry of the same name is replaced. <!-- test: cli_add_copies_a_zip_into_deps_writes_the_entry_and_the_app_runs_it, cli_add_a_zip_already_in_the_project_is_not_copied, cli_add_replaces_the_entry_of_the_same_name, cli_add_keeps_the_rest_of_the_manifest_and_adds_after_the_last_entry -->
* `mote add <dir>` writes `NAME = { path = "..." }` with the path from the project. <!-- test: cli_add_a_directory_writes_a_path_entry_relative_to_the_project -->
* `mote add` names each dependency of the added package that the project does not list. <!-- test: cli_add_names_the_dependencies_of_the_package_that_the_project_lacks -->
* `mote add` refuses a file that is not a zip, a directory that is not a package, the project itself and a reserved name. <!-- test: cli_add_refuses_what_is_not_a_package -->
* `mote remove <name>` deletes the entry and leaves the zip. <!-- test: cli_remove_deletes_the_entry_and_leaves_the_zip -->
* `mote install` links the main of the package around the current directory, of a directory, or of a zip into `<home>/bin/<name>`; the home is `$MOTE_HOME`, else `~/.mote`. <!-- test: cli_install_links_a_package_directory_into_the_bin_of_the_home, cli_install_of_a_zip_builds_it_from_the_cache, cli_install_of_a_library_is_an_error_and_writes_nothing -->

## Libraries

* `--std <dir>` and `--core <dir>` of `build`, `check`, `compile`, `install`, `run` and `test` use the std or the core libraries in a directory instead of the embedded ones. <!-- test: core_tools_an_app_imports_a_core_library_by_name_and_runs, cli_help_for_a_command_shows_its_arguments_and_flags -->

## Usage

* `mote help` prints the commands; `mote help <command>` prints the flags of one. <!-- test: cli_help_lists_every_command_of_the_table_with_its_help_line, cli_help_for_a_command_shows_its_arguments_and_flags, cli_help_for_a_command_without_flags_has_no_flags_section, cli_help_for_an_unknown_command_is_a_usage_error -->

<!-- table: usage -->
```text
Usage: mote <command> [arguments]

Commands:
  add <package>      Add a dependency: a zip file or a package directory
  build [dir]        Link the main of a package into target/<name>
  check [dir]        Check a package without running it
  compile [dir]      Compile the main of a package to the object file target/<name>.o
  help [command]     Show the commands, or the usage of one
  init [dir]         Make a directory a package: a new one, or `.`
  install [package]  Link the main of a package or zip file into <home>/bin/<name>
  package [dir]      Pack a package into target/<name>-<version>.zip
  remove <name>      Remove a dependency
  run [file]         Run the main of a package, or of one of its files
  test [dir]         Run the test blocks of a package
  version            Print the version

Run `mote help <command>` for the flags of a command.
```
<!-- /table -->
