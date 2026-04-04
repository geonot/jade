fn main() {
    let n: i64 = 2_000_000_000;
    let mut sum: i64 = 0;
    for i in 0..n { sum ^= i; sum += i; }
    println!("{}", sum);
}
