# Modules

## Files

* Every `.mote` file under `src/` is a module. Its path without the extension names it: `util.mote` of the package `app` is `app.util`, and `sub/worker.mote` is `app.sub.worker`. <!-- test: graph_names_modules_from_their_paths -->
* `lib.mote` is the root module of its package; `.lib` also reaches it from a sibling. <!-- test: graph_names_modules_from_their_paths, graph_lib_is_also_reachable_as_a_sibling -->
* A file or a directory under `src/` whose name is not a module name is an error. <!-- test: graph_load_a_file_or_directory_that_is_not_a_module_name_is_an_error -->
* A file or folder whose name starts with `_` is private to its package: `import pane._native` from another package is an error at the import, and the package's own files import it as any other. A name from such a file reaches a dependent only through a `pub import`. <!-- test: graph_a_file_or_folder_that_starts_with_an_underscore_is_private_to_its_package -->
* A folder's module is the file of its name beside it: `sub.mote` is `app.sub` and `sub/worker.mote` is `app.sub.worker`; there is no `mod.mote`. <!-- test: graph_a_folder_module_is_the_file_of_its_name_beside_it -->
* A source file that is not UTF-8 is an error that names the file. <!-- test: graph_load_a_source_file_that_is_not_utf8_is_an_error_naming_the_file -->

## Importing

<!-- test: graph_resolves_a_dot_import_to_the_sibling_file, graph_resolves_dot_dot_and_super_to_the_directory_above, graph_resolves_std_imports_in_every_package, graph_resolves_a_dependency_name_to_its_lib_and_its_files, a_module_import_binds_the_last_segment_or_the_alias, a_named_import_takes_one_name_under_an_alias, a_glob_brings_every_pub_name -->
| Written | Brings in |
|---|---|
| `import .helper` | the file `helper.mote` in the importing file's directory, used as `helper.f()` |
| `import .sub.worker` | `sub/worker.mote` below that directory |
| `import ..util` `import super.util` | `util.mote` one directory up |
| `import std.iter as it` | a standard module under the name `it` |
| `import pane` | a dependency package: what its `src/lib.mote` has |
| `import pane.nav` | another file of that package that does not start with `_`, used as `nav.f()` |
| `import .utils as u` | a local module under the name `u` |
| `import { compute } from .helper` | one name, used bare |
| `import { helper as ha } from .a` | one name under another name |
| `import { * } from .helper` | every `pub` name of the module, used bare |

* An imported standard module is used by its alias, and an imported name bare. <!-- block -->

```mote
import std.iter as it
import { parse_int } from std.string

let doubled = it.map([1, 2, 3], |n| n * 2)
println(doubled)
println(parse_int("42")!)
```

```output
[2, 4, 6]
42
```

* After `import .m`, call `m.f()`; the names of `m` are not brought in bare. <!-- test: a_module_gives_its_functions_and_globals_by_qualified_name, a_named_import_takes_one_name_under_an_alias -->
* A module name is not a value, and a function of a module is: `let a = m` is an error and `let b = m.f` is not. <!-- test: a_module_name_is_not_a_value_and_its_function_is -->
* A module name is not a type; `m.T` is. <!-- test: unknown_modules_traits_and_values_are_not_types, a_type_is_reached_through_a_module_name -->
* An import of a private or a missing name is an error at the import. <!-- test: importing_a_private_or_missing_name_is_an_error -->
* Two `{ * }` imports of one name are an error unless they reach one item. <!-- test: two_globs_of_one_name_are_an_error_unless_it_is_the_same_item -->
* A name the module defines, and a name it imports by name, win over a `{ * }` one. <!-- test: a_modules_own_and_explicit_names_win_over_a_glob -->
* A circular import is an error that names the modules in the cycle, and it is skipped. <!-- test: a_circular_import_names_its_modules_and_is_skipped, a_module_importing_itself_is_a_cycle, graph_a_cycle_of_imports_is_an_error_at_the_closing_import -->

## Resolution

* Items may be used before their declaration, in any order. <!-- block -->

