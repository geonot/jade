fn apply(f: fn(i64) -> i64, x: i64) -> i64 { f(x) }

fn main() {
    let base: i64 = 100;
    let adder = |x: i64| -> i64 { base + x };
    let mut total: i64 = 0;
    for i in 0i64..2_000_000_000 {
        total += adder(i);
        total += apply(|x| base + x, i);
        total ^= i;
    }
    println!("{}", total);
}
