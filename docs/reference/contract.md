# Compiler and runtime contract

## Lang items

* A `std` declaration marked `@lang("name")` is one the compiler looks up by that name. <!-- test: lang_items_are_std_only_and_register_once, a_lang_declaration_stands_for_its_built_in_type, every_lang_item_is_declared -->
* A handle is Sendable and does not change inside: a task may capture one and a `Once` may hold one. <!-- test: a_handle_is_sendable_and_does_not_change_inside_so_a_once_may_hold_one, a_mutex_and_a_once_are_sendable_handles -->
* The names are these, and `yes` marks a handle. <!-- test: tables_every_generated_table_in_the_documents_is_what_the_code_says -->

<!-- table: lang-items -->
| `@lang` name | Handle |
|---|---|
| `bool` | no |
| `string` | no |
| `int8` | no |
| `int16` | no |
| `int32` | no |
| `int` | no |
| `uint8` | no |
| `uint16` | no |
| `uint32` | no |
| `uint64` | no |
| `float32` | no |
| `float` | no |
| `option` | no |
| `result` | no |
| `list` | no |
| `array` | no |
| `map` | no |
| `set` | no |
| `range` | no |
| `stream` | no |
| `task` | yes |
| `sender` | yes |
| `receiver` | yes |
| `mutex` | yes |
| `once` | yes |
| `atomic_int` | yes |
| `atomic_bool` | yes |
| `port_end` | yes |
| `pointer` | no |
| `display` | no |
| `debug` | no |
| `eq` | no |
| `ord` | no |
| `clone` | no |
| `closeable` | no |
| `key` | no |
| `error` | no |
| `error_kind` | no |
<!-- /table -->

## Intrinsics

* A `native fn` in `std` marked `@intrinsic("name")` is an operation the compiler or runtime provides. <!-- test: an_intrinsic_is_a_generic_native_function_in_std -->
* The operations are these, each with the number of its arguments and its effects. <!-- test: tables_every_generated_table_in_the_documents_is_what_the_code_says, tables_intrinsic_effects_are_named_in_a_fixed_order -->