```mote
println(double(4))

fn double(n: Int) -> Int {
    return n * 2
}
```

```output
8
```

* A name defined twice in a module, and a name both defined and imported by name, are errors. <!-- test: a_name_defined_twice_is_an_error, an_import_clashing_with_an_own_name_is_an_error -->
* A name is private to its module or `pub`; a module has no other tier. <!-- test: importing_a_private_or_missing_name_is_an_error -->
* An import of a module that does not exist is an error at the import: a missing file, a missing standard module, a path above the package, a package that is not a dependency, a dependency with no `src/lib.mote`. <!-- test: graph_a_missing_module_is_an_error_at_the_import, graph_a_missing_package_or_lib_is_an_error_at_the_import -->

## Standard library

<!-- table: std-modules -->
* The modules are `std.atomic` `std.bytes` `std.collections` `std.error` `std.float` `std.io` `std.iter` `std.math` `std.meta` `std.numbers` `std.numeric` `std.option` `std.port` `std.signed` `std.stream` `std.string` `std.sync` `std.task` `std.test` `std.text` `std.time` `std.traits` `std.types` `std.unsigned`.
<!-- /table -->

* Every standard module is imported by its path under `std`, in every package. <!-- test: graph_resolves_std_imports_in_every_package, graph_names_the_modules_of_the_real_std_as_the_semantics_harness_does -->
* The standard source is embedded in the toolchain; `--std DIR` replaces it with a directory that holds a `mote.toml` for `std`. <!-- test: std_embed_carries_the_manifest_and_every_file_of_the_std_directory, std_embed_a_directory_replaces_the_embedded_std, std_embed_a_directory_that_is_not_the_std_is_an_error, std_embed_a_directory_with_no_manifest_is_an_error, cli_check_uses_the_std_flag_in_place_of_the_embedded_std -->

## Prelude

* In scope with no import: the number types, `Bool`, `Float`, `String`, `Option` with `Some` and `None`, `Result` with `Ok` and `Err`, `Error`, `ErrorKind`, `List`, `Map`, `print`, `println`, `eprint`, `eprintln` and `typeof`. <!-- block --> <!-- test: the_prelude_is_visible_without_an_import_and_not_in_the_scope -->

```mote
fn pick(a: Int64, b: UInt8, c: Float32) -> Option<String> {
    return Some("${a} ${b} ${c}")
}

let ok: Result<Int, Error> = Ok(1)
let xs: List<Int> = [1]
let m: Map<String, Int> = {"a": 1}
println(pick(1, 2, 3.5))
println(ok)
println(typeof(xs))
println(m)
```

```output
1 2 3.5
Ok(1)
List<Int>
{"a": 1}
```

* Every other standard name is imported by name. <!-- block -->

```mote
let seen: Set<Int> = Set() // error: unknown name `Set`
```

<!-- block -->
| Name | Module |
|---|---|
| `Set` `Stream` | `std.collections` |
| `Task` | `std.task` |
| `Channel` `Sender` `Receiver` `Mutex` `Once` | `std.sync` |
| `Port` `PortMode` `PortEnd` | `std.port` |
| `Display` `Debug` `Eq` `Ord` `Clone` `Closeable` `Key` | `std.traits` |
| `Pointer` | `std.types` |

```mote
import { Set, Stream } from std.collections
import { Task } from std.task
import { Channel, Sender, Receiver, Mutex, Once } from std.sync
import { Port, PortMode, PortEnd } from std.port
import { Display, Debug, Eq, Ord, Clone, Closeable, Key } from std.traits
import { Pointer } from std.types
```

## Exporting

* Nothing is visible outside its module unless marked `pub`. <!-- test: importing_a_private_or_missing_name_is_an_error, a_glob_brings_every_pub_name -->

