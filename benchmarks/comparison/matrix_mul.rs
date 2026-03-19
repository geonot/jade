fn main() {
    let n: i64 = 800;
    let mut total: i64 = 0;
    for i in 0..n {
        for j in 0..n {
            let mut sum: i64 = 0;
            for k in 0..n {
                sum = sum.wrapping_add((i * n + k).wrapping_mul(k * n + j));
            }
            total = total.wrapping_add(sum);
        }
    }
    println!("{}", total);
}
