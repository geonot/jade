use std::sync::{Arc, Mutex, Condvar};
use std::thread;

const MAILBOX_CAP: usize = 256;

struct Mailbox {
    buf: Vec<i64>,
    head: usize,
    tail: usize,
    count: usize,
    alive: bool,
    state_count: i64,
}

fn main() {
    let pair = Arc::new((Mutex::new(Mailbox {
        buf: vec![0i64; MAILBOX_CAP],
        head: 0, tail: 0, count: 0, alive: true, state_count: 0,
    }), Condvar::new(), Condvar::new()));

    let worker_pair = Arc::clone(&pair);
    thread::spawn(move || {
        let (lock, cond_ne, cond_nf) = &*worker_pair;
        loop {
            let mut mb = lock.lock().unwrap();
            while mb.count == 0 && mb.alive {
                mb = cond_ne.wait(mb).unwrap();
            }
            if !mb.alive { break; }
            let n = mb.buf[mb.head];
            mb.head = (mb.head + 1) % MAILBOX_CAP;
            mb.count -= 1;
            mb.state_count += n;
            cond_nf.notify_one();
        }
    });

    let (lock, cond_ne, cond_nf) = &*pair;
    for _ in 0..1_000_000 {
        let mut mb = lock.lock().unwrap();
        while mb.count == MAILBOX_CAP {
            mb = cond_nf.wait(mb).unwrap();
        }
        mb.buf[mb.tail] = 1;
        mb.tail = (mb.tail + 1) % MAILBOX_CAP;
        mb.count += 1;
        cond_ne.notify_one();
    }

    std::thread::sleep(std::time::Duration::from_millis(500));
    println!("0");
}