<!-- test: pub_import_re_exports_and_a_plain_import_does_not, pub_glob_and_pub_module_import_re_export, a_glob_brings_every_pub_name, a_module_gives_its_functions_and_globals_by_qualified_name -->
| Written | Exports |
|---|---|
| `pub fn f() { … }` | a function |
| `pub let MAX = 100` | a global, readable elsewhere |
| `pub struct` `pub class` `pub enum` `pub trait` `pub type` | a type |
| `pub import { real } from .origin` | a name from another module: a facade |
| `pub import { * } from .origin` | every `pub` name of another module |
| `pub import .origin as inner` | a module under a name |

```mote
pub let MAX = 100
pub fn double(n: Int) -> Int { return n * 2 }
pub struct Point {
    pub x: Int
}
pub class Counter {
    pub var n: Int = 0
}
pub enum Light {
    Red
    Green
}
pub trait Named {
    fn name(self) -> String
}
pub type Id = Int

@link("c")
pub native fn getpid() -> Int32
```

* A plain `import` does not re-export: another module importing that name from the facade gets an error. <!-- test: pub_import_re_exports_and_a_plain_import_does_not -->
* `pub` goes before an item; `pub` before a statement is an error. <!-- block -->

```mote
pub println(1) // error: expected an item
```

* A `pub` function, a `pub` method, a `pub` field, a variant of a `pub` enum, a `pub type` and a `pub let` cannot name a type that is not `pub`: an importer could not write it. <!-- block -->

```mote
struct Secret {}

pub fn reveal(s: Secret) {} // error: private type `Secret` in a public declaration
```

## Member visibility

* Each member of a `pub` type has its own. <!-- test: another_module_reads_pub_fields_and_calls_pub_methods_only -->

<!-- test: another_module_reads_pub_fields_and_calls_pub_methods_only, another_module_writes_a_field_or_a_global_only_through_a_method_of_its_owner, a_bound_needs_methods_the_caller_sees_and_a_pub_trait_is_imported_like_a_type -->
| Member | Another module may |
|---|---|
| a `pub` field | read it |
| a private field | not read it |
| any field | never assign it; writes go through a method of the type, so `p.inner.bump()` is an error too |
| a `pub` method | call it |
| a private method | not call it |
| a method a listed `pub` trait names | call it |
| the literal `Point { … }` of a type with a private field | not write it; use a `pub` static method |

## Globals

* A top-level `let` or `var` is a module global. Another module reads a `pub` one as `m.NAME`; only the module's own top-level code assigns a global. <!-- test: a_module_gives_its_functions_and_globals_by_qualified_name, another_module_writes_a_field_or_a_global_only_through_a_method_of_its_owner, top_level_code_assigns_its_own_globals_and_destructures -->

* A function or a lambda does not assign a `var` global. <!-- block -->

```mote
var ready = false
ready = true

fn reset() {
    ready = false // error: only the module's own top-level code assigns a global
}
```

* A function or a lambda does not write through a `var` global either: `items.push(1)`, `items[0] = 2`, `cell.n = 3` and passing it to a `var` parameter are errors. Top-level code can. <!-- test: functions_and_lambdas_do_not_write_into_a_var_global -->

```mote
var items: List<Int> = [1]
items.push(2)

fn add() {
    items.push(3) // error: no `var` path
}
```

* Globals start in source order, except that a global which reads another one starts after it; a cycle of them is an error. <!-- test: globals_run_in_source_order_unless_one_reads_another, a_read_through_a_function_orders_the_global, a_global_that_reads_itself_through_functions_is_a_cycle, globals_that_depend_on_each_other_are_an_error_and_functions_do_not_assign_them -->
* A task that starts at start-up reads no `var` global. <!-- test: a_task_started_at_start_up_reads_no_var_global -->

## Native functions

* `native fn name(params) -> Ret` declares a function of a C library and has no body. <!-- block -->
* `@link("lib")` names the library, and the symbol is the function name; `@link("lib", symbol = "c_name")` names another symbol. <!-- block -->

```mote
@link("m")
native fn pow(x: Float, y: Float) -> Float

@link("sqlite3", symbol = "sqlite3_libversion_number")
native fn sqlite_version() -> Int32
```

