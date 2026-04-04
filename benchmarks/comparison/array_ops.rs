fn main() {
    let mut total: i64 = 0;
    for i in 0i64..1_500_000_000 {
        let arr = [i ^ total, i + 1, i + 2, i + 3, i + 4];
        total += arr[0] + arr[1] + arr[2] + arr[3] + arr[4];
    }
    println!("{}", total);
}
