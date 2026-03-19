fn double_val(x: i64) -> i64 { x * 2 }
fn add_one(x: i64) -> i64 { x + 1 }
fn apply(f: fn(i64) -> i64, x: i64) -> i64 { f(x) }

fn main() {
    let mut total: i64 = 0;
    for i in 0i64..10_000_000 {
        total += apply(double_val, i);
        total += add_one(double_val(i));
    }
    println!("{}", total);
}