* A native fn needs `@link`; `@link` and `@blocking` go on a native fn only. <!-- test: native_functions_record_their_link, link_and_blocking_belong_on_native_functions -->
* In `std` only, `@intrinsic("name")` takes the place of `@link`: the function may take type parameters and any parameter types, and is not loaded from a library. <!-- test: an_intrinsic_is_a_generic_native_function_in_std -->
* `@blocking` marks a native whose call may wait; the mark is kept in the link table. <!-- test: native_functions_record_their_link, a_signature_collects_params_blocking_and_result -->
* A native that nothing calls is not linked and is not an error. <!-- test: native_a_native_that_is_never_called_is_not_linked_and_not_an_error, only_natives_the_program_calls_are_linked -->
* A native a global or top-level code calls is linked too. <!-- test: a_native_a_global_or_top_level_code_calls_is_linked -->

### Libraries

* The library name is logical: letters, digits and `_-.+`. A name with a path in it is an error at the `native fn`. <!-- test: native_a_library_name_with_a_path_is_an_error_at_the_native_fn -->
* `native/<arch>-linux/lib<name>.so` is searched in the package that declares the function, then in the other packages from the root back to `std`, then by the system loader. `<arch>` is `x86_64`. <!-- test: native_triple_names_the_architecture_of_this_machine, native_finds_the_library_in_the_native_directory_of_the_package_that_declares_it, native_finds_the_library_of_a_dependency_in_the_dependency, native_a_library_the_declaring_package_lacks_is_found_in_another_package, native_a_library_no_package_has_is_left_to_the_system -->
* An executable (`mote build`) links `lib<name>.a`, when it lies beside the `lib<name>.so` that was found, and fills the native addresses from the linked symbols; the archive must be position-independent. A library with no archive, and the system's, are opened when the executable starts. <!-- test: native_an_archive_beside_the_shared_library_is_the_libraries_archive, aot_static_a_program_whose_natives_ship_an_archive_runs_without_the_shared_library, aot_static_a_library_with_no_archive_is_opened_when_the_executable_starts -->
* The optional `void mote_init_<name>(const MoteHost *)` of a linked archive is called before `main`, with every character of `<name>` but letters and digits as `_`. <!-- test: object_tables_a_library_linked_in_has_its_symbols_in_the_slots_and_a_weak_init, aot_static_a_linked_library_gets_its_init_before_main -->
* A library that two packages find in different files is an error at the second declaration. <!-- test: native_one_library_found_in_two_files_is_an_error_at_the_second_declaration -->
* A program with no natives links nothing. <!-- test: native_a_program_with_no_natives_links_nothing -->

### Types

<!-- test: numbers_bool_and_unit_map_to_their_c_types, strings_go_in_by_copy_and_come_out_by_copy, arrays_of_scalars_are_parameters_only, a_pointer_is_a_pointer_and_an_optional_pointer_may_be_null, a_port_end_crosses_as_a_parameter_only, a_var_parameter_is_passed_by_pointer, a_signature_collects_params_blocking_and_result -->
| Mote type | C type |
|---|---|
| `Int8` `Int16` `Int32` `Int` | `int8_t` `int16_t` `int32_t` `int64_t` |
| `UInt8` `UInt16` `UInt32` `UInt64` | `uint8_t` `uint16_t` `uint32_t` `uint64_t` |
| `Float32`, `Float` | `float`, `double` |
| `Bool` | `uint8_t` |
| `Null` result | `void` |
| `String` parameter | a C string copied for the call |
| `String` result | a C string copied out |
| `[UInt8]`, an array of scalars, as a parameter | two arguments: a pointer to the elements, then the length |
| `Pointer<T>` | a pointer |
| `Pointer<T>?` | a pointer; `None` is NULL |
| a `var` number, `Bool` or pointer parameter | a pointer to the value |
| `PortEnd` as a parameter | a port handle |

