# Mote ABI

Value layout and runtime contract between `semantics`, `codegen`, `heap` and `runtime`. Offsets are bytes.

A *value* is what a binding, field or register holds. An *object* is a heap allocation that starts with a header. A *reference* is one pointer to an object, or null. A *slot* is a place inside a value or object where the collector looks for a reference.

* Words are 8 bytes, every pointer is one word, and values are stored little-endian. <!-- test: layout_the_offsets_the_documents_name_are_these, bytes_values_are_little_endian -->

## Type ids

* `TypeId` is a `u32` index into the `TypeTable`; `TypeId::NONE` is 0 and is reserved so a zeroed word never names a type. <!-- test: none_is_the_reserved_first_type, a_zeroed_word_names_no_type -->
* An id is stored in the low half of an object header and passed to the runtime as `I32`. <!-- test: the_type_id_and_flags_round_trip, allocation_takes_the_context_and_a_type_id_and_returns_a_reference -->
* The table holds for each type a name (for `typeof`), the `Kind`, the value layout, field and variant offsets, slots and the heap shape. <!-- test: kinds_a_type_holds_what_typeof_and_the_collector_need -->

## The table of a program

* `mote_codegen::build_types` gives every type the IR names one entry, in the order the walk meets them, so one program gives one table. <!-- test: typetable_every_std_test_file_gets_a_table_with_an_entry_for_each_register, typetable_every_type_of_a_register_has_an_entry_and_a_second_build_gives_the_same_bytes -->
* `Bool`, `Int8` and `UInt8` are `I8`, `Int16` and `UInt16` are `I16`, `Int32` and `UInt32` are `I32`, `Int` and `UInt64` are `I64`, `Float32` is `F32` and `Float` is `F64`; each type keeps its own entry, so its `TypeId` differs from the other types of the same scalar. <!-- test: typetable_numbers_and_bool_are_scalars_and_each_type_keeps_its_own_entry -->
* A struct and a tuple are one kind, `Struct`, with the fields or elements inline. <!-- test: typetable_a_struct_is_its_fields_inline_and_travels_in_registers, typetable_a_tuple_is_a_struct_of_its_elements -->
* A class is a reserved object, so it may name itself. <!-- test: typetable_a_class_is_an_object_with_a_reference_and_may_hold_itself -->
* An enum field that holds the enum by value, directly or through a tuple, optional or fixed array, is held in a `Box`; the box of a type is `TypeMap::boxed`. <!-- test: typetable_a_recursive_enum_holds_its_children_in_boxes, typetable_a_value_type_that_holds_itself_through_an_optional_is_boxed_too -->
* A union is one `Pair`, and each member that is carried in a box has the box. <!-- test: typetable_a_union_is_a_pair_and_a_member_that_is_a_value_has_a_box -->
* A `List<T>` has the `[T]` of its buffer, and a `Sender<T>` or `Receiver<T>` has a `Channel` over `[T]`, found by `TypeMap::channel`. <!-- test: typetable_a_list_has_a_buffer_array_and_an_array_strides_by_its_element, typetable_handles_have_their_kinds_and_a_channel_is_behind_its_ends -->
* A function that makes a closure, a stream frame or a function value has its own `Closure` entry over its captures, a `var` shared with a closure as a `Box`, and `TypeMap::closure` finds it. <!-- test: typetable_a_closure_keeps_its_captures_and_a_shared_var_is_a_cell -->
* A named function used as a value is a closure with no captures. <!-- test: typetable_a_named_function_used_as_a_value_is_a_closure_with_no_captures -->

## Compiled code

