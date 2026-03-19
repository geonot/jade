struct Vec3 { x: i64, y: i64, z: i64 }

fn dot(a: &Vec3, b: &Vec3) -> i64 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

fn main() {
    let mut total: i64 = 0;
    for i in 0i64..10_000_000 {
        let a = Vec3 { x: i, y: i + 1, z: i + 2 };
        let b = Vec3 { x: i + 3, y: i + 4, z: i + 5 };
        total += dot(&a, &b);
    }
    println!("{}", total);
}
