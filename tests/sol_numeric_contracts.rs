use std::fmt::Write;
use std::process::Command;

fn compile(
    source: &str,
    opt: &str,
) -> (tempfile::TempDir, std::path::PathBuf, std::process::Output) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("numeric.jn");
    let binary = dir.path().join("numeric");
    std::fs::write(&path, source).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_jinnc"))
        .arg(&path)
        .args(["--opt", opt, "-o"])
        .arg(&binary)
        .output()
        .unwrap();
    (dir, binary, output)
}

fn expect(source: &str, expected: &str) {
    for opt in ["0", "3"] {
        let (dir, binary, compiled) = compile(source, opt);
        assert!(
            compiled.status.success(),
            "opt {opt}: {}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        let output = Command::new(binary)
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "opt {opt}: {}\nstdout: {}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            expected,
            "opt {opt}"
        );
    }
}

fn oracle(a: i128, b: i128, bits: u32, signed: bool, op: &str) -> (i128, i128, bool) {
    let modulus = 1i128 << bits;
    let mask = modulus - 1;
    if signed {
        let min = -(1i128 << (bits - 1));
        let max = (1i128 << (bits - 1)) - 1;
        let mathematical = match op {
            "add" => a + b,
            "sub" => a - b,
            "mul" => a * b,
            _ => unreachable!(),
        };
        let raw = mathematical & mask;
        let wrapped = if raw > max { raw - modulus } else { raw };
        (
            wrapped,
            mathematical.clamp(min, max),
            mathematical < min || mathematical > max,
        )
    } else {
        let a = a as u128;
        let b = b as u128;
        let mathematical = match op {
            "add" => a.checked_add(b),
            "sub" => a.checked_sub(b),
            "mul" => a.checked_mul(b),
            _ => unreachable!(),
        };
        let max = mask as u128;
        match mathematical {
            Some(value) => ((value & max) as i128, value.min(max) as i128, value > max),
            None => (((a.wrapping_sub(b)) & max) as i128, 0, true),
        }
    }
}

fn integer_matrix(bits: u32, signed: bool) {
    let ty = format!("{}{bits}", if signed { 'i' } else { 'u' });
    let min = if signed { -(1i128 << (bits - 1)) } else { 0 };
    let max = (1i128 << (bits - u32::from(signed))) - 1;
    let mut pairs = vec![
        (0, 0),
        (1, 1),
        (min, 1),
        (max, 1),
        (max, max),
        (min, min),
        (min, max),
        (max, min),
        (max / 2, 2),
        (max / 2 + 1, 2),
    ];
    if signed {
        pairs.extend([
            (min, -1),
            (-1, min),
            (-1, max),
            (max, -1),
            (min / 2 - 1, 2),
            (-2, -3),
        ]);
    }
    let mut seed = 0x56a94f17a30473d1u64;
    for _ in 0..12 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let a = min + (seed as i128 % (max - min + 1));
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let b = min + (seed as i128 % (max - min + 1));
        pairs.push((a, b));
    }
    let mut source = String::from("*main\n");
    let mut expected = String::new();
    for (index, (a, b)) in pairs.into_iter().enumerate() {
        writeln!(
            source,
            "    a{index} is ({a}) as {ty}\n    b{index} is ({b}) as {ty}"
        )
        .unwrap();
        for op in ["add", "sub", "mul"] {
            let (value, saturated, overflow) = oracle(a, b, bits, signed, op);
            writeln!(source, "    value{index}{op}, flag{index}{op} is checked_{op}(a{index}, b{index})\n    log(value{index}{op})\n    log(flag{index}{op})\n    log(saturating_{op}(a{index}, b{index}))\n    log(wrapping_{op}(a{index}, b{index}))").unwrap();
            writeln!(expected, "{value}\n{overflow}\n{saturated}\n{value}").unwrap();
        }
    }
    expect(&source, &expected);
}