* A compiled function uses the Cranelift `tail` convention (checked on 0.116.1), with the task context as its first parameter and the arguments after it as in [Calls](#calls). <!-- test: jit_a_function_is_tail_convention_with_the_context_first -->
* A narrow integer is held in its own width; an operation is done in 64 bits and a result that does not fit its width is an `IntegerOverflow` fault. <!-- test: differential_scalar_the_compiled_code_shows_the_recorded_results, differential_scalar_the_compiled_code_shows_the_recorded_results -->
* A signed integer widens by sign, an unsigned one by zero, and each compares and divides by its own signedness. <!-- test: differential_scalar_the_compiled_code_shows_the_recorded_results -->
* Each check adds one site to the `FaultTable` with its kind and the span of the operation, and each function prologue adds a `StackDepth` site for the stack limit check. <!-- test: jit_each_check_has_a_site_with_its_kind_and_the_prologue_has_the_stack_site -->
* A safepoint poll loads the context's `poll` word and branches to a cold block that calls `Entry::Poll`; every function polls in its prologue after the stack check, and every loop header polls at its start. <!-- test: gc_polls_a_recursion_with_no_loop_is_stopped_before_it_ends, gc_polls_a_stopped_recursion_goes_on_and_ends_with_its_result, gc_polls_a_loop_with_no_call_is_stopped_before_it_ends -->
* A poll is placed before every instruction that can allocate or call a function that can, and a collection runs only inside `Entry::Poll`. <!-- test: gc_maps_a_program_that_holds_references_in_every_kind_of_frame_prints_what_it_prints_without_collection -->
* Each register that holds a `Ref` has a word in the frame, zeroed in the prologue and written at every definition of the register; a register held in memory is scanned by its type. <!-- test: gc_maps_a_program_that_holds_references_in_every_kind_of_frame_prints_what_it_prints_without_collection, gc_maps_pairs_enums_closures_and_cells_live_across_collections_keep_what_they_hold -->
* The first word of a frame is zero and is listed at every call so that Cranelift makes a stack map for each call site; each call site gets a frame descriptor, by return address, from the frame size and the offsets of those words and slots, and the probe is not in it. <!-- test: jit_each_call_has_a_descriptor_that_lists_the_words_of_the_references_and_never_the_probe -->
* A program runs as the main task of a scheduler, on a stack of its own, and a fault of that task ends the run and gives the host its kind, message and span. <!-- test: differential_scalar_the_compiled_code_shows_the_recorded_results -->
* A program with a detached task that faulted and nobody awaited ends with that fault when `main` returns. <!-- test: differential_tasks_the_compiled_code_shows_the_recorded_results, differential_tasks_the_compiled_code_shows_the_recorded_results -->
* `Spawn` makes the closure of its function over the captured values and calls `TaskSpawn` with the type of the task, the address of the function made for that type, the closure and the scope token or -1. <!-- test: differential_tasks_the_compiled_code_shows_the_recorded_results -->
* The function made for a task type takes the context, the closure and the handle, calls the closure through its code pointer, and stores the result in the handle at the end of its fixed words: by its leaves, or through the address of the payload when the result is passed in memory. <!-- test: differential_tasks_the_compiled_code_shows_the_recorded_results -->
* `task_await` gives `Ok` of the payload when the task returned, else `Err` of an `Error` with the kind `Other` for a fault or `Interrupted`, and the message `TaskMessage` gives; `task_cancel`, `task_is_ready` and `spawn_call` call their entries. <!-- test: differential_tasks_the_compiled_code_shows_the_recorded_results -->
* A `Task` of one result read as a `Task` of a wider result is a `task_view`: a new handle of the wider type with the same `runtime` number and a `convert` closure over the task that gives the converted result; `task_await` and `task_value` give what that closure gives when the word is set, else the payload, and the branch is emitted only for the task types a view makes. <!-- test: differential_tasks_a_task_widened -->
* `ScopeBegin` and `ScopeEnd` call their entries; a fault of a task of the scope that nobody awaited gives an `Err` of an `Error` of kind `Other` to a function that returns a `Result<_, Error>` (through `ScopeEndError`), and stops the task with that fault in any other function. <!-- test: differential_cancel_a_scope_with_a_faulted_task_gives_an_error_to_a_function_returning_a_result, differential_tasks_the_compiled_code_shows_the_recorded_results -->
* `Defer` records the resource of a `with` and its close function with the task, and `Undefer` forgets the innermost one; a cancelled task calls the close functions of the ones left, innermost first. <!-- test: differential_cancel_a_cancelled_task_closes_its_resources_innermost_first_and_a_fault_closes_none, differential_cancel_the_compiled_code_shows_the_recorded_results -->
* A value that the close function of a `with` gives is dropped when a cancelled task calls it. <!-- test: differential_gate_every_block_of_std_tests_gives_the_recorded_verdict_and_output -->
* What a program prints is kept by the runtime until the host takes it, and prints a number as `print` shows it. <!-- test: jit_a_run_returns_what_it_printed, helpers_a_scalar_prints_as_its_text, entries_the_text_a_program_prints_is_kept_until_the_host_takes_it -->
* A register of a number or `Bool` type is a Cranelift variable; a register of a struct, tuple, enum, tagged optional, union or fixed array type is a stack slot laid out as the type table says, zeroed when the function starts. <!-- test: differential_values_the_compiled_code_shows_the_recorded_results -->
* A value in memory is passed as [Calls](#calls) says: its leaves are loaded from the slot for the call and stored in the callee's slot, and a value passed in memory is copied to a slot the caller makes. <!-- test: jit_a_value_past_four_leaves_goes_by_address_and_a_small_one_by_its_leaves, differential_values_the_compiled_code_shows_the_recorded_results -->
* A function with `var` parameters takes one more `I64` parameter after its arguments: the address of a block where it stores the final value of each `var` parameter before it returns, each at the next offset aligned to its type, and the caller copies them out. <!-- test: jit_a_var_parameter_adds_one_last_parameter_for_the_block_of_final_values, differential_values_the_compiled_code_shows_the_recorded_results -->
* The tag of an enum is read at its size, `!` on a `Result` compares it with the position of `Ok`, and `!` on a tagged optional compares its tag word with zero; a mismatch is `UnwrapFailed`. <!-- test: differential_values_the_compiled_code_shows_the_recorded_results, differential_values_the_compiled_code_shows_the_recorded_results -->
* An index of a fixed array outside `0..N` is `IndexOutOfRange`. <!-- test: differential_values_the_compiled_code_shows_the_recorded_results -->
* A union converted from a scalar member holds the `TypeId` of the member in its type word and the scalar in the low bytes of its payload word; a member that is an object is held by its reference, and any other member is copied into a new `Box` that the payload word refers to. <!-- test: differential_values_the_compiled_code_shows_the_recorded_results, differential_objects_the_compiled_code_shows_the_recorded_results -->
* An object is allocated by `mote_alloc` or, for a string or array, `mote_alloc_array` with its length, and comes back zeroed; a class, a record of a class and an enum payload in a `Box` are written field by field after the allocation. <!-- test: differential_objects_the_compiled_code_shows_the_recorded_results -->
* A string constant is an object in static data, outside the heap: a header with `FLAG_STATIC`, its length, its bytes and zero padding to a whole word. <!-- test: jit_a_string_constant_is_an_object_with_a_header_a_length_and_its_bytes_padded_to_a_word -->
* `+` on strings and `Concat` allocate one string of the summed length and copy each part; `Show` of a number or `Bool` writes its text to a buffer of 400 bytes and copies it to a new string; strings compare by their bytes. <!-- test: differential_objects_the_compiled_code_shows_the_recorded_results, helpers_the_text_of_a_scalar_is_written_to_a_buffer_and_the_longest_fits, helpers_strings_compare_by_their_bytes -->
* An optional class, string or other object is its reference, null being `None`, and `!` on it is `UnwrapFailed` when it is null. <!-- test: differential_objects_the_compiled_code_shows_the_recorded_results -->
* Every store is emitted by `Lowering::store`, and a reference is stored through `store_reference`, which emits no barrier. <!-- test: jit_every_store_is_emitted_by_one_function -->
* `Detach` of a value that holds no reference is a copy; one that holds references is a `deep_clone`. <!-- test: differential_objects_the_compiled_code_shows_the_recorded_results, differential_tasks_the_compiled_code_shows_the_recorded_results -->
* A global has a cell of the size of its type, zeroed. <!-- test: differential_values_the_compiled_code_shows_the_recorded_results -->
* An array literal and `[x; n]` allocate an array of the length with `mote_alloc_array` and write each element at its stride; a list literal allocates a buffer array and a list that holds its length, its capacity and the buffer. <!-- test: differential_sequences_the_compiled_code_shows_the_recorded_results -->
* `Index` and `SetIndex` on an array, a list or a fixed array compare the position, as an unsigned number, with the length, and a position at or past it is `IndexOutOfRange`. <!-- test: differential_sequences_the_compiled_code_shows_the_recorded_results, differential_sequences_the_compiled_code_shows_the_recorded_results -->
* `[x; n]` is `IndexOutOfRange` when `n` is negative or more than 8388608. <!-- test: differential_sequences_the_compiled_code_shows_the_recorded_results -->
* A `Slice` is a new array, list or string of the range, and is `IndexOutOfRange` when the start is negative, the end is before the start or past the length, a step is less than 1, or a string is cut inside a character; a step takes every step-th element. <!-- test: differential_sequences_the_compiled_code_shows_the_recorded_results, differential_sequences_the_compiled_code_shows_the_recorded_results, helpers_a_cut_inside_a_character_is_refused -->
* A `SetSlice` on a list copies another list of the range's length over it, or writes one value to every element of the range, and is `IndexOutOfRange` for a range outside the list or a list of another length. <!-- test: differential_sequences_the_compiled_code_shows_the_recorded_results, differential_sequences_the_compiled_code_shows_the_recorded_results -->
* `list_push` on a full list allocates a buffer of twice the capacity (4 when it was empty), copies the elements to it and stores it through `store_reference`; the buffer's length word is its capacity. <!-- test: differential_sequences_the_compiled_code_shows_the_recorded_results -->
* A `Range` is a struct of its start, its end and whether the end is included. <!-- test: differential_sequences_the_compiled_code_shows_the_recorded_results -->
* A function that is a closure body or is made into a value takes the closure after the context and the result slot; a direct call of such a function that captures nothing passes null, and one that captures is not compiled. <!-- test: jit_a_function_called_through_a_closure_takes_it_after_the_context, differential_functions_the_compiled_code_shows_the_recorded_results -->
* A closure that captures nothing is a static object outside the heap: a header of its closure type and the code pointer; one that captures is allocated with the code pointer at 8 and the captures from 16 in the order of the table. <!-- test: differential_functions_the_compiled_code_shows_the_recorded_results -->
* A `var` that a closure captures lives in a `Box` both sides hold; a closure keeps the reference to the box, and the function that made it reads and writes the value at offset 8. <!-- test: differential_functions_the_compiled_code_shows_the_recorded_results -->
* A call through a function value loads the code pointer at offset 8 and calls it with the signature of the function type, with a block for the final values of its `var` parameters last. <!-- test: differential_functions_the_compiled_code_shows_the_recorded_results -->
* Compiled code runs start-up, then one entry: `main` or a `test` block; a `test` block that faults fails alone, and a program with no `main` runs start-up only. <!-- test: differential_functions_a_test_block_compiled_gives_its_verdict_and_output, docs_every_block_is_run_rejected_as_marked_or_listed_as_a_fragment -->
* An entry that returns a `Result` keeps it in a data object; when the tag is not `Ok`, the run state holds 1 and the text of the error, the `message` of an `Error` or what the compiled `std.reflect.debug_text` of the error type gives, which the host reads before it frees the code. <!-- test: jit_an_entry_that_returns_an_err_gives_the_message_of_its_error, jit_an_entry_that_returns_an_ok_has_no_error, differential_gate_a_main_that_returns_an_err_ends_with_the_text_of_the_error, differential_gate_a_main_that_returns_an_ok_returns -->
* A `wrapping_*`, `checked_*`, `saturating_*` or `overflowing_*` operation on an integer of 64 bits uses the overflow flag of the instruction; on a narrower integer it computes the exact value in 64 bits and compares it with the value cut to the width. <!-- test: differential_numeric_the_compiled_code_shows_the_recorded_results -->
* A `checked_*` result is the optional's tag word, 1 for a value and 0 for none, with the payload beside it; an `overflowing_*` result is the pair of the wrapped value and the overflow flag. <!-- test: differential_numeric_the_compiled_code_shows_the_recorded_results -->
* `floor`, `ceil` and `sqrt` are instructions; `round` rounds half away from zero, and it and the other `math_*` functions call `mote_math`. <!-- test: differential_numeric_the_compiled_code_shows_the_recorded_results -->
* `float_to_int` truncates, and gives 0 for NaN and the least or greatest `Int` out of range. <!-- test: differential_numeric_the_compiled_code_shows_the_recorded_results -->
* `string_len`, `string_concat` and `string_slice` are emitted inline: the length is the word at offset 8, and a slice has the faults of `Slice` on a string. <!-- test: differential_strings_the_compiled_code_shows_the_recorded_results, differential_strings_the_compiled_code_shows_the_recorded_results -->
* The other `string_*` rows and `parse_*` call `mote_text`, which allocates its result from the context's region; a result of 0 is the `MemoryLimit` fault, and a `repeat` of more than 64 MiB is that fault too. <!-- test: differential_strings_the_compiled_code_shows_the_recorded_results, differential_strings_the_compiled_code_shows_the_recorded_results -->
* `string_split` gives a list whose length and capacity are the number of parts and whose buffer holds the parts; `string_bytes` gives an array of the bytes of the string. <!-- test: differential_strings_the_compiled_code_shows_the_recorded_results -->
* `string_from_bytes` is `null` for bytes that are not UTF-8. <!-- test: helpers_bytes_that_are_not_utf8_are_refused_and_valid_ones_make_a_string -->
* `string_find`, `string_rfind`, `parse_int` and `parse_float` give an optional number as the tag word and the payload. <!-- test: differential_strings_the_compiled_code_shows_the_recorded_results -->
* `==` and `!=` of an optional number with another or with a number of its type compare the tags and then the payloads. <!-- test: differential_strings_the_compiled_code_shows_the_recorded_results -->
* `Show` of a number or `Bool` writes the text through `mote_show` for every scalar width. <!-- test: differential_strings_the_compiled_code_shows_the_recorded_results -->
* `list_get`, `list_set`, `array_get`, `array_set`, `array_len`, `list_pop`, `list_clear`, `list_slice`, `array_slice`, `list_clone`, `array_clone` and `list_set_slice` are emitted inline with the checks of `Index`, `Slice` and `SetSlice`; a clone is a slice of the whole. <!-- test: differential_collections_the_compiled_code_shows_the_recorded_results, differential_collections_the_compiled_code_shows_the_recorded_results -->
* `list_pop` gives the last element and shortens the list, and gives none for an empty one: a null reference for a reference element, else the tag word 0. <!-- test: differential_collections_the_compiled_code_shows_the_recorded_results -->
* `equal_of`, `key_eq`, `list_contains`, `array_contains` and `==` on a value that is not a number call `mote_value`, which reads both values by the type table: contents for strings, lists, arrays, structs, enums, optionals and unions, identity for classes, closures and handles. <!-- test: differential_collections_the_compiled_code_shows_the_recorded_results -->
* As a key, `-0.0` equals `0.0` and `NaN` equals `NaN`; as a value, `NaN` equals nothing. <!-- test: walk_nan_is_not_equal_to_itself_as_a_value_but_is_as_a_key, walk_keys_that_are_equal_hash_alike_and_zero_has_one_hash -->
* `hash_of` is a 63-bit hash of the key's contents seeded once per process, so keys that are equal as keys hash alike. <!-- test: walk_keys_that_are_equal_hash_alike_and_zero_has_one_hash, differential_collections_the_compiled_code_shows_the_recorded_results -->
* `deep_clone` copies everything reachable through classes, boxes, lists and arrays once each, shares strings, closures and handles, and is the `MemoryLimit` fault when the heap is full. <!-- test: differential_collections_the_compiled_code_shows_the_recorded_results -->
* `==` and `contains` compare a `Map` or `Set` by its live entries in any order, with the keys as keys; a key and `hash_of` take it by address; `deep_clone` of a `Map` recomputes the hashes and slots of the copy, since its keys held by address have new addresses. <!-- test: differential_collections_a_map_inside -->
* `fault` calls `mote_fault_message` with the message and has no fault site; `typeof` is the static string object of the rendered type, and of a union the one of the member whose type number the value holds. <!-- test: differential_collections_the_compiled_code_shows_the_recorded_results, differential_collections_the_compiled_code_shows_the_recorded_results, differential_gate_typeof_a_union_names_the_type_it_holds -->
* `print` and `Show` of a value that is not a number, `Bool` or string call `mote_value`, which writes it from the type table and the names the code generator keeps in the runtime's data. <!-- test: differential_meta_the_compiled_code_shows_the_recorded_results -->
* Static reflection (`std.reflect`) lowers at compile time with no runtime reflection tables or dynamic `Dyn` boxes. <!-- test: differential_meta_the_compiled_code_shows_the_recorded_results -->
* `require` calls `mote_fault_message` with its message when the condition is false and has no fault site. <!-- test: differential_meta_the_compiled_code_shows_the_recorded_results -->
* A `Stream` is a closure object of its resume function: the code pointer at 8, then the frame slots in the order of `Function.frame`, slot 0 being the state; `Stream` allocates it with the state zero and the initial values in the slots after, and `FrameLoad` and `FrameStore` read and write a slot of the closure the resume function was called through, a cell slot holding the reference to its `Box`. <!-- test: differential_streams_compiled_generators_yield_in_order_widen_and_catch_a_read_from_inside, differential_streams_the_compiled_code_shows_the_recorded_results -->
* `stream_next` calls the code pointer of the stream with the stream as the closure, and the result is the next element or `null`; a read of a stream from inside its own body is the fault `a stream was read while it runs`. <!-- test: differential_streams_compiled_generators_yield_in_order_widen_and_catch_a_read_from_inside -->
* A `Stream` read as a `Stream` of a wider element is a new stream over a resume function that reads the inner one and converts each element. <!-- test: differential_streams_compiled_generators_yield_in_order_widen_and_catch_a_read_from_inside -->
* Every row of the intrinsic table is emitted except `type_id`, which is a constant; a call of a row left out is a `CompileError`. <!-- test: table_complete -->
* `==` and `!=` of two `Null` values are constants, and `as` on an enum whose variants all have constants selects the constant of the variant by its tag and cuts it to the integer type. <!-- test: differential_gate_unit_compares_equal_to_null_and_an_enum_with_constants_casts_to_an_integer -->
* Every `test` block of `std/tests`, every runnable block of `docs/reference` and every program of the corpus gives the verdict, result, fault kind and output recorded for it when compiled, with the optimizer or without, except the blocks `integration/gate_refused.txt` lists, which need a library the machine lacks. <!-- test: differential_gate_every_block_of_std_tests_gives_the_recorded_verdict_and_output, differential_optimized_every_block_of_std_tests_gives_the_recorded_verdict_and_output, docs_every_block_is_run_rejected_as_marked_or_listed_as_a_fragment, docs_every_pass_of_optimize_shows_what_the_unoptimized_run_of_a_block_shows, differential_scalar_the_compiled_code_shows_the_recorded_results, differential_values_the_compiled_code_shows_the_recorded_results, differential_objects_the_compiled_code_shows_the_recorded_results, differential_sequences_the_compiled_code_shows_the_recorded_results, differential_functions_the_compiled_code_shows_the_recorded_results, differential_numeric_the_compiled_code_shows_the_recorded_results, differential_strings_the_compiled_code_shows_the_recorded_results, differential_collections_the_compiled_code_shows_the_recorded_results, differential_meta_the_compiled_code_shows_the_recorded_results, differential_tasks_the_compiled_code_shows_the_recorded_results, differential_channels_the_compiled_code_shows_the_recorded_results, differential_sync_the_compiled_code_shows_the_recorded_results, differential_timed_the_compiled_code_shows_the_recorded_results, differential_ports_the_compiled_code_shows_the_recorded_results, differential_cancel_the_compiled_code_shows_the_recorded_results, differential_deadlock_the_compiled_code_shows_the_recorded_results, differential_streams_the_compiled_code_shows_the_recorded_results -->
* A recorded result (`name.expect` beside a corpus program and beside a `std/tests` file) holds how `main` ended and its output, or each block's verdict and output; it changes only when the tests run with `MOTE_BLESS=1`, which prints the lines that changed. <!-- test: differential_a_recorded_file_changes_only_when_recording_and_a_difference_is_shown, differential_a_recorded_result_gives_the_ending_and_the_output_lines -->

## Scalars

<!-- table: abi-scalars -->
| Scalar | Bytes |
|---|---|
| `I8` | 1 |
| `I16` | 2 |
| `I32` | 4 |
| `I64` | 8 |
| `F32` | 4 |
| `F64` | 8 |
| `Ref` | 8 |
<!-- /table -->

* A scalar's alignment is its size. <!-- test: a_scalar_aligns_to_its_size -->
* Only `Ref` is a reference. <!-- test: only_ref_is_a_reference -->
* A value of any type is at most 8-aligned. <!-- test: layout_every_type_aligns_to_a_power_of_two_of_at_most_eight_and_every_object_is_whole_words -->

## Layout

* `Layout { size, align }`: `align` is a power of two and `size` is a multiple of `align`. <!-- test: layout_every_type_aligns_to_a_power_of_two_of_at_most_eight_and_every_object_is_whole_words -->
* `align_up(offset, align)` rounds `offset` up to a multiple of `align`. <!-- test: align_up_rounds_to_the_next_multiple -->
* Fields are placed in declaration order, each at the next offset that fits its alignment; nothing is reordered. <!-- test: struct_fields_follow_declaration_order_with_padding -->
* A type's size is its end offset rounded up to its alignment, which is the largest field alignment. <!-- test: struct_fields_follow_declaration_order_with_padding -->

```text
struct { Int8, Int, Int8 }
 0     8     16        size 24, align 8
```

## Kinds

<!-- test: kinds_each_kind_has_the_layout_the_table_says -->
| Kind | Value or object | Value layout |
|---|---|---|
| `Unit` | value | 0 bytes, align 1 |
| `Scalar` | value | the scalar |
| `Struct` | value | fields inline |
| `Enum` | value | tag + largest variant |
| `Optional` | value | a reference, or tag word + payload |
| `Pair` | value | 16 bytes |
| `FixedArray` | value | `N` inline elements |
| `Class` `Box` `String` `Array` `List` `Closure` `Handle` | object | one reference |

* A value kind names only types already in the table, so it cannot contain itself; a recursive type goes through an object kind that is reserved first. <!-- test: kinds_a_value_type_cannot_name_itself, a_class_can_refer_to_itself_through_an_optional -->

## Structs

* Fields are inline, and a nested struct is flattened into its parent at an aligned offset. <!-- test: nested_structs_are_flat_and_keep_their_reference_slots -->
* An empty struct has size 0. <!-- test: unit_and_empty_structs_take_no_room -->
* A `[Point]` array and a `List<Point>` hold their elements inline in one block, with no object per element. <!-- test: array_of_structs_is_one_block_of_inline_elements -->

```text
struct Point { x: Int, y: Int }        size 16: x@0 y@8
struct Line { a: Point, b: Point }     size 32: a@0 b@16
```

## Enums

* The tag is at offset 0, then each variant's fields follow it, aligned as usual; the size is the largest variant, rounded up. <!-- test: an_enum_holds_its_largest_variant_after_the_tag, a_small_payload_sits_right_after_the_tag -->
* A variant's tag is its position in the type's `variants`, and its fields and slots are measured from the start of the enum. <!-- test: enum_with_no_variants_has_no_size_and_the_tag_is_the_variants_position, an_enum_with_references_has_one_dynamic_slot -->

<!-- test: enum_the_tag_is_one_two_or_four_bytes_by_the_variant_count -->
| Variants | Tag size |
|---|---|
| up to 256 | 1 byte |
| up to 65,536 | 2 bytes |
| more | 4 bytes |

* A unit-only enum is just its tag. <!-- test: a_unit_only_enum_is_its_tag -->
* An enum with no variants has size 0. <!-- test: enum_with_no_variants_has_no_size_and_the_tag_is_the_variants_position -->

```text
enum Shape { Circle(Int), Rect(Int, Int), Empty }
 tag@0   Circle: Int@8   Rect: Int@8 Int@16        size 24, align 8

enum E { A(Int32) }       tag@0  Int32@4            size 8, align 4
```

## Optionals

* `T?` where `T` is an object is the reference itself, null for `None`. <!-- test: an_optional_of_an_object_is_the_pointer_itself -->
* Any other `T?` is an enum of two variants with an 8-byte tag word: variant 0 is `None`, variant 1 is `Some(T)` at offset 8. <!-- test: an_optional_of_a_scalar_is_a_tag_word_and_a_payload -->
* An optional of an optional needs the tag, because the inner null is already `None`. <!-- test: an_optional_of_an_optional_object_needs_a_tag -->
* An optional of a union, an enum or a struct always has the tag. <!-- test: optional_of_a_union_an_enum_or_a_struct_uses_the_tag_form -->

```text
Int?                tag@0 (0 = None, 1 = Some)  Int@8        size 16
Point?              tag@0                       Point@8      size 24
String?             one reference, null = None  size 8
String??            tag@0  String?@8            size 16
```

## Unions

* A union is a pair of two words: the member's `TypeId` at offset 0 as a 64-bit word and the payload at offset 8; size 16, align 8. <!-- test: a_pair_is_a_type_word_and_a_payload_word -->
* One kind, `Pair`, stands for every union type. <!-- test: a_pair_is_a_type_word_and_a_payload_word -->
* A zeroed pair has type id 0, which no value has. <!-- test: zero_a_zeroed_value_is_none_the_first_variant_null_or_the_none_pair -->

<!-- test: pair_payload_is_inline_a_reference_or_a_box_by_the_member_kind -->
| Member kind | Payload word |
|---|---|
| scalar, `Unit` | the scalar itself, in the low bytes (`Float32` is its bits) |
| object reference, optional object | the reference |
| struct, enum, tag-form optional, fixed array, pair | a reference to a `Box` holding the value |

* `pair_payload(id)` returns `Inline`, `Reference` or `Boxed` for a member type. <!-- test: pair_members_are_inline_references_or_boxed, pair_payload_is_inline_a_reference_or_a_box_by_the_member_kind -->
* The payload word is a reference only for `Reference` and `Boxed` members, so it is a dynamic slot: the collector reads the type id to decide. <!-- test: a_pair_is_a_type_word_and_a_payload_word -->

## Fixed arrays

* `[T; N]` is `N` elements inline; the stride is `size(T)` rounded to `align(T)` and the size is `stride * N`. <!-- test: a_fixed_array_is_inline_with_a_repeating_slot -->
* The collector scans one repeating slot, not one per element. <!-- test: a_fixed_array_is_inline_with_a_repeating_slot -->
* A size over 4 GiB panics when the type is added. <!-- test: fixed_array_over_4_gib_panics_when_the_type_is_added -->

```text
[Int32; 3]     size 12, align 4
```

## The header

* Every object starts with one 8-byte `Header`: the object's `TypeId` in the low 32 bits and flags in the high 32 bits. <!-- test: the_type_id_and_flags_round_trip, the_top_type_id_does_not_reach_the_flags -->
* No object stores its own reference map: the type id leads to the type's `Object` shape and its slots. <!-- test: kinds_a_type_holds_what_typeof_and_the_collector_need -->
* A zeroed value is valid: `0`, `null`, `None`, the first enum variant and the none pair. <!-- test: zero_a_zeroed_value_is_none_the_first_variant_null_or_the_none_pair -->

## Header flags

<!-- test: header_flags_each_flag_is_one_bit_of_the_high_half -->
| Flag | Bit | Meaning |
|---|---|---|
| `FLAG_LARGE` | 0 | the object lives in the large-object space |
| `FLAG_FINALIZE` | 1 | the object owns an OS resource and is finalized when it dies |
| `FLAG_STATIC` | 2 | the object lives in static data outside the heap and holds references only to other static objects |

* The collector neither marks nor scans an object with `FLAG_STATIC`. <!-- test: static_a_static_object_is_neither_marked_nor_scanned -->
* Every other flag bit is reserved and stays zero; a header with one is refused. <!-- test: header_flags_every_other_bit_is_reserved, header_flags_a_reserved_bit_is_rejected -->

## Objects

* An object is its header, then fields or elements. <!-- test: a_class_starts_after_the_header, an_array_strides_by_its_element -->
* `Object { size, fields, slots, element }`: `size` is the fixed part, header included, a multiple of 8. <!-- test: layout_every_type_aligns_to_a_power_of_two_of_at_most_eight_and_every_object_is_whole_words -->
* `byte_size(count)` is `size + stride * count` rounded up to a word. <!-- test: object_size_rounds_to_a_word -->

<!-- test: a_class_starts_after_the_header, a_closure_keeps_its_code_pointer_before_its_captures, a_recursive_enum_holds_its_children_in_boxes, a_string_keeps_its_length_then_its_bytes, an_array_strides_by_its_element, a_list_points_at_a_separate_buffer, layout_the_offsets_the_documents_name_are_these, handles_the_payload_starts_after_the_words -->
| Object | Layout |
|---|---|
| `Class` | header@0, fields from 8 in declaration order |
| `Closure` | header@0, code pointer@8, captures from 16 |
| `Box` | header@0, value@8 |
| `String` | header@0, byte length@8, UTF-8 bytes from 16 |
| `Array` | header@0, length@8, elements from 16 |
| `List` | header@0, length@8, capacity@16, buffer reference@24; size 32 |
| `Handle` | header@0, the words of its kind, then its payload |

## Classes

* A class value is one reference. <!-- test: a_class_starts_after_the_header -->
* Fields that are values (structs, enums, optionals) sit inline in the object. <!-- test: a_class_starts_after_the_header -->
* A class may refer to itself through an optional or another object: reserve the type, then define it. <!-- test: a_class_can_refer_to_itself_through_an_optional -->

## Closures

* A function value is one reference to a closure object. <!-- test: a_closure_keeps_its_code_pointer_before_its_captures -->
* The captures are laid out like class fields, starting at 16. <!-- test: a_closure_keeps_its_code_pointer_before_its_captures -->
* A call through a closure loads the code pointer at offset 8 and passes the closure after the context and the result slot. <!-- test: callconv_a_closure_call_passes_the_closure_after_the_result_slot, layout_the_offsets_the_documents_name_are_these -->

## Strings

* A string is one immutable object: the byte length, then the UTF-8 bytes, with no terminator. <!-- test: a_string_keeps_its_length_then_its_bytes -->

```text
"hello"  header@0  5@8  'h' 'e' 'l' 'l' 'o'@16..21      byte_size 24
```

## Arrays and lists

* An `Array(T)` (`[T]`) is an object with a length and inline elements. <!-- test: an_array_strides_by_its_element -->
* A `List(T)` is a fixed 32-byte object that holds a reference to an `Array(T)` buffer. <!-- test: a_list_points_at_a_separate_buffer -->
* `Map` and `Set` are classes written in `std`; the ABI has no kind for them. <!-- test: abi_map_and_set_are_classes_with_the_fields_in_the_order_the_abi_names -->
* A `Map<K, V>` has the fields `held_keys: List<K>`, `held_values: List<V>`, `hashes: List<Int>`, `slots: [Int]` and `count: Int`, in that order; a `Set<T>` has one field, `members: Map<T, Null>`. <!-- test: abi_map_and_set_are_classes_with_the_fields_in_the_order_the_abi_names -->
* A key is compared by content when it is a scalar, a `String`, a tuple, a struct or an enum, and by identity when it is a class; a `Float`, `List`, `[T]`, `Map` or `Set` cannot be part of a key. <!-- test: differential_collections_the_compiled_code_shows_the_recorded_results, a_key_holds_no_float_and_no_collection_that_can_change -->

## Boxes

* A `Box(T)` is an object holding one value of `T` at offset 8. <!-- test: a_recursive_enum_holds_its_children_in_boxes -->
* A box carries a union member that is not a scalar or a reference, and the recursive children of enums. <!-- test: pair_payload_is_inline_a_reference_or_a_box_by_the_member_kind, a_recursive_enum_holds_its_children_in_boxes -->
* A recursive enum reserves its box type first, so the enum's payload is a plain reference. <!-- test: a_recursive_enum_holds_its_children_in_boxes -->

```text
enum Tree { Leaf(Int), Node(Box<Tree>, Box<Tree>) }
 size 24: tag@0  Leaf: Int@8  Node: ref@8 ref@16
```

## Slots

* Every type lists where a value of it holds references, from the start of the value; an object lists them from the start of the object. <!-- test: kinds_a_type_holds_what_typeof_and_the_collector_need -->

<!-- test: slots_shift_every_offset, an_enum_with_references_has_one_dynamic_slot, a_pair_is_a_type_word_and_a_payload_word, a_fixed_array_is_inline_with_a_repeating_slot, an_optional_of_an_object_is_the_pointer_itself -->
| Slot | Meaning |
|---|---|
| `Ref(offset)` | a reference at `offset` |
| `Enum { offset, ty }` | an enum at `offset`; read its tag, then apply that variant's slots |
| `Pair(offset)` | a pair at `offset`; read its type id to decide whether the payload is a reference |
| `Repeat { offset, count, stride, ty }` | `count` values of `ty`, `stride` apart |

* The slots of a nested value are shifted into its parent, so a struct's slots are already flat. <!-- test: nested_structs_are_flat_and_keep_their_reference_slots, slots_shift_every_offset -->
* A type with no references has no slots. <!-- test: struct_fields_follow_declaration_order_with_padding -->
* An array or a string describes its elements by an `Element`: the `stride` and `offset` apply to every element, which has the slots of the element type. <!-- test: an_array_strides_by_its_element, a_string_keeps_its_length_then_its_bytes -->

## Handles

* A handle is an object whose layout the runtime defines: the header, a fixed run of words for its kind, then its payload, which is typed like class fields. <!-- test: handles_each_word_sits_after_the_header_in_order, handles_the_payload_starts_after_the_words -->

<!-- table: abi-handles -->
| Handle | Words after the header | Payload types | Bytes before the payload |
|---|---|---|---|
| `Task` | `state`@8 `runtime`@16 `convert`@24 | 1 | 32 |
| `Sender` | `channel`@8 `open`@16 | 0 | 24 |
| `Receiver` | `channel`@8 `open`@16 | 0 | 24 |
| `Channel` | `senders`@8 `receivers`@16 `capacity`@24 `head`@32 `length`@40 `waiters`@48 | 1 | 56 |
| `Mutex` | `owner`@8 `waiters`@16 | 1 | 24 |
| `Once` | `state`@8 `runner`@16 `waiters`@24 | 1 | 32 |
| `AtomicInt` | `value`@8 | 0 | 16 |
| `AtomicBool` | `value`@8 | 0 | 16 |
| `Port` | `port`@8 | 0 | 16 |
<!-- /table -->

* The payload of a `Task` is its result, of a `Mutex` and a `Once` their value, and of a `Channel` the array that holds its messages. <!-- test: handles_the_payload_keeps_its_reference_slots, handles_a_channel_end_refers_to_its_channel_and_the_channel_to_its_buffer -->
* A `Task` holds a reference to its `convert` closure, which is null for a spawned task. <!-- test: handles_the_payload_keeps_its_reference_slots -->
* A sender and a receiver each hold a reference to their channel, which is the only reference among their words. <!-- test: handles_a_channel_end_refers_to_its_channel_and_the_channel_to_its_buffer -->
* The words other than those references are not scanned, and the payload is scanned by its type, so a payload with a dynamic word has the slot of that word. <!-- test: handles_a_payload_with_a_dynamic_word_is_scanned_by_its_type -->
* A `Channel`'s `senders` counts its open `Sender` ends, `receivers` is 1 until the receiving side closes, `capacity` is the number of messages it holds, `head` and `length` place the messages in the ring that is its payload, which has one place for a capacity of 0, and `waiters` is the number of its record in the runtime; a `Sender`'s `open` is 1 until it closes. <!-- test: channels_a_new_channel_has_one_sender_one_receiver_and_a_buffer_of_its_capacity, channels_a_rendezvous_channel_keeps_one_place_and_a_negative_capacity_is_refused, channels_a_ring_that_wrapped_keeps_the_order -->
* A handle lists exactly the payload types its kind takes. <!-- test: handles_every_kind_lists_the_payloads_it_takes, handles_a_payload_of_the_wrong_count_is_rejected -->
* A task's `state` is 0 pending, 1 done, 2 cancelled or 3 faulted; a `Once`'s `state` is 0 empty, 1 running or 2 done. <!-- test: handles_the_states_are_stable_numbers -->

## The task context

* The task context is the first parameter of every function, closure code pointer and runtime entry, passed as an `I64` that is not a reference. <!-- test: callconv_the_context_is_the_first_parameter_and_is_not_a_reference, every_entry_takes_the_context_first -->
* It is a block of six words that the runtime keeps for each task. <!-- test: callconv_the_context_is_six_words_with_the_region_first -->

<!-- table: abi-context -->
| `TaskContext` field | Offset |
|---|---|
| `region_cursor` | 0 |
| `region_limit` | 8 |
| `poll` | 16 |
| `stack_limit` | 24 |
| `fault_site` | 32 |
| `runtime` | 40 |
<!-- /table -->

* `fault_site` holds the raw id of the fault site that stopped the task, plus one, and is zero while none has. <!-- test: callconv_the_context_is_six_words_with_the_region_first -->
* A request for a poll stores 1 in `poll` with an atomic store, and the runtime's poll entry clears it. <!-- test: region_a_requested_poll_is_taken_once, entries_a_stop_asks_each_joined_context_to_poll_and_a_left_one_is_not_asked -->

## The allocation region

* A task allocates from its region: zeroed memory from `region_cursor` up to `region_limit`, both in the context. <!-- test: region_lives_in_the_first_two_words_of_the_context -->
* The inline sequence adds the object's size to the cursor and, when the result is not past the limit, stores the new cursor and writes the header at the old one; otherwise it calls `Entry::Alloc`. <!-- test: region_a_bump_returns_the_cursor_and_moves_it_past_the_object, region_an_object_ending_exactly_at_the_limit_fits, region_an_object_past_the_limit_leaves_the_cursor_for_the_slow_path -->
* The sequence does not wrap at the top of the address space. <!-- test: region_a_cursor_near_the_top_of_the_address_space_does_not_wrap -->
* Only an object of a fixed size of at most `MAX_INLINE_ALLOC` (256) bytes is allocated inline; an array, a string and a larger object call the runtime. <!-- test: region_only_a_small_fixed_size_object_is_allocated_inline -->
* A port is finalized, so it calls the runtime too. <!-- test: region_a_port_is_finalized_so_it_calls_the_runtime -->
* Compiled code allocates a small fixed-size object inline: the runtime is called once for a region and not for each object. <!-- test: gc_fastpath_small_objects_are_bumped_in_the_region_and_the_runtime_is_called_only_for_a_new_one -->
* With `torture` the runtime empties the region after each allocation, so each object calls the runtime and is followed by a poll. <!-- test: gc_fastpath_under_torture_every_object_calls_the_runtime -->

## The heap

* The heap is non-moving: an object keeps its address until it dies, and its fields are never touched by the collector. <!-- test: sweep_live_objects_are_never_moved_or_overwritten, mark_the_mark_bits_are_outside_the_object -->
* A reference is the address of an object's header, a multiple of 8. <!-- test: alloc_returns_zeroed_memory_with_the_header_written, torture_the_verifier_reports_references_that_dangle -->

<!-- test: blocks_the_sizes_are_32_kib_blocks_of_128_byte_lines_in_4_mib_chunks, alloc_objects_up_to_the_threshold_stay_in_a_block, alloc_an_object_past_the_threshold_gets_its_own_mapping_flagged_large -->
| Unit | Bytes |
|---|---|
| Line | 128 |
| Block | 32768 (256 lines) |
| Chunk | 4194304 (128 blocks), one mapping |
| Largest object in a block | 8192 |

* Blocks come from the chunks, and an address finds its block. <!-- test: blocks_a_chunk_is_block_aligned_zeroed_and_numbered_from_zero, blocks_an_address_finds_its_block_across_chunks -->
* An object larger than 8192 bytes has a mapping of its own, rounded to whole pages, and carries `FLAG_LARGE`. <!-- test: alloc_an_object_past_the_threshold_gets_its_own_mapping_flagged_large -->
* An object of a `Port`, `Task`, `Channel`, `Mutex` or `Once` handle carries `FLAG_FINALIZE`; no other kind does. <!-- test: alloc_a_finalizable_kind_carries_the_finalize_flag, alloc_a_task_channel_mutex_or_once_carries_the_finalize_flag_and_an_end_does_not, finalize_only_ports_are_finalizable, region_a_task_channel_mutex_and_once_are_finalized_like_a_port -->

### Allocation

* `alloc` takes zeroed memory from the region, writes the header, and writes the length word of a string or an array. <!-- test: alloc_returns_zeroed_memory_with_the_header_written, alloc_a_string_and_an_array_keep_their_length_after_the_header -->
* Objects from one region follow each other, and a full region takes the next hole. <!-- test: alloc_objects_from_one_region_follow_each_other, alloc_a_full_region_takes_a_new_hole -->
* A hole is a run of unmarked lines; memory reused after a collection is zeroed again. <!-- test: blocks_a_hole_search_skips_runs_that_are_too_short, sweep_a_reused_hole_is_zeroed, sweep_a_discarded_block_reads_zero_when_it_is_handed_out_again -->
* Blocks in use plus large objects never pass the heap limit; past it `alloc` reports `Full`. <!-- test: alloc_the_limit_stops_blocks_and_large_objects, alloc_a_large_object_counts_against_the_limit, alloc_a_count_too_large_to_hold_is_full_not_an_overflow -->
* `alloc_collecting` collects when the bytes handed out since the last collection pass the trigger, and when the heap is full. <!-- test: alloc_a_trigger_asks_for_a_collection_after_that_many_bytes, alloc_collecting_reuses_the_memory_of_dead_objects -->
* When a collection leaves no room, the allocation fails with the `MemoryLimit` fault. <!-- test: alloc_collecting_past_the_limit_with_everything_live_is_the_memory_limit_fault -->
* Only a string or an array takes a count. <!-- test: alloc_a_count_for_an_object_without_elements_is_a_bug -->

### Marking

* The collector starts from the roots the program reports: references, and frames by their descriptors. <!-- test: mark_a_chain_keeps_every_node_and_only_those, mark_a_frame_marks_its_references_and_its_dynamic_slots -->
* It follows exactly the slots of the type table, so a word that is not in a slot is never a reference. <!-- test: mark_does_not_scan_an_array_of_integers, mark_follows_an_enum_variant_chosen_by_its_tag, mark_follows_a_pair_payload_only_when_its_type_says_reference -->
* It scans the elements of an array, the repeating slot of a fixed array, and the buffer of a list and the captures of a closure. <!-- test: mark_scans_the_elements_of_an_array_of_references, mark_scans_a_fixed_array_with_its_repeating_slot, mark_a_list_keeps_its_buffer_and_a_closure_its_captures -->
* An object is marked once, however many references reach it, and a cycle ends. <!-- test: mark_a_cycle_and_a_shared_object_are_visited_once -->
* The marks are bits in the block's metadata, one for each word and one for each line, never in the object; marking sets the bit at the header and every line the object covers. <!-- test: mark_sets_the_object_bit_at_the_header_and_every_line_the_object_covers, mark_the_mark_bits_are_outside_the_object -->
* A large object is marked in a table of its own, and an unmarked one is unmapped by the collection. <!-- test: mark_large_objects_are_marked_and_scanned_and_dead_ones_are_freed, sweep_large_objects_are_given_back_by_the_collection_itself -->
* Each collection starts from clear marks. <!-- test: mark_a_second_collection_starts_from_clear_marks -->

### Sweeping

* A collection abandons every region, so the memory it reuses has one owner. <!-- test: sweep_abandons_a_region_so_the_swept_memory_has_one_owner -->
* Sweeping is lazy: a block is swept when an allocation needs it, and one with holes is reused before the next is swept. <!-- test: sweep_is_lazy_and_a_partly_live_block_is_recycled_before_the_next_is_swept -->
* A swept block with no marked line is free; one with unmarked lines is recycled in them; a new object never overlaps a live one. <!-- test: sweep_blocks_with_nothing_marked_return_to_free_when_swept, sweep_new_objects_never_overlap_live_ones, sweep_a_hole_is_a_run_of_unmarked_lines_so_a_larger_object_skips_short_ones -->
* Free blocks beyond the retained count are given back to the system. <!-- test: sweep_empty_blocks_past_the_retain_count_are_given_back -->
* A full heap has room again after a collection. <!-- test: sweep_a_full_heap_has_room_again_after_a_collection -->

### Stopping the world

* A thread that runs program code registers as a mutator and polls at its safepoints; a stop waits until every mutator is parked. <!-- test: parallel_a_stop_waits_until_every_mutator_is_at_a_safepoint -->
* A mutator in a blocking call counts as parked. <!-- test: parallel_a_blocked_mutator_does_not_hold_up_a_stop -->
* A mutator can start the stop, and two stops never overlap. <!-- test: parallel_a_mutator_can_start_the_stop_and_two_stops_never_overlap -->
* A thread that registers during a stop waits for it to end. <!-- test: parallel_a_thread_registering_during_a_stop_waits_for_it_to_end -->
* A stop can run a request once it is asked for, which makes each mutator reach its safepoint. <!-- test: parallel_a_stop_requesting_runs_its_request_while_a_stop_is_asked_for -->
* Marking runs on several workers that steal from each other, and finds what one worker finds. <!-- test: parallel_marking_finds_what_one_worker_finds, parallel_marking_a_wide_object_is_shared_between_workers, parallel_marking_a_single_long_chain_terminates_with_idle_workers -->
* Threads allocate from regions of their own without a lock, and mutators allocate while a stopped world collects. <!-- test: parallel_allocation_from_many_threads_gives_distinct_zeroed_objects, parallel_mutators_allocate_while_a_stopped_world_collects_in_parallel -->

### Finalizers

* The collection runs the finalizer once on each dying finalizable object, before its memory is reused, and a live one is kept. <!-- test: finalize_a_dead_port_runs_the_finalizer_once_with_its_words_intact, finalize_a_live_port_is_kept_until_it_dies, finalize_runs_before_the_memory_is_reused -->
* A dying object is forgotten when no finalizer is set. <!-- test: finalize_without_a_finalizer_still_forgets_the_dead_port -->

### Checking

* With `verify` on, each collection checks that every reachable object is marked and on marked lines, and that no reference is unaligned, outside the heap, into a free block, or to a word that is no header. <!-- test: torture_the_verifier_passes_after_a_sound_collection, torture_the_verifier_reports_a_reachable_object_without_a_mark, torture_the_verifier_reports_an_object_on_a_line_that_is_not_marked, torture_the_verifier_reports_references_that_dangle, torture_the_verifier_reports_a_reference_into_a_free_block -->
* With `torture` on, `alloc_collecting` collects before every allocation. <!-- test: torture_collects_at_every_allocation_and_keeps_a_rooted_list, torture_a_random_graph_of_every_shape_stays_sound, torture_a_full_collection_per_allocation_reuses_blocks_without_growing -->
* `Stats` reports the counters `--mem-stats` prints. <!-- test: torture_stats_report_what_the_heap_did -->

## The scheduler

* A task runs on a stack of its own and keeps it when it moves between workers. <!-- test: switch_a_task_resumes_where_it_left_off_with_its_locals_and_floats, queues_a_task_keeps_its_stack_when_it_moves_between_workers -->
* A stack is a slot of one reservation of equal slots: 16 MiB for a spawned task, 100,000 slots by default, and 256 MiB of virtual space for the main task. <!-- test: stacks_the_slots_are_equal_adjacent_and_aligned, stacks_a_spawned_task_has_16_mib_and_the_default_is_100000_slots, stacks_the_main_stack_is_256_mib_of_virtual_space -->
* A slot is committed when touched, and `madvise` gives its pages back when it is released; the next task to get it reads zero. <!-- test: stacks_memory_is_committed_when_touched_and_given_back_on_release, stacks_a_released_slot_is_the_next_one_handed_out_and_reads_zero, queues_thousands_of_parked_tasks_cost_only_their_touched_pages -->
* A task takes its slot when it first runs, so a task that has not run costs no stack. <!-- test: queues_a_task_takes_its_stack_when_it_first_runs -->
* A task that finds every slot in use waits for one, and an ended task's slot is reused. <!-- test: stacks_a_slab_hands_out_no_more_slots_than_it_has, queues_unstarted_tasks_wait_for_a_slot, queues_stack_slots_are_reused_as_tasks_end -->
* The pages below a parked task's stack pointer hold nothing live and can be given back. <!-- test: stacks_the_pages_below_a_stack_pointer_can_be_given_back -->
* A task parked longer than `cold_after` has its live stack copied to a heap buffer and its pages released, and it resumes with the same bytes at the same addresses. <!-- test: queues_a_cold_parked_task_gives_back_its_pages_and_resumes_intact, queues_a_cancelled_cold_task_resumes_and_ends -->
* A cold stack deeper than 64 KiB keeps its live pages and gives back the rest. <!-- test: queues_a_cold_task_with_a_deep_stack_keeps_its_live_pages_and_drops_the_rest -->
* A wake that lands while a stack is being copied out is not lost, and a park is handled once. <!-- test: queues_a_wake_during_compaction_is_not_lost -->
* A seeded run compacts only when asked. <!-- test: queues_a_seeded_run_compacts_only_when_asked_and_resumes_intact -->
* A slot has no guard page: `stack_limit` is 16 KiB above the bottom of the slot, and a function prologue compares the stack pointer to it. <!-- test: stacks_the_limit_leaves_the_red_zone_above_the_bottom, preempt_a_context_starts_with_its_stack_limit_set_and_no_poll_request -->
* A switch saves the callee-saved registers on the old stack and swaps the stack pointer; the first switch to a task enters its function with a frame pointer of zero. <!-- test: switch_the_entry_runs_on_the_new_stack_with_its_argument_and_comes_back, switch_the_frame_pointer_chain_of_a_new_task_starts_at_zero, switch_two_tasks_keep_separate_stacks_and_state -->
* A runtime or native call runs on the worker's OS stack and returns to the task stack; it must not park. <!-- test: switch_a_call_on_the_os_stack_leaves_the_task_stack_and_returns_to_it, queues_a_call_on_the_os_stack_runs_off_the_task_stack_and_returns -->

### Tasks

* A task is ready, running, parked or done; a park ends at an `unpark`, also one made before the park, and a timeout. <!-- test: queues_the_state_machine_goes_ready_running_parked_done, queues_a_parked_task_waits_for_an_unpark_and_runs_on, queues_an_unpark_before_the_park_is_not_lost -->
* Each worker has a queue it takes from in order, an idle worker steals from the others, and every spawned task runs once. <!-- test: queues_one_worker_runs_ready_tasks_in_order_and_a_yield_goes_to_the_back, queues_idle_workers_steal_ready_tasks, queues_every_spawned_task_runs_exactly_once -->
* A task woken by a task that parks next stays on that worker: it is queued there, wakes no sleeping worker, and is stolen only after 50 µs; one woken by a task that keeps running is stolen by an idle worker. <!-- test: queues_a_task_woken_by_a_task_that_parks_next_stays_on_its_worker, queues_a_task_woken_by_a_task_that_keeps_running_is_stolen_by_an_idle_worker -->
* A panic ends its task with the message at `join`, and the worker goes on; a panic in the main task is raised by `run`. <!-- test: queues_join_gives_the_panic_message_of_a_task_that_panicked_and_the_worker_lives_on, queues_a_panic_in_main_is_raised_by_run -->
* A parked task reports where its live stack is. <!-- test: queues_a_parked_task_reports_where_its_live_stack_is -->
* A thread outside the scheduler wakes a parked task. <!-- test: queues_a_parked_task_is_woken_from_a_thread_outside_the_scheduler -->

### Time

* Timers sit in a wheel and fire at their deadline rounded up to the next millisecond, in deadline order and in arming order for ties. <!-- test: timers_a_timer_fires_once_its_deadline_has_passed_and_not_before, timers_a_deadline_is_rounded_up_to_the_next_tick, timers_many_fire_in_deadline_order_and_ties_in_arming_order, timers_the_wheel_agrees_with_a_list_over_random_arming_cancelling_and_jumping -->
* A cancelled timer never fires. <!-- test: timers_a_cancelled_timer_never_fires_and_a_fired_one_cannot_be_cancelled, timers_a_timed_park_cancels_its_timer_when_woken_early -->
* A sleeping task is parked while others run, and sleepers wake in deadline order. <!-- test: timers_a_task_sleeps_on_the_real_clock_while_others_run, timers_a_sleeping_task_shows_as_parked, timers_sleepers_wake_in_deadline_order_whatever_the_spawn_order -->
* An `unpark` beats a timed park's timer. <!-- test: timers_a_timed_park_times_out_and_an_unpark_beats_the_timer -->
* A virtual clock moves only when no task can run, and then jumps to the next deadline. <!-- test: timers_a_virtual_clock_moves_only_when_told_and_never_back, timers_a_virtual_clock_does_not_move_while_tasks_can_run, timers_a_virtual_clock_makes_an_hour_pass_at_once, timers_virtual_sleepers_wake_in_order_at_their_times -->

### Descriptors

* A task parks until a descriptor is readable or writable, with an optional timeout. <!-- test: poller_a_task_parks_until_its_descriptor_is_readable, poller_a_writable_descriptor_does_not_park_for_long, poller_a_wait_with_a_timeout_ends_at_the_timeout, poller_a_readable_descriptor_wakes_its_waiter -->
* A descriptor has one waiter for each direction, and a second is refused; a closed peer wakes the waiters. <!-- test: poller_a_second_waiter_for_the_same_thing_is_refused, poller_a_second_task_for_the_same_thing_is_refused_and_a_closed_peer_wakes, poller_a_closed_peer_wakes_the_waiters, poller_a_reader_and_a_writer_of_one_descriptor_wake_apart -->
* A bad descriptor is an error, and a cancelled wait leaves the descriptor free. <!-- test: poller_a_bad_descriptor_is_an_error, poller_a_cancelled_task_leaves_its_descriptor_wait, poller_a_cancelled_waiter_does_not_wake -->
* Another thread's write wakes a waiting task. <!-- test: poller_a_thread_outside_the_scheduler_writes_and_the_task_wakes -->
* A parked port receiver is not a deadlock while a foreign source lives. <!-- test: poller_a_parked_port_receiver_is_not_a_deadlock_while_a_foreign_source_lives -->

### Preemption

* A timer sets the `poll` word of a task that has run a whole slice, and the task yields at its next poll point. <!-- test: preempt_the_ticker_sets_the_poll_flag_of_a_task_that_runs_a_whole_slice, preempt_a_poll_point_without_a_request_does_not_yield -->
* `cancel` marks the task, sets `poll` and poisons `stack_limit`, so the task stops at its next poll or prologue, and every poll after. <!-- test: preempt_cancel_marks_the_task_sets_its_poll_flag_and_poisons_its_stack_limit, preempt_a_cancelled_task_keeps_getting_cancelled_at_every_poll -->
* A stack trap is an overflow, a cancellation or spurious. <!-- test: preempt_a_stack_trap_tells_an_overflow_from_a_spurious_entry -->
* Cancelling a sleeping task ends its sleep. <!-- test: preempt_cancelling_a_sleeping_task_ends_its_sleep -->

### Seeded runs

* A seeded run has one worker and a virtual clock that starts at zero. <!-- test: seeded_a_run_uses_one_worker_and_a_clock_that_starts_at_zero -->
* The seed chooses the next ready task, and one seed gives one schedule trace; another seed gives another. <!-- test: seeded_the_next_ready_task_is_chosen_by_the_seed_not_in_spawn_order, seeded_one_seed_gives_one_schedule_every_time, seeded_different_seeds_give_different_schedules -->
* A seeded counter yields a task at its poll points. <!-- test: seeded_the_preemption_counter_yields_a_task_at_its_poll_points -->
* A task of a seeded run starts with its `poll` word set and every poll sets it again, so compiled code that never calls the scheduler is preempted at its polls. <!-- test: seeded_a_task_starts_with_its_poll_flag_set_and_every_poll_sets_it_again, aot_seeds_every_task_program_shows_one_result_under_every_seed -->
* A run gives its stacks back when it ends, though a task was abandoned in one. <!-- test: seeded_a_run_gives_its_stacks_back_when_it_ends_though_a_task_was_abandoned -->
* A trace starts with the main task and ends when it is done, and every step runs once. <!-- test: seeded_a_schedule_starts_with_the_main_task_and_ends_when_it_is_done, seeded_every_schedule_runs_every_step_once -->
* The virtual clock jumps forward between events. <!-- test: seeded_the_virtual_clock_jumps_forward_between_events -->

### Deadlock

* When every task is parked and no timer, descriptor wait or foreign source can wake one, `run` reports a deadlock listing each parked task and what it waits on, with one worker, many workers or a seed. <!-- test: deadlock_all_tasks_parked_with_nothing_to_wake_them_is_reported_with_what_each_waits_on, deadlock_the_report_prints_one_line_for_each_task, deadlock_is_found_with_one_worker_many_workers_and_a_seed -->
* A task that waits for a free stack is listed with the others. <!-- test: deadlock_tasks_waiting_for_a_stack_are_reported -->
* A pending timer, a live foreign source, or a main task that returns is not a deadlock. <!-- test: deadlock_a_pending_timer_is_not_one, deadlock_a_foreign_source_is_not_one_until_it_goes_away_without_waking_anyone, deadlock_a_main_that_returns_while_other_tasks_are_parked_is_not_one -->
* Tasks that wake each other are never reported, and a panicking task does not hide a deadlock. <!-- test: deadlock_ping_pong_between_tasks_never_looks_like_one, deadlock_a_panicking_task_does_not_hide_one -->

## Calls

* A value flattens to scalars. <!-- test: scalars_and_references_take_one_register, a_struct_flattens_to_its_leaves, a_fixed_array_flattens_to_the_leaves_of_its_elements -->

<!-- test: scalars_and_references_take_one_register, a_struct_flattens_to_its_leaves, a_fixed_array_flattens_to_the_leaves_of_its_elements, enums_optionals_and_pairs_travel_as_words, an_optional_object_is_one_reference, a_value_with_no_room_takes_no_register -->
| Value | Passed as |
|---|---|
| `Unit` | nothing |
| scalar | itself |
| reference, optional object | one `Ref` |
| struct, tuple | each leaf, in order |
| fixed array | each element's leaves |
| enum, tag-form optional, pair | `ceil(size / 8)` words of `I64` |

* An enum or a pair travels as `I64` words, never as a `Ref`, because the words that hold references are dynamic slots. <!-- test: enums_optionals_and_pairs_travel_as_words -->
* If the flat list is longer than `MAX_PASSED_SCALARS` (4), the value goes through a slot the caller provides, and the callee receives its address as an `I64`. <!-- test: past_the_limit_a_value_goes_through_memory, callconv_past_four_leaves_an_argument_goes_through_a_slot -->
* A result follows the same rule; one in memory is a slot whose address is the parameter after the context, and the function returns nothing. <!-- test: callconv_a_result_in_registers_returns_each_leaf, callconv_past_four_leaves_a_result_goes_through_a_slot_after_the_context -->
* The parameters of a call are the context, the result slot if any, the closure if the call is through one, then the arguments. <!-- test: callconv_arguments_follow_the_context_leaf_by_leaf, callconv_a_closure_call_passes_the_closure_after_the_result_slot -->

```text
Point { Float, Float }     F64 F64
Int?                       I64 I64
Int | String               I64 I64
Shape (size 24)            I64 I64 I64
5 Ints                     memory
```

## Faults

* A check (overflow, division by zero, shift range, bounds, `!` on a missing value, wrong type) is a compare and a branch to a cold block that calls `Entry::Fault` with a fault site. <!-- test: only_the_fault_entries_never_return, a_raw_id_finds_its_site -->
* A `FaultSite` is an index into the `FaultTable`; each entry is a `FaultKind` and the `Span` of the check. <!-- test: a_raw_id_finds_its_site -->
* The kinds a compiled program stops with, and their messages, are the table in [contract.md](../docs/reference/contract.md). <!-- test: faults_the_kinds_a_compiled_program_raises_are_these -->
* A kind is numbered by its place in `FaultKind::ALL`, and a new kind goes at the end. <!-- test: faults_a_kind_is_numbered_by_its_place_and_new_kinds_go_last -->
* A fault with a message from `std` is `Raised`; it calls `Entry::FaultMessage` with the message string and no fault site. <!-- test: a_fault_with_a_message_takes_the_message_string_and_no_site -->

## Runtime entries

<!-- table: abi-entries -->
| Entry | Symbol | Parameters after the context | Results |
|---|---|---|---|
| `Fault` | `mote_fault` | `I32` | none |
| `Alloc` | `mote_alloc` | `I32` | `Ref` |
| `AllocArray` | `mote_alloc_array` | `I32` `I64` | `Ref` |
| `FaultMessage` | `mote_fault_message` | `Ref` | none |
| `Poll` | `mote_poll` | none | none |
| `TaskSpawn` | `mote_task_spawn` | `I32` `I64` `Ref` `I64` | `Ref` |
| `TaskAwait` | `mote_task_await` | `Ref` | `I64` |
| `TaskCancel` | `mote_task_cancel` | `Ref` | none |
| `TaskReady` | `mote_task_ready` | `Ref` | `I64` |
| `TaskMessage` | `mote_task_message` | `Ref` | `Ref` |
| `ScopeBegin` | `mote_scope_begin` | none | `I64` |
| `ScopeEnd` | `mote_scope_end` | `I64` | none |
| `ChannelNew` | `mote_channel_new` | `I32` `I64` | `Ref` |
| `ChannelEnd` | `mote_channel_end` | `Ref` `I32` | `Ref` |
| `ChannelSend` | `mote_channel_send` | `Ref` `I64` `I64` | `I64` |
| `ChannelRecv` | `mote_channel_recv` | `Ref` `I64` `I64` | `I64` |
| `SenderClose` | `mote_sender_close` | `Ref` | none |
| `SenderClone` | `mote_sender_clone` | `Ref` | `Ref` |
| `ReceiverClose` | `mote_receiver_close` | `Ref` | none |
| `ReceiverClone` | `mote_receiver_clone` | `Ref` | `Ref` |
| `ReceiverClosed` | `mote_receiver_closed` | `Ref` | `I64` |
| `MutexNew` | `mote_mutex_new` | `I32` | `Ref` |
| `MutexTake` | `mote_mutex_take` | `Ref` `I64` | none |
| `MutexPut` | `mote_mutex_put` | `Ref` `I64` | none |
| `OnceNew` | `mote_once_new` | `I32` | `Ref` |
| `OnceBegin` | `mote_once_begin` | `Ref` | `I64` |
| `OnceFinish` | `mote_once_finish` | `Ref` `I64` | none |
| `OnceValue` | `mote_once_value` | `Ref` `I64` | none |
| `Sleep` | `mote_sleep` | `I64` | none |
| `TaskAwaitTimeout` | `mote_task_await_timeout` | `Ref` `I64` | `I64` |
| `ChannelRecvTimeout` | `mote_channel_recv_timeout` | `Ref` `I64` `I64` | `I64` |
| `PortNew` | `mote_port_new` | `Ref` `I32` `I64` | `Ref` |
| `PortRef` | `mote_port_ref` | `Ref` | `I64` |
| `Defer` | `mote_defer` | `Ref` `I64` | none |
| `Undefer` | `mote_undefer` | none | none |
| `ScopeEndError` | `mote_scope_end_error` | `I64` | `Ref` |
| `NativeBlocking` | `mote_native_blocking` | `I64` `I64` `I64` | none |
<!-- /table -->

* `Fault` takes the fault site, `Alloc` the type id, `AllocArray` the type id and the element count, `FaultMessage` the message, and `Poll` nothing. <!-- test: allocation_takes_the_context_and_a_type_id_and_returns_a_reference, a_fault_with_a_message_takes_the_message_string_and_no_site -->
* `Fault` and `FaultMessage` never return. <!-- test: only_the_fault_entries_never_return -->
* The symbols are unique and start with `mote_`. <!-- test: names_are_unique_and_prefixed -->
* New entries go at the end of `Entry::ALL`. <!-- test: names_are_unique_and_prefixed -->

### The runtime

* Each entry is a symbol of the runtime's static library under the name in the table. <!-- test: c_link_the_library_defines_a_symbol_for_every_entry_of_the_abi, entries_every_entry_of_the_abi_has_a_distinct_symbol -->
* A context is attached to a runtime before its code runs, and every entry finds the runtime through the context's `runtime` word. <!-- test: entries_alloc_returns_a_zeroed_object_with_its_header_and_moves_the_region, c_link_a_c_program_allocates_through_the_library -->
* `Alloc` and `AllocArray` take zeroed memory from the context's region and move it, write the header, and set the length word of a string or an array. <!-- test: entries_alloc_returns_a_zeroed_object_with_its_header_and_moves_the_region, entries_alloc_array_sets_the_length_word_of_a_string_or_an_array -->
* An allocation that finds no room stops the program with the `MemoryLimit` fault. <!-- test: entries_a_heap_without_room_stops_the_program_with_the_memory_limit, entries_a_negative_count_is_too_large_to_hold_and_stops_the_program -->
* A call that names a type that is not an object, or the wrong entry for the type, stops the program with a message. <!-- test: entries_a_site_that_is_not_in_the_table_and_a_wrong_type_stop_the_program_with_a_message -->
* `Fault` takes the kind, the message and the span of its site from the fault table and stores the site plus one in the context; a site that is not in the table stops the program with a message. <!-- test: entries_fault_reports_the_kind_message_site_and_span_and_marks_the_context, entries_a_site_that_is_not_in_the_table_and_a_wrong_type_stop_the_program_with_a_message -->
* `FaultMessage` stops the program with the text of its string as a `Raised` fault, empty when the reference is null, and leaves the context's fault site zero. <!-- test: entries_fault_message_reports_the_text_of_the_string_as_a_raised_fault, entries_a_message_that_is_null_is_empty_and_one_that_is_not_a_string_is_named -->
* A fault report holds the kind, the message, the site and span, and the return addresses of the call to the entry and of each caller up the frame-pointer chain, innermost first. <!-- test: entries_the_backtrace_starts_where_the_entry_was_called, entries_a_backtrace_lists_the_call_site_then_each_return_address_up_the_chain, c_link_a_fault_reaches_the_c_handler_with_the_frames_of_the_c_callers -->
* The chain ends at a null, misaligned, unreadable or non-rising frame record, and after 64 frames. <!-- test: entries_a_backtrace_ends_at_a_null_unaligned_unreadable_or_falling_pointer, entries_a_backtrace_keeps_at_most_the_frame_limit, entries_a_readable_address_gives_its_words_and_a_wild_one_gives_none -->
* The report goes to the host's fault hook, which ends the task or the process; without a hook it is printed to standard error and the process exits with code 1. <!-- test: entries_a_report_prints_its_message_and_one_line_for_each_frame, c_link_without_a_handler_the_report_goes_to_standard_error_and_the_process_exits_with_1 -->
* `Poll` clears the context's `poll` word and waits while a stop is in progress; the host counts the program's task as a mutator from before its code runs until it returns, and a thread that is not a mutator stops the world with `stop_world`, which asks each joined context to poll and waits for them. <!-- test: gc_polls_a_loop_with_no_call_is_stopped_before_it_ends, gc_polls_a_stopped_loop_goes_on_and_ends_with_its_result, gc_polls_a_stop_with_no_running_program_does_not_wait -->
* A collection asked for by an allocation, or by torture, runs at the next poll of the task that made the request, with every other mutator parked; its roots are the globals, the result and failure cells of the entry, and the frames of each parked stack, walked by the frame-pointer chain from the return address of the call into the runtime to the first return address with no descriptor. <!-- test: collect_a_walk_marks_the_slots_of_each_frame_in_the_map_and_stops_at_the_first_return_address_that_is_not, collect_a_global_root_is_marked_through_its_slots, gc_maps_a_program_that_holds_references_in_every_kind_of_frame_prints_what_it_prints_without_collection -->
* A program ends with the same output and fault when a collection follows every allocation as when none does, for every program and test block of the corpus, the standard tests and the reference. <!-- test: gc_gate_every_corpus_program_ends_as_it_does_when_every_allocation_is_followed_by_a_collection, gc_gate_every_block_of_std_tests_ends_as_it_does_when_every_allocation_is_followed_by_a_collection, gc_gate_every_runnable_block_of_the_reference_ends_as_it_does_when_every_allocation_is_followed_by_a_collection -->
* A program that allocates more than 100 times the heap limit and keeps a live structure runs to its end with the same output, and the heap never passes its limit. <!-- test: gc_gate_a_program_that_allocates_a_hundred_times_the_heap_limit_keeps_its_live_structure -->
* A collection empties the region of each mutator, so the next allocation takes a new one. <!-- test: collect_the_regions_of_the_mutators_are_emptied -->
* A C host makes a runtime from the byte forms of the type table and fault table with `mote_runtime_new`, which refuses tables that do not read, attaches a context with `mote_runtime_attach`, and sets the fault handler with `mote_runtime_on_fault`. <!-- test: c_link_a_message_from_the_program_is_a_raised_fault_without_a_site, entries_a_runtime_is_refused_for_tables_that_do_not_read -->

### Task runtime

* A spawned task is a scheduler task with a context of its own, counted as a mutator from before its body runs until it ends, and its handle is a `Task` object whose `runtime` word holds the task's number. <!-- test: tasks_a_spawned_task_runs_its_body_and_await_returns_done_with_the_result_in_the_handle, tasks_a_handle_is_pending_and_numbered_until_its_task_runs -->
* A body fills the payload of the handle, and the runtime sets the `state` word when the task ends. <!-- test: tasks_a_spawned_task_runs_its_body_and_await_returns_done_with_the_result_in_the_handle -->
* `await` parks until the task has ended and returns its state, and at once when it has. <!-- test: tasks_a_spawned_task_runs_its_body_and_await_returns_done_with_the_result_in_the_handle, tasks_awaiting_a_task_that_has_ended_returns_its_state_at_once -->
* `cancel` ends a task at its next poll as cancelled, and a task cancelled before it starts never runs its body. <!-- test: tasks_cancel_ends_a_task_that_polls_as_cancelled, tasks_a_task_cancelled_before_it_starts_does_not_run_its_body -->
* A `scope` waits for every task started in it. <!-- test: tasks_a_scope_waits_for_every_task_started_in_it -->
* A task that faults ends faulted and cancels the rest of its scope; the scope reports the fault once, unless the task was awaited. <!-- test: tasks_a_fault_ends_the_task_faulted_cancels_its_scope_and_the_scope_reports_it_once, tasks_a_fault_that_was_awaited_is_not_reported_by_its_scope -->
* A task started in a cancelled scope ends cancelled without running. <!-- test: tasks_a_task_started_in_a_cancelled_scope_ends_cancelled_without_running -->
* A fault of a task that no scope holds and nobody awaited is kept for the program's end. <!-- test: tasks_a_fault_nobody_awaited_and_no_scope_holds_is_reported_until_it_is_awaited, tasks_a_task_that_faulted_unawaited_keeps_its_record_after_its_handle_died -->
* The handle and closure of a task that has not ended are roots. <!-- test: tasks_a_pending_task_keeps_its_handle_and_closure_alive_and_an_ended_one_does_not -->
* The record of a task is freed when its handle has died and the task has ended, unless a scope that is open holds it or its fault is still to be reported; its place is used by a later task. <!-- test: tasks_the_record_of_a_task_is_freed_with_its_handle_and_its_place_is_used_again, tasks_a_task_that_faulted_unawaited_keeps_its_record_after_its_handle_died -->
* A view of a task shares the task's record, so its death frees nothing. <!-- test: tasks_a_view_of_a_task_shares_its_record_and_frees_nothing -->
* A scope is freed when it ends, and its place is used by a later scope; a fault of its tasks that it did not report is not reported by the program either. <!-- test: tasks_a_scope_is_freed_when_it_ends_and_its_place_is_used_again, tasks_a_fault_a_scope_did_not_report_is_not_reported_by_the_program_either -->
* A task that stops waiting for another, by a time that ran out or by being cancelled, leaves the waiters of that task. <!-- test: tasks_a_wait_that_gave_up_or_was_cancelled_leaves_the_waiters_of_the_task -->
* A lock word names a task by its place and generation, so the name of a freed task is ended also while another task has the place. <!-- test: tasks_the_name_of_a_task_that_was_freed_is_ended_while_a_task_in_its_place_runs, locks_a_mutex_left_locked_by_a_freed_task_is_refused_while_another_task_has_its_place -->
* A task blocked in the runtime does not hold up a stop, and its frames are scanned from where it blocked, also when a compactor holds its stack. <!-- test: tasks_the_frames_of_a_task_parked_in_the_runtime_are_roots, tasks_the_frames_of_a_parked_task_are_roots_when_a_compactor_holds_its_stack -->
* `TaskSpawn` takes the type of the task, the address of its function, the closure and the scope token; `TaskAwait`, `TaskCancel`, `TaskReady` and `TaskMessage` take the handle; `ScopeEnd` takes the token. <!-- test: the_task_entries_take_the_handle_and_the_closure_by_reference, a_scope_is_opened_for_a_token_and_closed_with_it, tasks_the_entries_start_wait_for_and_ask_a_task_through_their_symbols -->
* `TaskSpawn` has five parameters after the context's own, so its stub passes the frame pointer in `r9` and the return address as the seventh argument, on the stack. <!-- test: tasks_the_return_address_reaches_the_spawn_entry_on_the_stack -->
* A fault of a task other than the program's first ends that task faulted and does not reach the host's hook; `TaskMessage` gives the message of the fault, or `the task was cancelled`. <!-- test: tasks_a_fault_in_a_task_ends_the_task_and_not_the_program_and_await_reads_its_message, tasks_the_message_of_a_task_that_was_cancelled_says_so -->
* A cancelled task ends at its next poll, or at the stack limit check of its next function, which cancellation poisons; the same check in a task that was not cancelled is the `StackDepth` fault. <!-- test: tasks_cancel_ends_a_task_that_polls_as_cancelled, tasks_a_cancelled_task_that_meets_the_stack_limit_check_ends_cancelled_and_another_ends_faulted -->
* A task that waits in `TaskAwait` or `ScopeEnd` and is cancelled ends cancelled, and cancels the tasks of its scope. <!-- test: tasks_a_task_cancelled_while_it_waits_for_its_scope_ends_cancelled_and_cancels_the_scope -->
* A task that ends cancelled first undoes its `Defer` and `Scope` records, innermost first: a close function is called, and a scope has its tasks cancelled and waited for. <!-- test: differential_cancel_a_cancelled_task_closes_its_resources_innermost_first_and_a_fault_closes_none, differential_cancel_a_cancelled_task_cancels_and_waits_for_the_tasks_of_its_scope -->
* A task that faults closes none of its resources. <!-- test: differential_cancel_a_cancelled_task_closes_its_resources_innermost_first_and_a_fault_closes_none -->
* A scope that has ended is forgotten, so a cancellation after it does not wait for it again; a `with` that has ended is not closed again. <!-- test: differential_cancel_a_cancelled_task_closes_its_resources_innermost_first_and_a_fault_closes_none -->
* While it undoes its records a task is not cancelled again, so a close function can wait. <!-- test: preempt_a_cleaning_task_is_not_cancelled_again_and_has_its_stack_limit_back, differential_cancel_the_compiled_code_shows_the_recorded_results -->
* `Defer` takes the box that holds the resource and the address of the close thunk; `Undefer` takes nothing; `ScopeEndError` takes the token and answers a `String` of the fault message, or null when the scope was clean. <!-- test: a_with_is_recorded_with_its_box_and_closer_and_forgotten_at_its_end -->
* `NativeBlocking` takes the address of the thunk of a `@blocking` external function, its argument block and the block's length in words, parks the calling task while a helper thread runs the thunk over a copy of the block, and copies the block back when it returns, so a task waiting in an external library holds no worker and does not delay a collection. <!-- test: core_net_the_tests_of_the_library_pass -->

### Task gate

* Every program of the groups `cancel`, `channels`, `deadlock`, `ports`, `streams`, `sync`, `tasks` and `timed`, and every `test` block of `std/tests/sync.mote` and `std/tests/tasks.mote`, shows one result under 1000 seeds, and it is the recorded one. <!-- test: aot_seeds_every_task_program_shows_one_result_under_every_seed, aot_seeds_every_task_block_of_std_tests_shows_one_result_under_every_seed -->
* No program is schedule dependent, so none is left out of the comparison. <!-- test: aot_seeds_every_task_program_shows_one_result_under_every_seed -->
* A program whose result depends on the schedule shows more than one result under the seeds, and one seed repeats its result. <!-- test: aot_seeds_a_racing_program_shows_both_orders_and_a_seed_repeats -->
* A program in which every task waits ends in a `Deadlock` fault with the scheduler's report and no span, on both backends. <!-- test: differential_deadlock_the_compiled_code_shows_the_recorded_results, differential_deadlock_the_compiled_code_shows_the_recorded_results -->
* A seeded run feeds the ports from a task of the scheduler at the times of the feeds on its virtual clock, and an open port is no source outside the scheduler, so the clock moves while one is open. <!-- test: aot_seeds_every_task_program_shows_one_result_under_every_seed -->
* A feed to a port that is not open at its time is dropped. <!-- test: aot_seeds_every_task_program_shows_one_result_under_every_seed -->

### Channel runtime

* `ChannelNew` allocates a channel and its buffer, and `ChannelEnd` a `Sender` or a `Receiver` of it; a negative capacity stops the program with `a channel buffer is not negative`. <!-- test: entries_channels_make_ends_and_pass_a_message_through_their_symbols, entries_a_channel_call_that_fails_stops_the_program_with_a_raised_fault, channels_a_rendezvous_channel_keeps_one_place_and_a_negative_capacity_is_refused -->
* `ChannelSend` and `ChannelRecv` take the end and the address of a message, which they copy to or from the buffer; the last parameter is 1 to wait and 0 not to. <!-- test: the_channel_entries_pass_the_end_first_and_an_address_for_the_message, entries_channels_make_ends_and_pass_a_message_through_their_symbols -->
* Messages come out in the order they went in, also after the ring wrapped. <!-- test: channels_a_buffered_channel_gives_its_messages_in_the_order_they_came, channels_a_ring_that_wrapped_keeps_the_order, differential_channels_the_compiled_code_shows_the_recorded_results -->
* A send waits while the buffer is full; on a rendezvous channel it waits until a receiver has taken the message. <!-- test: channels_a_rendezvous_send_waits_until_a_receiver_takes_the_message -->
* Tasks that wait to receive take messages oldest first. <!-- test: channels_waiting_receivers_take_the_messages_oldest_first -->
* `ChannelSend` answers 0 and `ChannelRecv` answers 2 when a wait ended without the call being done, and the code asks again. <!-- test: channels_a_rendezvous_send_waits_until_a_receiver_takes_the_message, channels_a_task_cancelled_while_it_waits_leaves_the_queue -->
* A send that does not wait answers whether there was room, and a receive that does not wait answers whether there was a message; a send without waiting on a rendezvous channel succeeds only while a receiver waits. <!-- test: channels_try_send_and_try_recv_never_wait, channels_try_send_on_a_rendezvous_channel_needs_a_receiver_that_waits -->
* A send on a closed `Sender` or after the receiving side closed, and a clone of a closed `Sender`, stop the program with `send on a closed Sender`, `send on a closed Receiver` and `clone of a closed Sender`. <!-- test: channels_a_sender_that_is_closed_or_whose_receiver_is_closed_cannot_send, entries_a_channel_call_that_fails_stops_the_program_with_a_raised_fault -->
* A channel is closed when its last `Sender` closes or its receiving side closes; a receive on a closed channel gives the messages left and then none. <!-- test: channels_a_closed_channel_gives_none_once_it_is_drained -->
* Closing the receiving side drops the messages left and wakes the tasks that wait. <!-- test: channels_closing_the_receiver_drops_the_messages_and_the_channel_is_closed -->
* A `Sender` belongs to the task that made it, that holds it in the closure it was spawned with, or that took it in a message; it closes when that task ends, and in a message it belongs to nobody. <!-- test: channels_a_sender_given_to_a_task_closes_when_that_task_ends, channels_a_sender_sent_in_a_message_belongs_to_the_task_that_takes_it -->
* The open `Sender` ends of tasks that have not ended are roots. <!-- test: channels_the_ends_a_task_owns_are_roots_until_it_ends -->
* A task cancelled while it waits on a channel leaves the queue it waited in and ends cancelled. <!-- test: channels_a_task_cancelled_while_it_waits_leaves_the_queue -->
* A compiled `channel_new`, `send`, `recv`, `try_send`, `try_recv`, `close`, `clone` and `is_closed` give the recorded results for messages of numbers, `Bool`, strings, structs, tuples, enums and lists, and for `Sender` ends inside a message. <!-- test: differential_channels_the_compiled_code_shows_the_recorded_results, differential_channels_the_compiled_code_shows_the_recorded_results, gc_gate_every_corpus_program_ends_as_it_does_when_every_allocation_is_followed_by_a_collection -->
* The checker refuses a channel of optional messages, so a `null` from `recv` is a closed channel. <!-- test: differential_channels_a_channel_of_optional_messages -->
* The waits of a channel are freed when the channel dies and the place is used by a later channel; a `Sender` of a task that has not ended keeps its channel alive. <!-- test: channels_the_waits_of_a_channel_that_died_are_freed_and_their_place_is_used_again, channels_a_channel_whose_sender_is_open_stays_alive_with_its_waits -->

### Timed runtime

* `Sleep` takes milliseconds of the scheduler's clock and returns after them; a time of zero or less returns at once. <!-- test: tasks_sleep_moves_the_scheduler_clock_by_the_milliseconds_and_a_time_of_zero_does_not, the_timed_entries_take_the_time_in_milliseconds_last -->
* `TaskAwaitTimeout` answers the state of the task, or pending when the time ran out; a time of zero asks once. <!-- test: tasks_a_timed_await_gives_up_at_its_time_and_a_later_one_gets_the_state -->
* `ChannelRecvTimeout` answers 1 with a message, and 0 when the channel is closed and empty or the time ran out; a message that is there is taken whatever the time. <!-- test: channels_a_timed_receive_gives_up_at_its_time_and_takes_a_message_that_comes_before_it, channels_a_timed_receive_with_no_time_takes_only_what_is_there_and_a_closed_channel_ends_it, entries_the_timed_calls_give_up_through_their_symbols -->
* A task cancelled in a `Sleep` or a timed await ends cancelled. <!-- test: tasks_a_task_cancelled_in_a_timed_await_or_a_sleep_ends_cancelled -->
* A compiled `sleep`, `await_timeout`, `recv_timeout` and `with_timeout` give the recorded results; a timed await that ran out is `null` in the optional of the result. <!-- test: differential_timed_the_compiled_code_shows_the_recorded_results, differential_timed_the_compiled_code_shows_the_recorded_results, gc_gate_every_corpus_program_ends_as_it_does_when_every_allocation_is_followed_by_a_collection -->

### Lock runtime

* `MutexNew` and `OnceNew` take the type and return the handle; `MutexTake`, `MutexPut`, `OnceFinish` and `OnceValue` take the handle and the address of the value; `OnceBegin` takes the handle and answers 1 when the caller is to run the initializer. <!-- test: the_lock_entries_pass_the_handle_first_and_an_address_for_the_value, entries_locks_make_a_mutex_and_a_once_and_pass_their_values_through_their_symbols -->
* The list of waiting tasks of a mutex or a once is freed when it dies and the place is used by a later one. <!-- test: locks_the_list_of_a_mutex_or_once_that_died_is_freed_and_its_place_is_used_again -->
* A mutex holds the name of the task that locked it in `owner`, 0 when it is free; taking copies its value to the address, and putting copies a value in and frees it. <!-- test: locks_a_mutex_gives_its_value_to_the_task_that_takes_it_and_keeps_the_value_put_back -->
* A task that finds the mutex held waits, and the tasks that wait take it oldest first. <!-- test: locks_a_task_that_waits_for_a_mutex_takes_it_when_the_holder_puts_it_back, differential_sync_the_compiled_code_shows_the_recorded_results -->
* A take by the task that holds the mutex stops the program with `Mutex is already locked by this task`, a put by another with `Mutex is not locked by this task`, and a take of a mutex whose holder ended with `Mutex was left locked by a task that stopped`. <!-- test: locks_taking_a_mutex_twice_or_putting_one_that_is_not_held_is_refused, locks_a_mutex_left_held_by_a_task_that_ended_is_refused, entries_a_lock_call_that_fails_stops_the_program_with_a_raised_fault -->
* A `Once` is empty, running or done; the first `OnceBegin` makes it running for its caller, a later one waits until it is done, and one by the runner stops the program with `Once is used inside its own initializer`. <!-- test: locks_a_once_runs_its_initializer_once_and_then_gives_the_value, locks_a_task_that_asks_a_once_that_is_running_waits_for_the_value -->
* A `Once` whose runner ended without a value is run again by the next caller, and `OnceValue` of an empty one stops the program with `Once has no value`. <!-- test: locks_a_once_whose_runner_ended_without_a_value_is_run_again, entries_a_lock_call_that_fails_stops_the_program_with_a_raised_fault -->
* A task cancelled while it waits for a mutex leaves the queue and ends cancelled. <!-- test: locks_a_task_cancelled_while_it_waits_for_a_mutex_leaves_the_queue -->
* A lock entry on a handle of another kind, or a new handle of a type of another kind, stops the program with a message. <!-- test: entries_a_lock_call_that_fails_stops_the_program_with_a_raised_fault, locks_a_handle_of_the_wrong_type_is_not_made -->
* An atomic integer or flag is the instruction on the word at 8 of its handle (load, store, exchange, compare-exchange), a flag as 0 or 1; `fetch_add` and `fetch_sub` are a compare-exchange loop that stops with `IntegerOverflow` before a write when the result leaves `Int`. <!-- test: differential_sync_the_compiled_code_shows_the_recorded_results, differential_sync_the_compiled_code_shows_the_recorded_results -->
* A compiled `Mutex`, `Once` and atomic give the recorded results, with the value copied in and out of the handle. <!-- test: differential_sync_the_compiled_code_shows_the_recorded_results, differential_sync_the_compiled_code_shows_the_recorded_results, gc_gate_every_corpus_program_ends_as_it_does_when_every_allocation_is_followed_by_a_collection -->

### Port runtime

* `PortNew` takes the channel, the `Port` type and the queue size, and returns the handle; `PortRef` takes the handle and returns the `MotePort *` a library is given. <!-- test: a_port_is_opened_on_a_channel_and_referenced_by_address, entries_a_port_opens_on_a_channel_and_hands_a_native_reference_through_its_symbols -->
* A port of a queue size above 0 holds that many messages and drops a push that finds it full; a size of 0 or less keeps only the newest message. <!-- test: ports_a_queue_keeps_up_to_its_size_and_a_latest_port_keeps_the_newest, differential_ports_a_push_is_queued_or_refused_when_full_and_a_released_port_closes, differential_ports_a_latest_port_keeps_the_newest_push -->
* A push copies its bytes into a new `[UInt8]` for the receiver, and the receiver takes the messages in the order they were pushed. <!-- test: ports_a_receiver_takes_the_messages_in_order_as_new_byte_arrays, differential_ports_a_library_thread_pushes_to_the_port_it_was_given -->
* A push after the port was closed or its `Receiver` closed answers 2 and takes nothing. <!-- test: ports_a_closed_port_refuses_pushes_and_a_closed_receiver_does_too -->
* A receiver that waits on a port is no deadlock while the port is open, and a push from a thread outside the scheduler wakes it; a timed receive gives up when the port stays quiet. <!-- test: ports_a_receiver_waits_for_a_thread_outside_the_scheduler_and_that_is_no_deadlock, ports_a_timed_receive_gives_up_on_a_port_that_stays_quiet -->
* A `MotePort *` holds the port alive after its handle is collected, and `port_close` releases it and closes the port; the collected handle releases its own reference. <!-- test: ports_a_native_reference_outlives_the_handle_and_the_handle_releases_its_own, differential_ports_a_push_is_queued_or_refused_when_full_and_a_released_port_closes -->
* A task cancelled while it waits for a message ends cancelled. <!-- test: ports_a_task_cancelled_while_it_waits_for_a_message_ends_cancelled -->
* Tasks that wait on one port take the messages oldest first. <!-- test: ports_waiting_receivers_of_one_port_take_the_messages_oldest_first -->
* The host feeds or closes the port the program opened `n`th, once the program has opened it; a port nobody holds any more is closed, and the runtime keeps no record of it. <!-- test: ports_the_runtime_feeds_and_closes_the_port_opened_nth, ports_the_list_of_ports_drops_those_nobody_holds_and_a_dropped_one_is_closed -->
* A port whose channel died is closed to the library. <!-- test: ports_a_port_whose_channel_died_is_closed_to_the_library -->
* The runtime is the `MoteHost` a library receives: its `port_push` and `port_close` are `host_port_push` and `host_port_close`. <!-- test: differential_ports_a_library_thread_pushes_to_the_port_it_was_given -->
* A compiled `Port.open` and a `Receiver` of a port give the recorded results under the pushes of `// feed` and `// close` lines of a corpus program. <!-- test: differential_ports_the_compiled_code_shows_the_recorded_results, differential_ports_the_compiled_code_shows_the_recorded_results, differential_ports_a_fed_program_prints_what_the_port_delivered, gc_gate_every_corpus_program_ends_as_it_does_when_every_allocation_is_followed_by_a_collection -->

## Frame descriptors

* A frame descriptor is the record for one safepoint: the frame size, the offsets from the frame base of the live `Ref` values, and the live dynamic slots (`Enum`, `Pair`, `Repeat`) by offset. <!-- test: frames_a_descriptor_keeps_the_references_and_the_dynamic_slots_apart -->
* The frame size is a whole number of words, each slot lies on a word inside the frame, and no two slots share an offset. <!-- test: frames_a_slot_off_a_word_outside_the_frame_or_twice_is_refused -->
* The slots are sorted by offset, so one descriptor has one byte form. <!-- test: frames_a_descriptor_reads_back_equal_and_gives_one_byte_string -->
* The byte form is the frame size as a `u32`, the number of references and each offset as a `u32`, then the number of dynamic slots and each slot. <!-- test: frames_the_byte_form_is_the_size_then_the_references_then_the_dynamic_slots -->
* Bytes that are cut short, out of order or followed by more are refused. <!-- test: frames_bytes_out_of_order_or_cut_short_are_refused -->

## Byte forms

* Every number is little-endian, a text is its length as a `u32` then its UTF-8 bytes, and a slot is a tag byte then its fields. <!-- test: bytes_values_are_little_endian, bytes_every_slot_reads_back -->
* A reader refuses a short input, an unknown tag, a bad text, a count the input cannot hold and bytes left over. <!-- test: bytes_a_short_input_is_truncated_not_a_panic, bytes_a_false_count_cannot_ask_for_more_than_the_input_holds, bytes_an_unknown_slot_tag_and_a_bad_text_are_errors, bytes_the_magic_and_the_end_are_checked -->

### The type table

* The type table starts with the magic `MOTETYPE` and the version `1` as a `u32`, then the number of types after `NONE`, then each type in id order. <!-- test: table_bytes_start_with_the_magic_and_the_version -->
* A type is its name, a kind tag and the types and numbers its kind names; the layout, offsets, slots and passing are computed again when the table is read. <!-- test: table_bytes_one_type_is_its_name_its_kind_tag_and_its_members, table_bytes_the_names_and_the_slots_read_back_for_every_type -->
* A table with every kind reads back equal, and one table gives one byte string. <!-- test: table_bytes_every_kind_reads_back_to_an_equal_table, table_bytes_one_table_gives_one_byte_string -->
* A table with a reserved type that is not defined is not written. <!-- test: table_bytes_a_type_that_is_reserved_and_not_defined_is_not_written -->
* A reader refuses a wrong magic or version, a table cut at any byte, a type that names one that is missing or a value type that names a later one, a handle with the wrong number of payload types, and a type over the size limit. <!-- test: table_bytes_a_wrong_magic_or_version_is_refused, table_bytes_every_cut_of_a_table_is_refused_and_none_panics, table_bytes_a_type_that_names_a_missing_or_later_value_is_refused, table_bytes_a_handle_with_the_wrong_payload_count_and_a_huge_array_are_refused -->

### The fault table

* The fault table starts with the magic `MOTEFAUL` and the version `1`, then the number of sites, then each site as its kind number and the two ends of its span. <!-- test: faults_a_table_is_the_magic_the_version_the_count_and_a_nine_byte_record_per_site -->
* A table reads back equal and gives one byte string. <!-- test: faults_a_table_reads_back_equal_and_gives_one_byte_string -->
* A reader refuses a wrong magic, an unknown kind, a span that ends before it starts, a cut table and bytes left over. <!-- test: faults_a_bad_table_is_refused -->

### The code table

* The code table starts with the magic `MOTECODE` and the version `1`, then the functions, then the roots. <!-- test: code_a_table_is_the_magic_the_version_the_functions_then_the_roots -->
* A function is its call sites, each the offset of the return address from the function's start, ascending, then the length and bytes of the frame descriptor at the call; a root is the slots of a static place that holds references. <!-- test: code_a_table_is_the_magic_the_version_the_functions_then_the_roots -->
* The addresses are not in the bytes: they are an array of words, one for each function and then one for each root, that the linker fills. <!-- test: code_a_table_reads_back_equal_and_gives_one_byte_string, object_the_code_table_places_each_function_by_a_relocation_and_keeps_its_sites_inside_the_function -->
* A table reads back equal and gives one byte string. <!-- test: code_a_table_reads_back_equal_and_gives_one_byte_string -->
* A reader refuses a wrong magic or version, a cut table, bytes left over and sites out of order. <!-- test: code_a_bad_table_is_refused -->

### The meta table

* The meta table starts with the magic `MOTEMETA` and the version `1`, then the string type, the optional boxed type (0 for none), how each type is shown by `print` and each described type record. <!-- test: meta_a_table_is_the_magic_the_version_the_two_types_then_the_ways_to_show_and_the_described -->
* A table reads back equal and gives one byte string. <!-- test: meta_a_table_reads_back_equal_and_gives_one_byte_string, meta_every_plain_way_to_show_reads_back, meta_every_kind_has_a_tag_that_reads_back -->
* A reader refuses a wrong magic or version, a cut table, bytes left over, an unknown tag and a described type that names one that is missing. <!-- test: meta_a_bad_table_is_refused -->

### The link table

* The link table starts with the magic `MOTELINK` and the version `1`, then the library names, the file to open for each library (none, or one for each), then each function as its library, symbol, parameters, result, `blocking` flag and span. <!-- test: link_a_table_is_the_magic_the_version_the_libraries_the_files_then_the_functions -->
* A table reads back equal and gives one byte string. <!-- test: link_a_table_reads_back_equal_and_gives_one_byte_string -->
* A reader refuses a wrong magic or version, a cut table, bytes left over, a library with no file, a function that names a missing library and a signature that puts a type where it is not allowed. <!-- test: link_a_bad_table_is_refused -->

## Object files

* `compile_object` writes a relocatable x86-64 ELF file of position-independent code. <!-- test: object_is_a_relocatable_x86_64_elf_file_that_defines_the_run_function_and_the_tables, object_the_code_in_the_file_is_position_independent -->
* The file defines `mote_run`, which runs start-up and then the entry, and `mote_tables`; an entry that returns a `Result` also defines `mote_failure`, the two words of the run state. <!-- test: object_is_a_relocatable_x86_64_elf_file_that_defines_the_run_function_and_the_tables, object_an_entry_that_returns_an_err_defines_the_failure_cell -->
* The symbols the file leaves undefined are the entries of the runtime and the helpers of compiled code. <!-- test: object_the_symbols_it_does_not_define_are_the_runtime_entries_and_helpers -->
* `mote_tables` is nineteen words: the address and the byte length of the type table, of the fault table and of the code table, then the address of the code table's array of address words and its number of words, then the address and byte length of the meta table and of the link table, then the address of the failure cell and of the native slots, each 0 when the program has none, then the address of the array of init functions of the libraries linked in and its number of words, then the address and byte length of the debug table, then a flags word (`1` when single-threaded); new words go at the end. <!-- test: object_the_directory_leads_to_a_type_table_that_reads_back, object_the_directory_leads_to_a_fault_table_with_a_site_for_each_check, object_the_code_table_places_each_function_by_a_relocation_and_keeps_its_sites_inside_the_function, object_tables_the_directory_points_at_the_failure_cell_only_when_the_entry_returns_a_result -->
* The meta table of the file reads back equal to the program's: how each type is shown and its described entries. <!-- test: object_tables_the_meta_table_reads_back_equal_to_the_programs -->
* The link table of the file lists the libraries and functions of the program with the file to open for each library, and the function slots are a word for each function. <!-- test: object_tables_the_link_table_carries_the_libraries_functions_and_files -->
* The type table of the file reads back equal to the program's. <!-- test: object_the_directory_leads_to_a_type_table_that_reads_back -->
* The fault table has a site for each check. <!-- test: object_the_directory_leads_to_a_fault_table_with_a_site_for_each_check -->
* The call sites of the file have the frames the JIT gives the same program. <!-- test: object_the_frames_are_the_ones_the_jit_gives -->
* A global that holds references is a root of the code table, placed by a relocation to its cell. <!-- test: object_a_global_that_holds_a_reference_is_a_root_placed_by_a_relocation -->
* The debug table starts with the magic `MOTEDBUG`, then the file names, the places (file, line, column), the functions as name, code size and rows of code offset and place, and a place for each site of the fault table; a table reads back equal. <!-- test: debug_a_table_reads_back_equal_and_gives_one_byte_string, debug_a_place_is_found_by_its_index_and_a_fault_site, debug_a_bad_table_is_refused -->
* The debug table of the file names each function of the code table, places its code by offset and places each check. <!-- test: object_tables_the_debug_table_names_each_function_and_places_its_code_and_its_checks -->
* The file also holds DWARF line tables: a sequence for each function, started by a relocation to the function's symbol, with the rows of the debug table. <!-- test: object_the_line_table_has_a_sequence_for_each_function_placed_by_a_relocation_with_the_rows_of_the_debug_table -->
* The runtime finds the function and the place of a return address through the code map, and a fault lists them for each caller, innermost first, as `  in <function> (<file>:<line>)`. <!-- test: aot_backtrace_a_fault_names_the_check_and_each_function_with_its_line, run_tests_the_compiled_backend_lists_each_function_of_the_stack_with_its_line -->
* A JIT run with `MOTE_PERF_MAP=1` writes `/tmp/perf-<pid>.map`, a line of start, size and name for each function, for `perf`. <!-- test: jit_the_perf_map_lists_each_function_with_its_start_size_and_name -->

## Executables

* `libmote_runtime.a` holds the runtime, the helpers of compiled code and the library loader; linked with the object it leaves nothing undefined, and its `main` starts from `mote_tables`. <!-- test: executable_a_program_prints_and_returns_with_code_0 -->
* The runtime builds its code map from the code table and its address words, opens the libraries of the link table into the function slots, and runs `mote_run`. <!-- test: executable_a_program_prints_and_returns_with_code_0 -->

## External library calls

* An `extern` fn is a C-ABI function declared by its Mote signature; `CType` is what a parameter or result may be, and `NativeSig { params, ret, blocking }` is the signature. <!-- test: a_signature_collects_params_blocking_and_result, a_signature_checks_its_directions_and_flattens_arrays -->

<!-- table: abi-ctypes -->
| `CType` | Machine values | Direction |
|---|---|---|
| `Void` | none | result |
| `I8` | `I8` | both |
| `I16` | `I16` | both |
| `I32` | `I32` | both |
| `I64` | `I64` | both |
| `U8` | `I8` | both |
| `U16` | `I16` | both |
| `U32` | `I32` | both |
| `U64` | `I64` | both |
| `F32` | `F32` | both |
| `F64` | `F64` | both |
| `StringIn` | `I64` | parameter |
| `StringOut` | `I64` | result |
| `Array` | `I64` `I64` | parameter |
| `Pointer` | `I64` | both |
| `NullablePointer` | `I64` | both |
| `PortEnd` | `I64` | parameter |
| `FixedArray` | `I64` `I64` | parameter |
<!-- /table -->

* A signed integer widens by sign and an unsigned one by zero. <!-- test: only_signed_integers_widen_by_sign -->
* A string goes in as a pointer and comes out as a pointer, an `Array` is a pointer to the elements then the length, and an array is a parameter only. <!-- test: strings_go_in_by_copy_and_come_out_by_copy, arrays_of_scalars_are_parameters_only, each_type_lowers_to_its_machine_values -->
* A pointer is an `I64`, never a `Ref`, so the collector ignores it. <!-- test: pointers_are_one_word_whether_or_not_they_may_be_null -->
* `Bool` is `U8`. <!-- test: numbers_bool_and_unit_map_to_their_c_types -->
* `Pointer<T>` is a universal builtin with one field, `address: UInt64`; `Pointer<T>?` is a `NullablePointer`. <!-- test: a_pointer_is_a_pointer_and_an_optional_pointer_may_be_null, differential_natives_a_pointer_is_an_address_and_a_nullable_one_may_be_null -->
* A `PortEnd` crosses as a parameter only. <!-- test: a_port_end_crosses_as_a_parameter_only, a_port_end_is_a_one_word_parameter_only -->
* A parameter is a `CParam { ty, by_ref }`; a `var` parameter is `by_ref`, one `I64` pointer, and is allowed for numbers, `Bool` and both pointer types only. <!-- test: a_by_reference_parameter_is_one_pointer_and_only_for_plain_values, a_var_parameter_is_passed_by_pointer, a_var_string_or_array_has_no_pointer_form -->
* `LinkTable` lists each library once and each function as a `LinkedFn { library, symbol, sig, span }`, and only the external functions the program calls are in it. <!-- test: a_library_is_listed_once, only_natives_the_program_calls_are_linked -->
* A narrow integer is passed widened by sign or by zero as its type says; a `Bool` goes in as a `U8` and a `Bool` result is true when it is not 0. <!-- test: differential_natives_numbers_cross_with_their_width_and_sign -->
* A `String` parameter is a NUL-terminated copy made for the call and freed after it; a `String` result is copied into a new string and the C pointer is not freed, and a NULL one is the empty string. <!-- test: differential_natives_strings_and_arrays_cross_by_copy_and_by_pointer -->
* An `Array` parameter is the address of the first element, then the length; the heap does not move, so the address holds for the call. <!-- test: differential_natives_strings_and_arrays_cross_by_copy_and_by_pointer -->
* A `Pointer` result of NULL is the `NullPointer` fault; a `Pointer<T>?` result of NULL is none, and none goes in as NULL. <!-- test: differential_natives_a_null_pointer_result_is_a_fault, differential_natives_a_pointer_is_an_address_and_a_nullable_one_may_be_null -->
* A `var` parameter is passed as a pointer to a temporary that holds the value, and the value in the temporary is the final value of the variable after the call. <!-- test: differential_natives_a_var_parameter_is_passed_by_pointer_and_copied_back -->
* A call of a `@blocking` external function runs on a helper thread while the caller waits; any other call runs on the calling thread. <!-- test: differential_natives_a_blocking_call_runs_on_another_thread, differential_natives_numbers_cross_the_same_on_a_helper_thread -->
* An external function used as a value is called through the function that wraps it. <!-- test: differential_natives_a_native_is_a_function_value -->
* Before `main` the loader opens each library of the link table, writes the address of each function into a table the code calls through, and calls the optional `void mote_init(const MoteHost *)` of each library once. <!-- test: differential_natives_the_library_is_given_the_host_table_before_main -->
* In an executable, a library whose archive is linked has its functions in the slots as relocations to its symbols and is not opened; its optional `mote_init_<name>` is a weak symbol in the array the directory leads to, and the runtime calls the ones that resolved before `main`. <!-- test: object_tables_a_library_linked_in_has_its_symbols_in_the_slots_and_a_weak_init, aot_static_a_linked_library_gets_its_init_before_main, aot_static_a_program_whose_libraries_ship_an_archive_runs_without_the_shared_library -->
* A library or symbol that cannot be loaded is an error at its `extern` fn and nothing runs; an external function the program does not call is not loaded. <!-- test: differential_natives_a_missing_library_is_an_error_at_its_native_fn, differential_natives_a_missing_symbol_is_an_error_at_its_native_fn, differential_natives_a_native_the_program_never_calls_is_not_loaded -->

## Ports

* `MoteHost`, `MotePort`, the `PUSH_*` results and the `WAIT_*` values are defined in `mote_abi` (`host.rs`); a Rust external library depends on `mote-abi` for them. <!-- test: the_table_is_four_words_with_its_size_first, push_results_are_stable, wait_values_are_stable -->
* `abi/mote.h` is the C header of the table, generated by `mote_abi::host_header` from the same list; a test compares the file with the text (`MOTE_BLESS=1` rewrites it) and compiles a C file that asserts the size and offset of each field. <!-- test: host_header_file_is_the_generated_text, host_header_compiles_and_matches_the_layout_of_the_table -->

<!-- test: the_table_is_four_words_with_its_size_first -->
| `MoteHost` field | C type | Meaning |
|---|---|---|
| `size` | `uint64_t` | bytes in the table; a library reads only the fields inside it |
| `port_push` | `int32_t (*)(MotePort *, const uint8_t *, int64_t len)` | copies the message in; any thread, never waits |
| `port_close` | `void (*)(MotePort *)` | the sender is finished and this reference is released |
| `fd_wait` | `int32_t (*)(int32_t fd, int32_t interest, int64_t timeout_ms)` | blocks the calling thread until the descriptor is ready or the time is out |

<!-- test: push_results_are_stable -->
| `port_push` result | Meaning |
|---|---|
| `0` | queued |
| `1` | full; the message was not taken |
| `2` | closed; the `Receiver` was closed or dropped |

<!-- test: wait_values_are_stable -->
| `fd_wait` | Value |
|---|---|
| interest `0` / `1` | readable / writable |
| result `0` | ready; a closed peer counts as readable |
| result `1` | the time ran out |
| result `2` | the descriptor is negative, invalid or in error |

* `fd_wait` polls on the thread that calls it and parks no task; an external library function that waits calls it from a `@blocking` function. <!-- test: host_fd_wait_times_out_while_empty_and_is_ready_after_a_write, host_fd_wait_a_closed_peer_is_ready_and_a_bad_descriptor_fails -->
* New `MoteHost` fields go at the end. <!-- test: the_table_is_four_words_with_its_size_first -->

## The `TypeTable` API

<!-- test: none_is_the_reserved_first_type, a_class_can_refer_to_itself_through_an_optional, define_rejects_a_type_that_was_not_reserved, kinds_a_type_holds_what_typeof_and_the_collector_need, pair_payload_is_inline_a_reference_or_a_box_by_the_member_kind, past_the_limit_a_value_goes_through_memory -->
| Call | Does |
|---|---|
| `new()` | a table holding only `NONE` |
| `add(name, kind)` | adds a type whose members are already present |
| `reserve_object(name)` | a reference type that can be named before it is defined |
| `define(id, kind)` | gives a reserved type its fields |
| `get(id)` `get_raw(raw)` | `TypeInfo`, by id or by the raw `u32` a header holds |
| `layout(id)` | a value's `Layout` |
| `pair_payload(id)` | `Inline`, `Reference` or `Boxed` |
| `passing(table, id)` | `Registers(scalars)` or `Memory` |
