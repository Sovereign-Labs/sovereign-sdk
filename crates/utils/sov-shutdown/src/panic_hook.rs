//! Panic-hook installation for full node services.

/// Chains [`tracing_panic::panic_hook`] onto the current panic hook so that
/// every panic — including those inside spawned [`crate::BackgroundHandle`]
/// tasks — is captured as a `tracing` event (with a backtrace) in addition to
/// whatever the previously installed hook does (e.g. the default
/// stderr/backtrace printer).
///
/// This is process-global and should be installed once, early in startup
/// (typically right after the `tracing` subscriber is initialized).
pub fn set_tracing_panic_hook() {
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        // `tracing_panic` logs the message + backtrace but has no notion of the
        // task name. If the panic happened inside a `BackgroundHandle` task,
        // emit a single self-contained line naming the task and its message.
        if let Some(task) = crate::background_handle::current_task_name() {
            let payload = panic_info.payload();
            let message = payload
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("<non-string panic payload>");
            tracing::error!("panic in background task {task}: {message}");
        }
        tracing_panic::panic_hook(panic_info);
        prev_hook(panic_info);
    }));
}
