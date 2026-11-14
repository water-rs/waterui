# waterui-contract

`contract_violation!` reports a broken contract the way WaterUI code running
inside a user's application must: a debug build panics with the message, a
release build logs it through `tracing::error!` and the caller continues to a
defined outcome.

It depends only on `tracing` without default features, builds on `no_std`, and
is shared by the framework and the engines below it. `waterui-core` re-exports
the macro.
