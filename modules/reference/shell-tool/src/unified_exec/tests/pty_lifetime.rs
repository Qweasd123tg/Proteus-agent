use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc,
};

struct FakeChild {
    reaped: Arc<AtomicBool>,
    signals: Arc<AtomicUsize>,
    lost_identity: bool,
    reap_entered: Option<mpsc::Sender<()>>,
    allow_reap: Option<Mutex<mpsc::Receiver<()>>>,
}

impl PtyChild for FakeChild {
    fn observe_exit(&mut self) -> io::Result<Option<i32>> {
        if self.lost_identity {
            return Err(io::Error::from_raw_os_error(libc::ECHILD));
        }
        Ok(Some(0))
    }
    fn kill(&mut self) {
        assert!(
            !self.reaped.load(Ordering::SeqCst),
            "stale process identity signalled"
        );
        self.signals.fetch_add(1, Ordering::SeqCst);
    }
    fn reap(&mut self) -> io::Result<()> {
        self.reaped.store(true, Ordering::SeqCst);
        if let Some(entered) = self.reap_entered.take() {
            entered.send(()).unwrap();
        }
        if let Some(allow) = self.allow_reap.take() {
            allow.lock().unwrap().recv().unwrap();
        }
        Ok(())
    }
}

#[test]
fn stale_session_eviction_cannot_signal_after_reap() {
    let reaped = Arc::new(AtomicBool::new(false));
    let signals = Arc::new(AtomicUsize::new(0));
    let lifetime = PtyLifetime::with_child(Box::new(FakeChild {
        reaped: reaped.clone(),
        signals: signals.clone(),
        lost_identity: false,
        reap_entered: None,
        allow_reap: None,
    }));
    assert_eq!(lifetime.observe_exit().unwrap(), Some(0));
    lifetime.finish();
    assert!(reaped.load(Ordering::SeqCst));
    assert_eq!(
        signals.load(Ordering::SeqCst),
        1,
        "descendants stopped before reap"
    );
    // The lifetime remains reachable in the store after exit, just as in LRU
    // and idle cleanup. No PID churn and no OS signals are involved.
    lifetime.kill();
    lifetime.finish();
    assert_eq!(signals.load(Ordering::SeqCst), 1);
}

#[test]
fn concurrent_cleanup_waits_for_reap_and_cannot_use_stale_identity() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let (allow_tx, allow_rx) = mpsc::channel();
    let reaped = Arc::new(AtomicBool::new(false));
    let signals = Arc::new(AtomicUsize::new(0));
    let lifetime = Arc::new(PtyLifetime::with_child(Box::new(FakeChild {
        reaped: reaped.clone(),
        signals: signals.clone(),
        lost_identity: false,
        reap_entered: Some(entered_tx),
        allow_reap: Some(Mutex::new(allow_rx)),
    })));
    let finish = lifetime.clone();
    let waiter = std::thread::spawn(move || finish.finish());
    entered_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    assert!(reaped.load(Ordering::SeqCst));
    assert!(
        lifetime.0.try_lock().is_err(),
        "reap must retain lifetime lock"
    );
    let cleanup = lifetime.clone();
    let killer = std::thread::spawn(move || cleanup.kill());
    allow_tx.send(()).unwrap();
    waiter.join().unwrap();
    killer.join().unwrap();
    assert_eq!(signals.load(Ordering::SeqCst), 1);
}

#[test]
fn externally_lost_child_identity_revokes_later_cleanup_signals() {
    let signals = Arc::new(AtomicUsize::new(0));
    let lifetime = PtyLifetime::with_child(Box::new(FakeChild {
        reaped: Arc::new(AtomicBool::new(true)),
        signals: signals.clone(),
        lost_identity: true,
        reap_entered: None,
        allow_reap: None,
    }));
    assert_eq!(
        lifetime.observe_exit().unwrap_err().raw_os_error(),
        Some(libc::ECHILD)
    );
    lifetime.kill();
    lifetime.finish();
    assert_eq!(signals.load(Ordering::SeqCst), 0);
}
