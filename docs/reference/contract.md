# Compiler and runtime contract

## Universal builtins and intrinsics

* Builtin types (`List`, `Array`, `Map`, `Set`, `Range`, `Stream`, `Option`, `Result`, `Error`, `ErrorKind`, `Pointer`, `Task`, `Sender`, `Receiver`, `Mutex`, `Once`, `PortEnd`, `AtomicInt`, `AtomicBool`) live in the universal scope and do not need `@lang` attributes. <!-- test: universal_builtin_types_resolve_without_import -->
* A compiler builtin `@name(...)` (or `@__name(...)`) invokes a compiler intrinsic or compile-time reflection operation directly in any module. <!-- block -->

```mote
fn main() {
    println(@type_name(Int))
}
```

```output
Int
```

## Intrinsics

* Compiler builtins and runtime intrinsics are operations provided directly by the compiler or runtime via `@name(...)`. <!-- test: tables_every_generated_table_in_the_documents_is_what_the_code_says -->
* The runtime operations are these, each with the number of its arguments and its effects. <!-- test: tables_every_generated_table_in_the_documents_is_what_the_code_says, tables_intrinsic_effects_are_named_in_a_fixed_order -->

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
| `type_tag` | 1 | pure |
| `fault` | 1 | faults |
| `index_out_of_range` | 0 | faults |
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
| `string_byte_at` | 2 | faults |
| `string_is_boundary` | 2 | pure |
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
| `mask_all` | 1 | pure |
| `mask_any` | 1 | pure |
| `mask_none` | 1 | pure |
| `array_all_by` | 2 | reads |
| `array_any_by` | 2 | reads |
| `array_none_by` | 2 | reads |
| `array_count` | 2 | reads |
| `array_count_by` | 2 | reads |
| `array_sum` | 1 | pure |
| `array_product` | 1 | pure |
| `array_mean` | 1 | pure |
| `array_min` | 1 | reads |
| `array_max` | 1 | reads |
| `array_argmin` | 1 | reads |
| `array_argmax` | 1 | reads |
| `array_dot` | 2 | reads, faults |
| `array_norm_sq` | 1 | pure |
| `array_fold` | 3 | reads |
| `array_reduce` | 2 | reads |
| `array_scan` | 3 | reads |
| `array_find` | 2 | reads |
| `array_find_index` | 2 | reads |
| `array_index_of` | 2 | reads |
| `array_select` | 3 | reads, faults |
| `array_map` | 2 | reads |
| `array_zip_with` | 3 | reads |
| `array_reverse` | 1 | reads |
| `array_clamp` | 3 | reads |
| `array_filter` | 2 | reads |
| `array_zip` | 2 | reads |
| `array_enumerate` | 1 | reads |
| `array_concat` | 2 | reads |
| `array_take` | 2 | reads |
| `array_drop` | 2 | reads |
| `array_flatten` | 1 | reads |
| `array_sort` | 1 | reads |
| `array_sort_by` | 2 | reads |
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
| `ptr_null` | 0 | pure |
| `ptr_from_addr` | 1 | pure |
| `ptr_alloc` | 1 | writes, faults |
| `ptr_realloc` | 2 | writes, faults |
| `ptr_free` | 1 | writes |
| `ptr_offset` | 2 | faults |
| `ptr_load` | 2 | reads, faults |
| `ptr_store` | 3 | writes, faults |
| `ptr_copy` | 3 | writes, faults |
| `ptr_zero` | 2 | writes, faults |
| `ptr_cast` | 1 | pure |
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
| `NullPointer` | external library returned a null pointer |
| `Raised` | the message the program gives |
| `MemoryLimit` | memory limit reached |
| `StackDepth` | call stack too deep |
| `Deadlock` | deadlock: every task waits |
<!-- /table -->