macro_rules! integer_tests {
    ($($name:ident: ($bits:literal, $signed:literal)),* $(,)?) => {
        $(#[test] fn $name() { integer_matrix($bits, $signed); })*
    };
}

integer_tests! {
    arithmetic_i8: (8, true), arithmetic_u8: (8, false),
    arithmetic_i16: (16, true), arithmetic_u16: (16, false),
    arithmetic_i32: (32, true), arithmetic_u32: (32, false),
    arithmetic_i64: (64, true), arithmetic_u64: (64, false),
}

#[test]
fn checked_aggregate_survives_calls_and_indexing() {
    expect(
        "*add(a as u8, b as u8) returns (u8, bool)\n    checked_add(a, b)\n\n*main\n    pair is add(250, 10)\n    log(pair[0])\n    log(pair[1])\n    value, flag is checked_add(12, 30)\n    log(value)\n    log(flag)\n",
        "4\ntrue\n42\nfalse\n",
    );
}

#[test]
fn integer_intrinsics_reject_invalid_signatures() {
    for expression in [
        "checked_add(1.0, 2.0)",
        "saturating_mul(true, false)",
        "wrapping_sub(1 as u8, 2 as i8)",
        "checked_add(1)",
        "saturating_add(1, 2, 3)",
    ] {
        let source = format!("*main\n    log({expression})\n");
        let (_dir, _binary, output) = compile(&source, "0");
        assert!(!output.status.success(), "accepted {expression}");
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(
            !diagnostic.contains("panicked") && !diagnostic.contains("ICE:"),
            "{diagnostic}"
        );
    }
}

fn float_matrix(single: bool) {
    let source_type = if single { "f32" } else { "f64" };
    let mut source = String::from("*main\n");
    let mut expected = String::new();
    let mut index = 0;
    for bits in [8, 16, 32, 64] {
        for signed in [false, true] {
            let target = format!("{}{bits}", if signed { 'i' } else { 'u' });
            let high = 2f64.powi(bits - i32::from(signed));
            let low = if signed { -high } else { 0.0 };
            let mut values = vec![0.0, -0.0, 1.5, -1.5, 1000.0, -1000.0, high, low];
            if single {
                let h = high as f32;
                let l = low as f32;
                values.extend([
                    f32::from_bits(h.to_bits() - 1) as f64,
                    f32::from_bits(h.to_bits() + 1) as f64,
                ]);
                if signed {
                    values.extend([
                        f32::from_bits(l.to_bits() - 1) as f64,
                        f32::from_bits(l.to_bits() + 1) as f64,
                    ]);
                }
            } else {
                values.extend([
                    f64::from_bits(high.to_bits() - 1),
                    f64::from_bits(high.to_bits() + 1),
                ]);
                if signed {
                    values.extend([
                        f64::from_bits(low.to_bits() - 1),
                        f64::from_bits(low.to_bits() + 1),
                    ]);
                }
            }
            values.extend([f64::INFINITY, f64::NEG_INFINITY, f64::NAN]);
            for value in values {
                let expression = if value.is_nan() {
                    "0.0 / 0.0".to_owned()
                } else if value == f64::INFINITY {
                    "1.0 / 0.0".to_owned()
                } else if value == f64::NEG_INFINITY {
                    "-1.0 / 0.0".to_owned()
                } else {
                    format!("{value:?}")
                };
                writeln!(
                    source,
                    "    f{index} is ({expression}) as {source_type}\n    log(f{index} as {target})"
                )
                .unwrap();
                let value = if single { (value as f32) as f64 } else { value };
                let integer = match (bits, signed) {
                    (8, true) => (value as i8).to_string(),
                    (8, false) => (value as u8).to_string(),
                    (16, true) => (value as i16).to_string(),
                    (16, false) => (value as u16).to_string(),
                    (32, true) => (value as i32).to_string(),
                    (32, false) => (value as u32).to_string(),
                    (64, true) => (value as i64).to_string(),
                    (64, false) => (value as u64).to_string(),
                    _ => unreachable!(),
                };
                writeln!(expected, "{integer}").unwrap();
                index += 1;
            }
        }
    }
    expect(&source, &expected);
}

#[test]
fn saturating_f32_casts_cover_boundaries_and_nonfinite_values() {
    float_matrix(true);
}

#[test]
fn saturating_f64_casts_cover_boundaries_and_nonfinite_values() {
    float_matrix(false);
}

fn expect_trap(source: &str, message: &str) {
    for opt in ["0", "3"] {
        let (dir, binary, compiled) = compile(source, opt);
        assert!(
            compiled.status.success(),
            "{}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        let output = Command::new(binary)
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(!output.status.success(), "did not trap at opt {opt}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(message),
            "opt {opt}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.stdout.is_empty(),
            "continued after trapping operation at opt {opt}"
        );
    }
}

#[test]
fn integer_power_is_exact_for_every_domain() {
    let mut source = String::from("*main\n");
    let mut expected = String::new();
    let mut index = 0;
    for bits in [8, 16, 32, 64] {
        for signed in [true, false] {
            let ty = format!("{}{bits}", if signed { 'i' } else { 'u' });
            let min = if signed { -(1i128 << (bits - 1)) } else { 0 };
            let max = (1i128 << (bits - u32::from(signed))) - 1;
            let mut cases = vec![
                (0, 0),
                (0, max),
                (1, max),
                (max, 0),
                (max, 1),
                (2, bits as i128 - i128::from(signed) - 1),
                (3, 3),
            ];
            if signed {
                cases.extend([
                    (-1, max),
                    (-1, max - 1),
                    (min, 1),
                    (-2, 3),
                    (-2, 4),
                    (-2, bits as i128 - 1),
                ]);
            }
            if bits == 64 {
                cases.push((9007199254740993, 1));
            }
            for (base, exponent) in cases {
                writeln!(source, "    a{index} is ({base}) as {ty}\n    b{index} is {exponent} as {ty}\n    log(a{index} pow b{index})").unwrap();
                let value = match (base, exponent) {
                    (_, 0) => 1,
                    (0, _) => 0,
                    (1, _) => 1,
                    (-1, _) => {
                        if exponent % 2 == 0 {
                            1
                        } else {
                            -1
                        }
                    }
                    _ => base.checked_pow(u32::try_from(exponent).unwrap()).unwrap(),
                };
                writeln!(expected, "{value}").unwrap();
                index += 1;
            }
        }
    }
    source.push_str("    log(2.0 pow 0.5 > 1.4)\n");
    expected.push_str("true\n");
    expect(&source, &expected);
}

#[test]
fn integer_power_rejects_overflow_even_when_unused() {
    for bits in [8, 16, 32, 64] {
        for signed in [false, true] {
            let ty = format!("{}{bits}", if signed { 'i' } else { 'u' });
            let exponent = bits - u32::from(signed);
            let source = format!(
                "*main\n    a is 2 as {ty}\n    b is {exponent} as {ty}\n    unused is a pow b\n    log('continued')\n"
            );
            expect_trap(&source, "integer overflow");
        }
    }
    expect_trap(
        "*main\n    a is 0 as i8\n    b is (-1) as i8\n    unused is a pow b\n    log('continued')\n",
        "negative integer exponent",
    );
}

#[test]
fn unused_checked_integer_operators_keep_their_traps() {
    for op in ["+", "-", "*"] {
        let rhs = if op == "-" { -1 } else { 2 };
        expect_trap(
            &format!(
                "*main\n    a is 9223372036854775807\n    b is {rhs}\n    unused is a {op} b\n    log('continued')\n"
            ),
            "integer overflow",
        );
    }
}

#[test]
fn integer_literal_boundaries_preserve_every_domain_and_base() {
    let mut source = String::new();
    let mut body = String::from("*main\n");
    let mut expected = String::new();
    let mut index = 0;
    for bits in [8, 16, 32, 64] {
        for signed in [true, false] {
            let ty = format!("{}{bits}", if signed { 'i' } else { 'u' });
            let min = if signed { -(1i128 << (bits - 1)) } else { 0 };
            let max = (1i128 << (bits - u32::from(signed))) - 1;
            writeln!(
                source,
                "*identity_{ty}(value as {ty}) returns {ty}\n    value\n"
            )
            .unwrap();
            writeln!(source, "*maximum_{ty}() returns {ty}\n    {max}\n").unwrap();
            writeln!(body, "    log(maximum_{ty}())").unwrap();
            writeln!(expected, "{max}").unwrap();
            for value in [min, 0, max] {
                let magnitude = value.unsigned_abs();
                let sign = if value < 0 { "-" } else { "" };
                for literal in [
                    format!("{sign}{magnitude}"),
                    format!("{sign}0x{magnitude:x}"),
                    format!("{sign}0o{magnitude:o}"),
                    format!("{sign}0b{magnitude:b}"),
                ] {
                    writeln!(body, "    value{index} as {ty} is {literal}\n    log(value{index})\n    log(identity_{ty}({literal}))").unwrap();
                    writeln!(expected, "{value}\n{value}").unwrap();
                    index += 1;
                }
            }
        }
    }
    source.push_str(&body);
    expect(&source, &expected);
    let formatted = jinnc::fmt::format_source(&source).unwrap();
    assert!(!formatted.contains("--9223372036854775808"));
    assert_eq!(jinnc::fmt::format_source(&formatted).unwrap(), formatted);
    expect(&formatted, &expected);
}

fn expect_literal_rejection(source: &str, fragments: &[&str]) {
    let (_dir, _binary, output) = compile(source, "0");
    assert!(!output.status.success(), "accepted {source}");
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    assert!(
        !diagnostic.contains("panicked") && !diagnostic.contains("ICE:"),
        "{diagnostic}"
    );
    for fragment in fragments {
        assert!(
            diagnostic.contains(fragment),
            "missing {fragment}: {diagnostic}"
        );
    }
}

#[test]
fn integer_literal_range_checks_use_mathematical_values() {
    for bits in [8, 16, 32, 64] {
        for signed in [true, false] {
            let ty = format!("{}{bits}", if signed { 'i' } else { 'u' });
            let min = if signed { -(1i128 << (bits - 1)) } else { 0 };
            let max = (1i128 << (bits - u32::from(signed))) - 1;
            for value in [min - 1, max + 1] {
                if value > u64::MAX as i128 {
                    continue;
                }
                let range = format!("{min}..{max}");
                let value = value.to_string();
                let source = format!("*main\n    value as {ty} is {value}\n    log(value)\n");
                expect_literal_rejection(&source, &[&value, &ty, &range]);
                let source = format!(
                    "*accept(value as {ty})\n    log(value)\n\n*main\n    accept({value})\n"
                );
                expect_literal_rejection(&source, &[&value, &ty, &range]);
            }
        }
    }
    for literal in [
        "9223372036854775808",
        "18446744073709551615",
        "0xffffffffffffffff",
    ] {
        expect_literal_rejection(
            &format!("*main\n    log({literal})\n"),
            &["does not fit in `i64`"],
        );
    }
    for literal in ["18446744073709551616", "0x10000000000000000"] {
        expect_literal_rejection(&format!("*main\n    log({literal})\n"), &["bad"]);
    }
    expect_literal_rejection(
        "*main\n    log((-9223372036854775809) as u64)\n",
        &["-9223372036854775809", "i64"],
    );
}

#[test]
fn literal_casts_preserve_unsigned_sources_and_wrap_explicitly() {
    expect(
        "*main\n    log(18446744073709551615 as u64)\n    log(18446744073709551615 as i64)\n    log(18446744073709551615 as u8)\n    log(18446744073709551615 as i8)\n    log(9223372036854775808 as i64)\n    log(300 as i8)\n    log((-1) as u64)\n    log((-9223372036854775808) as i64)\n    log(18446744073709551615 as strict u64)\n",
        "18446744073709551615\n-1\n255\n-1\n-9223372036854775808\n44\n18446744073709551615\n-9223372036854775808\n18446744073709551615\n",
    );
}

#[test]
fn literal_boundaries_propagate_through_inference_and_construction() {
    expect(
        "type Bounds\n    low as i64\n    high as u64\n\n*identity(value)\n    value\n\n*accept(value as u64)\n    log(value)\n\n*main\n    bounds is Bounds(low is -9223372036854775808, high is 18446744073709551615)\n    log(bounds.low)\n    log(bounds.high)\n    pending is 18446744073709551615\n    accept(pending)\n    log(identity(18446744073709551615 as u64))\n    values is [18446744073709551615 as u64, 18446744073709551615]\n    log(values[0])\n    log(values[1])\n",
        "-9223372036854775808\n18446744073709551615\n18446744073709551615\n18446744073709551615\n18446744073709551615\n18446744073709551615\n",
    );
    expect_literal_rejection(
        "type Bounds\n    high as u8\n\n*main\n    bounds is Bounds(high is 256)\n    log(bounds.high)\n",
        &["256", "u8", "0..255"],
    );
    expect_literal_rejection(
        "*main\n    values is [0 as u8, 256]\n    log(values[1])\n",
        &["256", "u8", "0..255"],
    );
}

#[test]
fn large_integer_float_adaptation_and_casts_preserve_sign() {
    expect(
        "*main\n    direct as f64 is 18446744073709551615\n    negative as f64 is -9223372036854775808\n    unsigned as u64 is 18446744073709551615\n    log(direct > 0.0)\n    log(negative < 0.0)\n    log((18446744073709551615 as f64) equals direct)\n    log((18446744073709551615 as f32) > 0.0)\n    log((unsigned as f64) equals direct)\n    log((unsigned as f32) > 0.0)\n    log(18446744073709551615 + 0.0 > 0.0)\n    log(0.0 + 18446744073709551615 > 0.0)\n",
        "true\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\n",
    );
}

#[test]
fn signed_minimum_negation_keeps_used_and_unused_traps() {
    for bits in [8, 16, 32, 64] {
        let min = -(1i128 << (bits - 1));
        for statement in ["log(-value)", "unused is -value"] {
            expect_trap(
                &format!(
                    "*main\n    value as i{bits} is {min}\n    {statement}\n    log('continued')\n"
                ),
                "integer overflow",
            );
        }
    }
    expect_trap(
        "*main\n    unused is -(-9223372036854775808)\n    log('continued')\n",
        "integer overflow",
    );
}

#[test]
fn formatter_write_preserves_extreme_literal_signs() {
    let source = "*main\n    minimum    is    -9223372036854775808\n    maximum as u64 is 18446744073709551615\n    log(minimum)\n    log(maximum)\n";
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("boundary.jn");
    std::fs::write(&path, source).unwrap();
    for _ in 0..2 {
        let output = Command::new(env!("CARGO_BIN_EXE_jinn"))
            .args(["fmt", "--write"])
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let formatted = std::fs::read_to_string(&path).unwrap();
        assert!(!formatted.contains("--9223372036854775808"));
        expect(&formatted, "-9223372036854775808\n18446744073709551615\n");
    }
}

#[test]
fn c_compatibility_limits_keep_unsigned_maximum_values() {
    let source = format!(
        "{}\n{}\n*main\n    log(LONG_MIN)\n    log(ULONG_MAX)\n    log(ULLONG_MAX)\n    log(UINT64_MAX)\n    log(SIZE_MAX)\n",
        include_str!("../libjn/limits.jn"),
        include_str!("../libjn/stdint.jn")
            .lines()
            .filter(|line| line.starts_with(|c: char| c.is_ascii_uppercase()))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    expect(
        &source,
        "-9223372036854775808\n18446744073709551615\n18446744073709551615\n18446744073709551615\n18446744073709551615\n",
    );
}

#[test]
fn declaration_integer_domains_reject_oversized_magnitudes() {
    expect_literal_rejection(
        "enum Tag\n    Large is 18446744073709551615\n\n*main\n    log(1)\n",
        &["enum discriminant does not fit in i64"],
    );
    expect_literal_rejection(
        "migration 'large' version 18446744073709551615\n    up\n        alter items\n            add value as i64\n\n*main\n    log(1)\n",
        &["version number does not fit in i64"],
    );
    expect_literal_rejection(
        "*maximum() returns i8\n    128\n\n*main\n    log(maximum())\n",
        &["128", "i8", "-128..127"],
    );
}

fn strict_cases(cases: &[(String, String, String, Option<String>)]) {
    use std::io::Write as _;
    use std::process::Stdio;
    let mut source = String::from(
        "extern *getchar() returns i32\n\n*main\n    choice is extern.getchar() * 256 + extern.getchar()\n",
    );
    for (index, (value, from, to, _)) in cases.iter().enumerate() {
        let output = if to.starts_with('f') {
            format!("result equals (value as {to})")
        } else {
            "result".into()
        };
        writeln!(source, "    if choice equals {index}\n        value is ({value}) as {from}\n        result is value as strict {to}\n        log({output})\n        return").unwrap();
    }
    for opt in ["0", "3"] {
        let (dir, binary, compiled) = compile(&source, opt);
        assert!(
            compiled.status.success(),
            "{}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        for (index, (value, from, to, expected)) in cases.iter().enumerate() {
            let mut child = Command::new(&binary)
                .current_dir(dir.path())
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(&[(index / 256) as u8, index as u8])
                .unwrap();
            let output = child.wait_with_output().unwrap();
            let context = format!("opt {opt}: {value} as {from} as strict {to}");
            if let Some(expected) = expected {
                assert!(
                    output.status.success(),
                    "{context}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                assert_eq!(
                    String::from_utf8_lossy(&output.stdout).trim(),
                    expected,
                    "{context}"
                );
            } else {
                assert!(!output.status.success(), "{context}: did not trap");
                assert!(
                    String::from_utf8_lossy(&output.stderr)
                        .contains("strict cast lost information"),
                    "{context}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                assert!(output.stdout.is_empty(), "{context}: continued after trap");
            }
        }
    }
}

#[test]
fn strict_integer_casts_check_all_mathematical_domain_pairs() {
    let mut cases = Vec::new();
    for bits in [8, 16, 32, 64] {
        for signed in [false, true] {
            let from = format!("{}{bits}", if signed { 'i' } else { 'u' });
            let low = if signed { -(1i128 << (bits - 1)) } else { 0 };
            let high = (1i128 << (bits - u32::from(signed))) - 1;
            for target_bits in [8, 16, 32, 64] {
                for target_signed in [false, true] {
                    let to = format!("{}{target_bits}", if target_signed { 'i' } else { 'u' });
                    let target_low = if target_signed {
                        -(1i128 << (target_bits - 1))
                    } else {
                        0
                    };
                    let target_high = (1i128 << (target_bits - u32::from(target_signed))) - 1;
                    let mut values = vec![low, low + 1, -1, 0, 1, high - 1, high];
                    for edge in [target_low, target_high] {
                        values.extend([edge - 1, edge, edge + 1]);
                    }
                    values.sort_unstable();
                    values.dedup();
                    for value in values.into_iter().filter(|v| (low..=high).contains(v)) {
                        let expected = (target_low..=target_high)
                            .contains(&value)
                            .then(|| value.to_string());
                        cases.push((value.to_string(), from.clone(), to.clone(), expected));
                    }
                }
            }
        }
    }
    strict_cases(&cases);
}

#[test]
fn strict_float_integer_casts_require_finite_integral_in_range_values() {
    let mut cases = Vec::new();
    for from in ["f32", "f64"] {
        for bits in [8, 16, 32, 64] {
            for signed in [false, true] {
                let to = format!("{}{bits}", if signed { 'i' } else { 'u' });
                let high = 2f64.powi(bits - i32::from(signed));
                let low = if signed { -high } else { 0.0 };
                let mut values = vec![
                    0.0,
                    -0.0,
                    1.0,
                    -1.0,
                    1.5,
                    -1.5,
                    high,
                    low,
                    f64::NAN,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                ];
                for edge in [high, low] {
                    if from == "f32" {
                        let edge = edge as f32;
                        values.extend([
                            f32::from_bits(edge.to_bits().saturating_sub(1)) as f64,
                            f32::from_bits(edge.to_bits() + 1) as f64,
                        ]);
                    } else {
                        values.extend([
                            f64::from_bits(edge.to_bits().saturating_sub(1)),
                            f64::from_bits(edge.to_bits() + 1),
                        ]);
                    }
                }
                for value in values {
                    let value = if from == "f32" {
                        value as f32 as f64
                    } else {
                        value
                    };
                    let expression = if value.is_nan() {
                        "0.0 / 0.0".into()
                    } else if value == f64::INFINITY {
                        "1.0 / 0.0".into()
                    } else if value == f64::NEG_INFINITY {
                        "-1.0 / 0.0".into()
                    } else {
                        format!("{value:?}")
                    };
                    let expected =
                        (value.is_finite() && value.fract() == 0.0 && value >= low && value < high)
                            .then(|| (value as i128).to_string());
                    cases.push((expression, from.into(), to.clone(), expected));
                }
            }
        }
    }
    strict_cases(&cases);
}

#[test]
fn strict_integer_float_casts_reject_rounding_and_unsigned_boundary_aliases() {
    let mut cases = Vec::new();
    for bits in [8, 16, 32, 64] {
        for signed in [false, true] {
            let from = format!("{}{bits}", if signed { 'i' } else { 'u' });
            let low = if signed { -(1i128 << (bits - 1)) } else { 0 };
            let high = (1i128 << (bits - u32::from(signed))) - 1;
            let mut values = vec![low, low + 1, -1, 0, 1, high - 1, high];
            for precision in [24, 53] {
                let edge = 1i128 << precision;
                values.extend([edge - 1, edge, edge + 1, -edge - 1, -edge, -edge + 1]);
            }
            for to in ["f32", "f64"] {
                for value in values.iter().copied().filter(|v| (low..=high).contains(v)) {
                    let rounded = if to == "f32" {
                        value as f32 as f64
                    } else {
                        value as f64
                    };
                    let expected = ((rounded as i128) == value).then(|| "true".into());
                    cases.push((value.to_string(), from.clone(), to.into(), expected));
                }
            }
        }
    }
    strict_cases(&cases);
}

#[test]
fn strict_float_casts_check_exactness_even_at_equal_width() {
    let mut cases = Vec::new();
    for from in ["f32", "f64"] {
        for to in ["f32", "f64"] {
            for value in [
                0.0,
                -0.0,
                1.5,
                -1.5,
                0.1,
                f32::MAX as f64,
                f64::MAX,
                f64::MIN_POSITIVE,
                f64::NAN,
                f64::INFINITY,
                f64::NEG_INFINITY,
            ] {
                let value = if from == "f32" {
                    value as f32 as f64
                } else {
                    value
                };
                let expression = if value.is_nan() {
                    "0.0 / 0.0".into()
                } else if value == f64::INFINITY {
                    "1.0 / 0.0".into()
                } else if value == f64::NEG_INFINITY {
                    "-1.0 / 0.0".into()
                } else {
                    format!("{value:?}")
                };
                let exact = value.is_finite() && (to == "f64" || value as f32 as f64 == value);
                cases.push((
                    expression,
                    from.into(),
                    to.into(),
                    exact.then(|| "true".into()),
                ));
            }
        }
    }
    strict_cases(&cases);
}

#[test]
fn strict_casts_reject_nonnumeric_domains_and_preserve_unused_traps() {
    for expression in [
        "true as strict i64",
        "'hello' as strict i64",
        "1 as strict bool",
        "1 as strict string",
    ] {
        let (_, _, output) = compile(&format!("*main\n    log({expression})\n"), "0");
        assert!(!output.status.success(), "{expression}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("strict cast requires numeric"));
    }
    for (value, from, to) in [
        ("-1", "i64", "u64"),
        ("18446744073709551615", "u64", "i64"),
        ("1.5", "f64", "i64"),
        ("16777217", "i64", "f32"),
    ] {
        expect_trap(
            &format!(
                "*main\n    value is ({value}) as {from}\n    unused is value as strict {to}\n    log(999)\n"
            ),
            "strict cast lost information",
        );
    }
}
