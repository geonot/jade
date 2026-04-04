fn main() {
    let mut sum: i64 = 0;
    for i in 1i64..=40_000 {
        for j in 1i64..=40_000 {
            sum = (sum ^ (i * j)) + i - j;
        }
    }
    println!("{}", sum);
}
