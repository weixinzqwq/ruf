# Minimal local patch to gc-arena 0.7.0

Source: the exact 0.7.0 crates.io package retained in the task's Cargo cache.
Upstream MIT and CC0 license files remain unchanged. Registry cache/checksum metadata
is deliberately excluded from this source dependency.

Changes are limited to `RunUntil::Step`, one stop check in `do_collection`, and
`Arena::collect_step`. This exposes one phase transition, root/object trace, or
sweep item independent of allocation debt. Existing barriers, trace operations,
weak-pointer rules, phase transitions and metrics accounting are unchanged.

The host can now continue an already active cycle even when object-count debt is
zero, and can periodically start a cycle when external bitmap allocations are
large but there are few GC allocations. Unlike a synchronous `finish_cycle`, the
host checks its elapsed-time budget between steps. One root or object trace is
indivisible, so the budget is cooperative, not a hard real-time guarantee.

Regression tests are in `tests/tests.rs` (prefix `aoqi_step_`) and the engine's
`core/src/player/aoqi_gc.rs`. No live objects are forcibly freed; normal tracing
continues to determine reachability.

The compile-fail snapshot `tests/ui/bad_write_field_projections.stderr` includes
Rust 1.99.1's backticks around `Deref`; error codes and rejected inputs are unchanged.
