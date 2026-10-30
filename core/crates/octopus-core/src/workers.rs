//! Background threads for async steps, timeouts and other one-off work.
//!
//! A cached pool: a task goes to an idle thread when there is one, and
//! otherwise starts a new thread, so a task never waits behind a busy one
//! (activities may block). Threads idle for a minute exit.

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Duration;

type Task = Box<dyn FnOnce() + Send + 'static>;

const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Default)]
struct Queue {
    tasks: VecDeque<Task>,
    idle: usize,
}

struct Pool {
    queue: Mutex<Queue>,
    ready: Condvar,
}

fn pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(|| Pool {
        queue: Mutex::new(Queue::default()),
        ready: Condvar::new(),
    })
}

/// Runs `task` on a background thread.
pub fn spawn(task: impl FnOnce() + Send + 'static) {
    let pool = pool();
    let task: Task = Box::new(task);
    {
        let mut q = pool.queue.lock().unwrap();
        if q.idle > q.tasks.len() {
            q.tasks.push_back(task);
            pool.ready.notify_one();
            return;
        }
    }
    let started = std::thread::Builder::new()
        .name("octopus-worker".to_string())
        .spawn(move || {
            task();
            work(pool);
        });
    if let Err(e) = started {
        eprintln!("octopus: could not start a worker thread: {e}");
    }
}

fn work(pool: &'static Pool) {
    let mut q = pool.queue.lock().unwrap();
    loop {
        if let Some(task) = q.tasks.pop_front() {
            drop(q);
            // A panicking task must not take the thread's queue lock with it.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(task));
            q = pool.queue.lock().unwrap();
            continue;
        }
        q.idle += 1;
        let (guard, timeout) = pool.ready.wait_timeout(q, IDLE_TIMEOUT).unwrap();
        q = guard;
        q.idle -= 1;
        if timeout.timed_out() && q.tasks.is_empty() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn runs_tasks_and_reuses_threads() {
        let (tx, rx) = mpsc::channel();
        for i in 0..20 {
            let tx = tx.clone();
            spawn(move || tx.send((i, std::thread::current().id())).unwrap());
            // Let the thread go idle so the next task can reuse it.
            std::thread::sleep(Duration::from_millis(2));
        }
        let got: Vec<_> = (0..20)
            .map(|_| rx.recv_timeout(Duration::from_secs(5)).unwrap())
            .collect();
        let mut ids: Vec<_> = got.iter().map(|(_, t)| *t).collect();
        ids.sort_by_key(|t| format!("{t:?}"));
        ids.dedup();
        assert!(ids.len() < 20, "every task started its own thread");
    }

    #[test]
    fn a_blocked_task_does_not_hold_up_others() {
        let (release_tx, release_rx) = mpsc::channel::<()>();
        spawn(move || {
            let _ = release_rx.recv_timeout(Duration::from_secs(5));
        });
        let (tx, rx) = mpsc::channel();
        spawn(move || tx.send(()).unwrap());
        rx.recv_timeout(Duration::from_secs(2))
            .expect("second task waited for the first");
        release_tx.send(()).unwrap();
    }
}