* A `Null` parameter, a `[T]` result, a `PortEnd` result and a `var` string or array parameter are errors; so is any other type: a union, an optional of anything but a pointer, a tuple, a fixed array, a class. <!-- test: numbers_bool_and_unit_map_to_their_c_types, arrays_of_scalars_are_parameters_only, other_types_do_not_cross, a_class_that_is_not_the_pointer_lang_item_does_not_cross, a_var_string_or_array_has_no_pointer_form, native_a_signature_that_does_not_cross_to_c_is_an_error_naming_the_parameter_and_the_type -->
* A native fn takes no type parameters, is not a method and has no variadic parameter. <!-- test: a_signature_names_the_first_thing_wrong -->
* A called native with a bad signature is an error at the declaration. <!-- test: a_called_native_with_a_bad_signature_is_reported -->

## Ports

* A `PortEnd` is a handle that a struct may hold and a native may take as a parameter. <!-- test: a_port_end_is_a_sendable_handle_a_struct_may_hold_and_a_native_may_take, a_port_end_crosses_as_a_parameter_only -->
* `Port.open(mode)` returns a `PortEnd` to give to a library and a `Receiver` of byte arrays. `PortMode.Queue(n)` holds up to `n` messages and `PortMode.Latest` keeps the newest. <!-- block -->

```mote
import { Port, PortMode, PortEnd } from std.port

@link("sensors")
native fn start(events: PortEnd)

fn watch() {
    let (end, events) = Port.open(PortMode.Queue(64))
    start(end)
    for event in events {
        println(event.len())
    }
}
```

## Pointers

* `Pointer<T>` is a struct of std with one `pub` field, `address: UInt64`. `T` names what it points at, so `Pointer<Db>` and `Pointer<Stmt>` are different types. <!-- block -->

```mote
import { Pointer } from std.types

struct Db {}
struct Stmt {}

fn close(db: Pointer<Db>) {}

fn address_of(db: Pointer<Db>) -> UInt64 {
    return db.address
}

fn wrong(stmt: Pointer<Stmt>) {
    close(stmt) // error: expected Pointer<Db>, found Pointer<Stmt>
}
```

* A native takes `Pointer<T>?` where the library accepts NULL, and a `var` pointer parameter where it writes one. <!-- block -->

```mote
import { Pointer } from std.types

struct Db {}

@link("sqlite3")
native fn sqlite3_open(path: String, var db: Pointer<Db>?) -> Int32

@link("sqlite3")
native fn sqlite3_close(db: Pointer<Db>) -> Int32

fn open(path: String) -> Pointer<Db>? {
    var db: Pointer<Db>? = null
    if sqlite3_open(path, db) != 0 {
        return null
    }
    return db
}
```

## Packages

* A package is a directory with a `mote.toml` and a `src/`, and optionally a `tests/` and a `native/`. <!-- test: graph_load_reads_the_manifest_and_every_mote_file_under_src_in_path_order, graph_load_a_package_with_no_src_has_no_files, graph_load_a_directory_without_a_manifest_is_an_error_naming_the_file -->
* `std`, `prelude` and the names of the core libraries are not package names, and a package name is unique in a build. <!-- test: graph_the_names_std_and_prelude_are_reserved, graph_two_packages_with_one_name_are_an_error_at_the_second -->
* `src/lib.mote` is the root a dependent imports. Every other file under `src/` is the module of its path. <!-- test: graph_resolves_a_dependency_name_to_its_lib_and_its_files, graph_records_each_package_range_lib_and_entry -->

<!-- test: manifest_reads_every_field, manifest_with_only_a_name_and_a_version_takes_the_defaults, manifest_names_versions_and_paths_are_checked, manifest_needing_a_newer_mote_is_an_error_at_the_key, graph_load_follows_path_dependencies_and_loads_each_package_once, graph_load_a_dependency_with_another_name_is_an_error_at_its_line, manifest_a_zip_dependency_is_a_zip_file_and_not_also_a_path, zip_a_dependency_is_loaded_from_the_cache -->
| Manifest key | Meaning |
|---|---|
| `package.name`, `package.version` | required; the name is letters, digits and `_`, the version is `major.minor.patch` |
| `package.mote` | the oldest toolchain version that builds the package |
| `package.entry` | the `.mote` file `mote run` starts in; `src/main.mote` when unset |
| `package.description`, `package.license`, `package.authors` | optional text |
| `dependencies.NAME = { path = "../dir" }` | a package by path; the name must be the dependency's own |
| `dependencies.NAME = { zip = "deps/name-1.0.0.zip" }` | a package in a zip file, relative to the manifest; the name must be the dependency's own, and an entry has `path` or `zip`, not both |

