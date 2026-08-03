use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

pub fn workers() -> usize {
    std::env::var("JINN_TEST_JOBS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
        })
}

pub fn par_map<T, R, F>(items: Vec<T>, f: F) -> Vec<R>
where
    T: Send + Sync,
    R: Send,
    F: Fn(&T) -> R + Sync,
{
    let n = items.len();
    if n == 0 {
        return Vec::new();
    }
    let jobs = workers().min(n);
    if jobs <= 1 {
        return items.iter().map(&f).collect();
    }

    let slots: Vec<Mutex<Option<R>>> = (0..n).map(|_| Mutex::new(None)).collect();
    let next = AtomicUsize::new(0);
    let items = &items;
    let slots = &slots;
    let next = &next;
    let f = &f;

    std::thread::scope(|s| {
        for _ in 0..jobs {
            s.spawn(move || {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= n {
                        break;
                    }
                    let r = f(&items[i]);
                    *slots[i].lock().unwrap() = Some(r);
                }
            });
        }
    });

    slots
        .iter()
        .map(|m| m.lock().unwrap().take().expect("worker filled every slot"))
        .collect()
}
