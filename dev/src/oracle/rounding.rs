//! Recalculate retained SUM and AVG answers with exact rational arithmetic.
//!
//! SUM rounds each addition to 53 significant bits, with subnormal spacing and
//! an unrestricted exponent until the final range check. AVG uses ordinary
//! binary64 arithmetic while finite, otherwise a rational error interval bounded
//! by its inputs. The retained input sequences preserve the original generator's
//! counterexamples without depending on its random-number implementation.

use crate::Result;
use num_bigint::BigInt;
use num_rational::BigRational as Rational;
use num_traits::{One, Signed, ToPrimitive, Zero};

fn exact(value: f64) -> Rational {
    Rational::from_float(value).expect("finite model input")
}
fn power(exponent: i64) -> Rational {
    if exponent >= 0 {
        Rational::from_integer(BigInt::one() << exponent as usize)
    } else {
        Rational::new(BigInt::one(), BigInt::one() << (-exponent) as usize)
    }
}
fn rounded(value: Rational) -> Rational {
    if value.is_zero() {
        return value;
    }
    let magnitude = value.abs();
    let mut exponent = magnitude.numer().bits() as i64 - magnitude.denom().bits() as i64;
    if magnitude < power(exponent) {
        exponent -= 1;
    }
    let unit = power((exponent - 52).max(-1074));
    let scaled = magnitude / &unit;
    let mut quotient = scaled.numer() / scaled.denom();
    let remainder = scaled.numer() % scaled.denom();
    let twice = remainder * 2;
    if twice > *scaled.denom() || (twice == *scaled.denom() && (&quotient % 2_u8) != BigInt::zero())
    {
        quotient += 1;
    }
    let result = Rational::from_integer(quotient) * unit;
    if value.is_negative() { -result } else { result }
}
fn bits(value: f64) -> String {
    format!("{:016x}", value.to_bits())
}
fn sum(values: &[f64]) -> String {
    if values.iter().any(|v| v.is_nan()) {
        return "nan".into();
    }
    let positive = values.contains(&f64::INFINITY);
    let negative = values.contains(&f64::NEG_INFINITY);
    if positive || negative {
        return if positive && negative {
            "nan".into()
        } else {
            bits(if negative {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            })
        };
    }
    let mut total = exact(values[0]);
    let mut negative_zero = values[0].to_bits() == (-0.0_f64).to_bits();
    for &value in &values[1..] {
        negative_zero =
            total.is_zero() && value == 0.0 && negative_zero && value.is_sign_negative();
        total = rounded(total + exact(value));
    }
    if total.abs() > exact(f64::MAX) {
        "overflow".into()
    } else if total.is_zero() && negative_zero {
        bits(-0.0)
    } else {
        bits(total.to_f64().expect("bounded finite sum"))
    }
}
fn average(values: &[f64]) -> String {
    if values.iter().any(|v| !v.is_finite()) {
        let value = sum(values);
        return format!("{value},{value}");
    }
    let mut total = values[0];
    for &value in &values[1..] {
        total += value;
        if !total.is_finite() {
            break;
        }
    }
    if total.is_finite() {
        let value = bits(total / values.len() as f64);
        return format!("{value},{value}");
    }
    let inputs: Vec<_> = values.iter().map(|&v| exact(v)).collect();
    let mean = inputs.iter().cloned().sum::<Rational>() / BigInt::from(values.len());
    let largest = inputs.iter().map(Signed::abs).max().unwrap();
    let error = Rational::from_integer(BigInt::from(8 * values.len())) * power(-52) * largest;
    let lower = inputs.iter().min().unwrap().clone().max(&mean - &error);
    let upper = inputs.iter().max().unwrap().clone().min(&mean + &error);
    let mut lo = lower.to_f64().expect("bounded mean lower endpoint");
    let mut hi = upper.to_f64().expect("bounded mean upper endpoint");
    if exact(lo) > lower {
        lo = lo.next_down();
    }
    if exact(hi) < upper {
        hi = hi.next_up();
    }
    format!("{},{}", bits(lo), bits(hi))
}

pub fn check(text: &str) -> Result<usize> {
    let mut count = 0;
    for (index, line) in text.lines().enumerate() {
        let fields: Vec<_> = line.split(';').collect();
        if fields.len() != 3 {
            return Err(format!("rounding record {} has wrong fields", index + 1).into());
        }
        let values: Vec<_> = fields[0]
            .split(',')
            .map(|raw| {
                if raw.len() != 16 || !raw.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err("invalid rounding input bits".into());
                }
                Ok(f64::from_bits(u64::from_str_radix(raw, 16)?))
            })
            .collect::<Result<_>>()?;
        if values.is_empty() || values.len() > 36 {
            return Err("rounding input count exceeds retained profile".into());
        }
        if sum(&values) != fields[1] || average(&values) != fields[2] {
            return Err(format!("rounding record {} differs from exact model", index + 1).into());
        }
        count += 1;
    }
    if count == 0 {
        return Err("no rounding inputs".into());
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rounding_distinguishes_ties_signs_and_changed_answers() {
        assert_eq!(rounded(exact(1.0) + power(-53)), exact(1.0));
        assert_eq!(
            rounded(exact(1.0) + power(-52) + power(-53)),
            exact(1.0) + power(-51)
        );
        assert_eq!(sum(&[-0.0, -0.0]), "8000000000000000");
        assert_eq!(sum(&[-0.0, 0.0]), "0000000000000000");
        assert_eq!(sum(&[f64::MAX, f64::MAX, -f64::MAX]), bits(f64::MAX));
        let valid = "3ff0000000000000;3ff0000000000000;3ff0000000000000,3ff0000000000000\n";
        assert_eq!(check(valid).unwrap(), 1);
        assert!(check(&valid.replacen(";3ff0000000000000;", ";0000000000000000;", 1)).is_err());
        assert!(check("").is_err());
    }
}