* An unknown key or table in `mote.toml` is an error at its line, and a missing part or a value of the wrong type is an error that names the file, the line and the key. <!-- test: manifest_unknown_keys_and_tables_are_errors, manifest_missing_parts_are_errors, manifest_values_of_the_wrong_type_are_errors, manifest_error_names_the_file_the_line_and_the_key, manifest_syntax_error_names_the_file_and_the_line -->
* `mote.toml` is read as a subset of TOML: tables, strings, integers, booleans, arrays and inline tables; the rest is an error. <!-- test: toml_reads_tables_strings_integers_and_booleans_with_the_line_of_each_key, toml_reads_escapes_comments_and_quoted_keys, toml_reads_arrays_across_lines_and_inline_tables, toml_the_parts_of_toml_outside_the_subset_are_errors, toml_a_key_or_table_set_twice_is_an_error_at_the_second -->
* A dependency directory that is missing, or that forms a cycle, is an error at the dependency's line. <!-- test: graph_load_a_dependency_directory_that_is_missing_is_an_error_at_its_line, graph_load_a_dependency_cycle_on_disk_is_an_error, graph_a_dependency_cycle_is_an_error_at_the_dependency_that_closes_it, graph_a_dependency_that_is_not_given_is_an_error_at_its_line -->
* Packages are ordered with `std` first, each dependency before its dependents and the root last; a package the root does not reach is left out. <!-- test: graph_packages_are_ordered_std_first_each_dependency_before_its_dependents_and_the_root_last, graph_a_package_the_root_does_not_reach_is_left_out -->
* A package imports `std`, the core libraries and its own direct dependencies by name, and nothing else by name. A file does not import its own package by name. <!-- test: graph_a_package_sees_its_own_dependencies_and_not_theirs, graph_a_package_does_not_import_itself_by_name -->
* `mote run` needs an entry file with a `main`. <!-- test: pipeline_run_needs_an_entry_file_with_a_main -->

### Zip packages

* A package is shared as a zip file with its `mote.toml` at the top: made by `mote package`, by `git archive --format=zip`, or by a host's download of a repository, whose single top folder is dropped. <!-- test: archive_packs_the_files_in_path_order_and_names_the_zip_after_the_package, archive_unpacks_a_zip_with_one_top_folder_without_the_folder, packages_a_git_archive_of_a_package_is_added_and_run, packages_an_archive_with_one_top_folder_as_a_host_makes_it_is_added_and_run -->
* The build unpacks a zip once into `<home>/cache/<sha256 of the file>/` and reads the package there; the home is `$MOTE_HOME`, else `~/.mote`. A changed zip is another directory. <!-- test: cache_the_home_is_mote_home_else_dot_mote_below_home, cache_unpacks_a_zip_once_into_a_directory_named_by_its_hash, zip_the_same_zip_twice_is_one_package_and_a_changed_zip_is_another_directory -->
* A package that came from a zip has the dependencies of the project that lists it: each name in its manifest is the root's entry of that name, and a name the root lacks is an error that says to add it. <!-- test: zip_the_dependencies_of_a_zip_package_are_the_roots_entries_of_those_names, zip_a_package_that_needs_a_name_the_root_lacks_is_an_error -->
* A missing zip, a missing home, a file that is not a zip and a package with another name are errors at the dependency's line. <!-- test: zip_a_missing_file_a_missing_home_a_bad_zip_and_another_name_are_errors_at_the_entry -->
* Packing keeps the files below the package in path order with a fixed time, so the same source makes the same bytes from any directory; it leaves out the top-level `target` and `deps` and every name that starts with `.`, and refuses a symbolic link. <!-- test: archive_leaves_out_target_deps_and_dot_names, archive_the_same_files_make_the_same_bytes, archive_a_symbolic_link_cannot_be_packed, packages_the_same_source_in_two_directories_makes_the_same_zip -->
* Unpacking refuses a path that leaves the package, a symbolic link, a zip without a `mote.toml` at its top, more than 20,000 files and more than 512 MiB. <!-- test: archive_a_path_that_leaves_the_package_is_an_error, archive_a_symbolic_link_in_a_zip_is_an_error, archive_a_zip_without_a_manifest_or_that_is_not_a_zip_is_an_error, archive_a_zip_with_too_many_files_or_bytes_is_an_error, archive_the_limits_are_the_documented_ones -->
* A `native/` folder in the zip is the package's natives, found as they are in a directory. <!-- test: packages_a_zip_with_a_native_library_keeps_the_native_beside_the_package -->

