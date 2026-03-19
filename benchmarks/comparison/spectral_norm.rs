fn a_elem(i: i64, j: i64) -> i64 {
    ((i + j) * (i + j + 1)) / 2 + i + 1
}

fn main() {
    let n: i64 = 1000;
    let mut sum: i64 = 0;
    for _ in 0..500 {
        for i in 0..n {
            let mut acc: i64 = 0;
            for j in 0..n {
                acc = acc.wrapping_add(a_elem(i, j).wrapping_mul(j + 1));
            }
            sum = sum.wrapping_add(acc % 1000000);
        }
    }
    println!("{}", sum);
}