<!-- table: intrinsics -->
| Intrinsic | Arguments | Effects |
|---|---|---|
| `print` | 1 | io |
| `println` | 1 | io |
| `eprint` | 1 | io |
| `eprintln` | 1 | io |
| `sys_read_line` | 0 | io |
| `sys_args` | 0 | reads |
| `sys_exit` | 1 | io |
| `sys_now` | 0 | io |
| `typeof` | 1 | pure |
| `fault` | 1 | faults |
| `port_open` | 1 | writes |
| `float_is_nan` | 1 | pure |
| `float_is_infinite` | 1 | pure |
| `float_to_int` | 1 | pure |
| `num_to_string` | 1 | pure |
| `num_wrapping_add` | 2 | pure |
| `num_wrapping_sub` | 2 | pure |
| `num_wrapping_mul` | 2 | pure |
| `num_wrapping_neg` | 1 | pure |
| `num_wrapping_shl` | 2 | pure |
| `num_wrapping_shr` | 2 | pure |
| `num_checked_add` | 2 | pure |
| `num_checked_sub` | 2 | pure |
| `num_checked_mul` | 2 | pure |
| `num_checked_neg` | 1 | pure |
| `num_checked_shl` | 2 | pure |
| `num_checked_shr` | 2 | pure |
| `num_saturating_add` | 2 | pure |
| `num_saturating_sub` | 2 | pure |
| `num_saturating_mul` | 2 | pure |
| `num_saturating_neg` | 1 | pure |
| `num_saturating_shl` | 2 | pure |
| `num_saturating_shr` | 2 | pure |
| `num_overflowing_add` | 2 | pure |
| `num_overflowing_sub` | 2 | pure |
| `num_overflowing_mul` | 2 | pure |
| `num_overflowing_neg` | 1 | pure |
| `num_overflowing_shl` | 2 | pure |
| `num_overflowing_shr` | 2 | pure |
| `math_sqrt` | 1 | pure |
| `math_floor` | 1 | pure |
| `math_ceil` | 1 | pure |
| `math_round` | 1 | pure |
| `math_pow` | 2 | pure |
| `math_exp` | 1 | pure |
| `math_ln` | 1 | pure |
| `math_log10` | 1 | pure |
| `math_sin` | 1 | pure |
| `math_cos` | 1 | pure |
| `math_tan` | 1 | pure |
| `math_atan2` | 2 | pure |
| `parse_int` | 1 | pure |
| `parse_float` | 1 | pure |
| `string_len` | 1 | pure |
| `string_concat` | 2 | pure |
| `string_replace` | 3 | pure |
| `string_slice` | 3 | faults |
| `string_to_upper` | 1 | pure |
| `string_to_lower` | 1 | pure |
| `string_trim` | 1 | pure |
| `string_repeat` | 2 | faults |
| `string_find` | 2 | pure |
| `string_rfind` | 2 | pure |
| `string_split` | 2 | pure |
| `string_bytes` | 1 | pure |
| `string_from_bytes` | 1 | pure |
| `list_new` | 0 | pure |
| `list_len` | 1 | reads |
| `list_push` | 2 | writes |
| `list_pop` | 1 | writes |
| `list_get` | 2 | reads, faults |
| `list_set` | 3 | writes, faults |
| `list_clear` | 1 | writes |
| `list_contains` | 2 | reads |
| `list_slice` | 3 | reads, faults |
| `list_set_slice` | 4 | writes, faults |
| `list_clone` | 1 | reads |
| `deep_clone` | 1 | reads |
| `hash_of` | 1 | pure |
| `key_eq` | 2 | pure |
| `equal_of` | 2 | reads |
| `array_len` | 1 | pure |
| `array_get` | 2 | reads, faults |
| `array_set` | 3 | writes, faults |
| `array_contains` | 2 | reads |
| `array_slice` | 3 | reads, faults |
| `array_clone` | 1 | reads |
| `stream_next` | 1 | writes, parks |
| `task_await` | 1 | parks |
| `task_await_timeout` | 2 | parks |
| `task_cancel` | 1 | writes |
| `task_is_ready` | 1 | reads |
| `spawn_call` | 1 | writes |
| `task_view` | 2 | reads |
| `task_value` | 1 | reads |
| `sleep` | 1 | parks |
| `channel_new` | 1 | faults |
| `sender_send` | 2 | writes, parks |
| `sender_try_send` | 2 | writes |
| `sender_close` | 1 | writes |
| `sender_clone` | 1 | writes |
| `receiver_recv` | 1 | writes, parks |
| `receiver_recv_timeout` | 2 | writes, parks |
| `receiver_try_recv` | 1 | writes |
| `receiver_is_closed` | 1 | reads |
| `receiver_close` | 1 | writes |
| `receiver_clone` | 1 | writes |
| `mutex_new` | 1 | pure |
| `mutex_take` | 1 | writes, faults, parks |
| `mutex_put` | 2 | writes, faults |
| `once_new` | 0 | pure |
| `once_begin` | 1 | writes, faults, parks |
| `once_finish` | 2 | writes |
| `once_value` | 1 | reads, faults |
| `atomic_int_new` | 1 | pure |
| `atomic_int_load` | 1 | reads |
| `atomic_int_store` | 2 | writes |
| `atomic_int_swap` | 2 | writes |
| `atomic_int_compare_exchange` | 3 | writes |
| `atomic_int_fetch_add` | 2 | writes, faults |
| `atomic_int_fetch_sub` | 2 | writes, faults |
| `atomic_bool_new` | 1 | pure |
| `atomic_bool_load` | 1 | reads |
| `atomic_bool_store` | 2 | writes |
| `atomic_bool_swap` | 2 | writes |
| `atomic_bool_compare_exchange` | 3 | writes |
| `type_id` | 0 | pure |
| `meta_ints` | 0 | pure |
| `meta_texts` | 0 | pure |
| `dyn_of` | 2 | pure |
| `dyn_type` | 1 | reads |
| `dyn_to` | 2 | reads |
| `dyn_len` | 1 | reads |
| `dyn_tag` | 1 | reads |
| `dyn_get` | 2 | reads, faults |
| `dyn_set` | 3 | writes, faults |
| `dyn_show` | 2 | reads |
| `require` | 2 | faults |
| `dyn_build` | 3 | reads, faults |
| `dyn_make` | 3 | reads, faults |
<!-- /table -->

## Faults

* The kinds of fault the toolchain defines are these. <!-- test: tables_every_generated_table_in_the_documents_is_what_the_code_says -->

<!-- table: fault-kinds -->
| Kind | Message |
|---|---|
| `IntegerOverflow` | integer overflow |
| `DivideByZero` | division by zero |
| `ShiftOutOfRange` | shift out of range |
| `IndexOutOfRange` | index out of range |
| `UnwrapFailed` | unwrapped a missing value |
| `NullPointer` | native returned a null pointer |
| `Raised` | the message the program gives |
| `MemoryLimit` | memory limit reached |
| `StackDepth` | call stack too deep |
| `Deadlock` | deadlock: every task waits |
<!-- /table -->
