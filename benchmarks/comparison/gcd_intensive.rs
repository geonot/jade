fn gcd(mut a: i64, mut b: i64) -> i64 {
    while b != 0 { let t = b; b = a % b; a = t; }
    a
}

fn main() {
    let mut sum: i64 = 0;
    for i in 1i64..10_000 {
        let mut j = i + 1;
        while j < 10_000 { sum += gcd(i, j); j += 100; }
    }
    println!("{}", sum);
}
