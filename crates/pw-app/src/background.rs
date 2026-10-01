//! Running blocking work (file walks) off the UI thread.

use iced::futures::channel::oneshot;

/// Runs `f` on its own thread; the future resolves with its result.
pub async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = oneshot::channel();
    std::thread::Builder::new()
        .name("background".into())
        .spawn(move || {
            let _ = tx.send(f());
        })
        .expect("spawn background thread");
    rx.await.expect("background work finished")
}

/// Resolves after `duration`, without a timer on the UI thread.
pub async fn sleep(duration: std::time::Duration) {
    blocking(move || std::thread::sleep(duration)).await;
}