### Core libraries

* The core libraries are the packages of the `core/` folder; the toolchain embeds their source as it does the std, and `--core <dir>` uses the packages in a directory instead. <!-- test: core_embed_carries_every_package_of_the_core_folder, core_embed_a_directory_holds_one_folder_for_each_library, core_embed_a_folder_named_otherwise_than_its_package_is_an_error, core_embed_a_missing_directory_is_an_error -->
* A package imports a core library by its name with no entry in `mote.toml`, as it imports `std`; one that nothing imports is not built, and a core library imports another the same way. <!-- test: core_embed_an_import_of_a_core_name_adds_the_library_with_no_manifest_entry, core_embed_a_library_nothing_imports_is_left_out, core_embed_a_library_that_imports_another_brings_it_first, core_embed_a_dependency_package_imports_a_core_library_too, core_embed_the_tests_of_the_root_import_a_library_only_when_they_are_built, core_embed_a_library_does_not_import_itself, core_embed_an_unknown_name_is_still_an_unknown_package, core_tools_an_app_imports_a_core_library_by_name_and_runs, core_tools_a_core_name_without_the_library_is_an_unknown_package -->
* The name of a core library is reserved: no package, dependency or `mote init` may take it, except the root that lies in the directory of that library, which stands in for it (that is how `tools/test-core.sh` tests a library). <!-- test: core_embed_a_package_named_as_a_core_library_is_reserved, core_embed_the_root_in_the_directory_of_a_library_stands_in_for_it, core_tools_an_app_named_as_a_core_library_is_reserved -->
* Two core libraries that import each other are an error. <!-- test: core_embed_a_cycle_between_libraries_is_an_error -->
* The natives of an embedded library are looked up in `<home>/core/<name>/native/<arch>-linux/`, where a release puts them; those of a `--core` directory are in the library's own folder. <!-- test: core_embed_the_natives_of_an_embedded_library_are_looked_up_under_the_home -->

### Tests

* `tests/` belongs to the root package. Each file is a module `<package>.tests.<path>`, and a dependency's `tests/` is not read. <!-- test: graph_tests_are_modules_of_the_root_under_tests, graph_a_dependency_has_no_tests_in_the_graph -->
* A file of `tests/` imports its siblings with `.` and `..`, and the package's `src/` by the package name; `src/` does not reach `tests/`, and a file of `tests/` does not reach `src/` with `.`. <!-- test: graph_a_test_file_imports_the_package_by_name_and_its_siblings_by_dot, graph_a_test_file_does_not_reach_src_by_dot_and_src_does_not_reach_tests, graph_a_file_outside_tests_still_cannot_import_its_own_package_by_name, graph_a_cycle_between_test_files_is_an_error -->
* `mote check` and `mote test` build `tests/`; `mote run` leaves it out, even when it does not compile. <!-- test: pipeline_test_and_check_targets_include_the_test_blocks_of_tests, pipeline_run_target_leaves_tests_out_even_when_they_do_not_compile, graph_tests_are_left_out_unless_asked_for, cli_run_test_includes_the_tests_directory_after_src_and_run_ignores_it -->
