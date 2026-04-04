enum Op { Add(i64, i64), Mul(i64, i64), Neg(i64) }

fn eval_op(op: Op) -> i64 {
    match op {
        Op::Add(a, b) => a + b,
        Op::Mul(a, b) => a * b,
        Op::Neg(a) => -a,
    }
}

fn main() {
    let mut total: i64 = 0;
    for i in 0i64..2_000_000_000 {
        total += eval_op(Op::Add(i, i + 1));
        total += eval_op(Op::Mul(i, 2));
        total += eval_op(Op::Neg(i));
        total ^= i;
    }
    println!("{}", total);
}
