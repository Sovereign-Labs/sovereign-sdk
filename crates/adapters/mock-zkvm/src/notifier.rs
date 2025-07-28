use std::sync::{Arc, Condvar, Mutex};

#[derive(Clone, Default)]
struct Notifier {
    notified: Arc<Mutex<bool>>,
    cond: Arc<Condvar>,
}

impl Notifier {
    fn wait(&self) {
        let mut notified = self.notified.lock().unwrap();
        while !*notified {
            notified = self.cond.wait(notified).unwrap();
        }
    }

    fn notify(&self) {
        let mut notified = self.notified.lock().unwrap();
        *notified = true;
        self.cond.notify_one();
    }
}

#[derive(Clone, Default)]
struct Pool {
    notifiers: Vec<Notifier>,
    pending_notifications: u64,
}

/// Manages a pool of Notifiers and buffers missed notifications.
/// The `wait` function suspends the current thread until it receives a notification via a call to `notify` and inserts a new `Notifier`` into the pool.
/// A pending notification occurs when notify is called on an empty pool. In such cases,
/// subsequent calls to `wait` will not block until all pending notifications are consumed.
#[derive(Clone, Default)]
pub(crate) struct NotificationManager {
    pool: Arc<Mutex<Pool>>,
}

impl NotificationManager {
    pub(crate) fn notify(&self) {
        let mut internal = self.pool.lock().unwrap();
        let notifier = internal.notifiers.pop();
        if let Some(notifier) = notifier {
            notifier.notify();
        } else {
            internal.pending_notifications += 1;
        }
    }

    pub(crate) fn wait(&self) {
        let mut internal = self.pool.lock().unwrap();
        if internal.pending_notifications > 0 {
            internal.pending_notifications -= 1;
        } else {
            let notifier = Notifier::default();
            internal.notifiers.push(notifier.clone());
            drop(internal);
            notifier.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{mpsc, Arc};

    use super::NotificationManager;

    impl NotificationManager {
        fn notifications(&self) -> u64 {
            self.pool.lock().unwrap().pending_notifications
        }
    }

    #[test]
    fn test_notifier() {
        let notification_manager = NotificationManager::default();
        let counter = Arc::new(AtomicU64::new(0));
        let (tx, rx) = mpsc::channel::<()>();

        let num_threads = 10;
        for _ in 0..num_threads {
            let notification_manager = notification_manager.clone();
            let counter = counter.clone();
            let tx = tx.clone();
            std::thread::spawn(move || {
                notification_manager.wait();
                counter.fetch_add(1, Ordering::SeqCst);
                tx.send(()).unwrap();
            });
        }

        {
            assert_eq!(counter.load(Ordering::SeqCst), 0);
        }

        // Check if only one thread is woken up.
        for i in 1..num_threads + 1 {
            notification_manager.notify();
            rx.recv().unwrap();
            assert_eq!(counter.load(Ordering::SeqCst), i);
        }

        // Check if missed notifications are buffered.
        {
            let notification_manager = NotificationManager::default();
            notification_manager.notify();
            assert_eq!(1, notification_manager.notifications());
            notification_manager.wait();
            assert_eq!(0, notification_manager.notifications());
        }
    }
}
