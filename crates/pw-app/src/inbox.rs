//! Delivers events from background threads to the app as one subscription.
//!
//! Threads hold the sending half; the app subscribes once. The receiver is handed to the
//! subscription the first time it starts, and the subscription's identity is the inbox's name,
//! so re-running `subscription()` every update never restarts it.

use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use iced::Subscription;
use iced::futures::channel::mpsc::{self, UnboundedReceiver, UnboundedSender};
use iced::futures::stream::{self, BoxStream, StreamExt};

pub struct Inbox<T> {
    name: &'static str,
    rx: Arc<Mutex<Option<UnboundedReceiver<T>>>>,
}

impl<T> Clone for Inbox<T> {
    fn clone(&self) -> Self {
        Self { name: self.name, rx: self.rx.clone() }
    }
}

impl<T> Hash for Inbox<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // There is exactly one inbox of each name per app, so the name is its identity.
        self.name.hash(state);
    }
}

/// A sender for background threads and the inbox the app subscribes to.
pub fn channel<T>(name: &'static str) -> (UnboundedSender<T>, Inbox<T>) {
    let (tx, rx) = mpsc::unbounded();
    (tx, Inbox { name, rx: Arc::new(Mutex::new(Some(rx))) })
}

impl<T: Send + 'static> Inbox<T> {
    pub fn subscription(&self) -> Subscription<T> {
        Subscription::run_with(self.clone(), |inbox: &Inbox<T>| -> BoxStream<'static, T> {
            match inbox.rx.lock().expect("inbox lock").take() {
                Some(rx) => rx.boxed(),
                None => stream::empty().boxed(),
            }
        })
    }
}
