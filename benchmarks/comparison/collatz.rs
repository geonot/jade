fn collatz_steps(mut n: i64) -> i64 {
    let mut steps: i64 = 0;
    while n != 1 {
        if n % 2 == 0 { n /= 2; } else { n = 3 * n + 1; }
        steps += 1;
    }
    steps
}

fn main() {
    let mut max_steps: i64 = 0;
    let mut max_n: i64 = 0;
    for n in 1i64..5_000_000 {
        let s = collatz_steps(n);
        if s > max_steps { max_steps = s; max_n = n; }
    }
    println!("{}", max_n);
    println!("{}", max_steps);
}
