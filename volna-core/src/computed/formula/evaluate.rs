use super::{Binary, Node, TypedNode, Unary, Value};
use anyhow::{Result, bail};

pub(super) fn unary(operator: Unary, value: Value) -> Value {
    match operator {
        Unary::Negate => Value::finite(value.real().map(|v| -v)),
        Unary::Positive | Unary::Real => Value::Real(value.real()),
        Unary::Not => Value::Boolean(value.boolean().map(|v| !v)),
        Unary::Abs => Value::finite(value.real().map(f64::abs)),
        Unary::Boolean => Value::Boolean(value.boolean()),
    }
}

pub(super) fn binary(operator: Binary, a: Value, b: Value) -> Value {
    match operator {
        Binary::And => Value::Boolean(match (a.boolean(), b.boolean()) {
            (Some(false), _) | (_, Some(false)) => Some(false),
            (Some(true), Some(true)) => Some(true),
            _ => None,
        }),
        Binary::Or => Value::Boolean(match (a.boolean(), b.boolean()) {
            (Some(true), _) | (_, Some(true)) => Some(true),
            (Some(false), Some(false)) => Some(false),
            _ => None,
        }),
        Binary::Equal | Binary::NotEqual => {
            let equal = match (a, b) {
                (Value::Boolean(a), Value::Boolean(b)) => a.zip(b).map(|(a, b)| a == b),
                _ => a.real().zip(b.real()).map(|(a, b)| a == b),
            };
            Value::Boolean(equal.map(|v| if operator == Binary::Equal { v } else { !v }))
        }
        Binary::Less | Binary::LessEqual | Binary::Greater | Binary::GreaterEqual => {
            Value::Boolean(a.real().zip(b.real()).map(|(a, b)| match operator {
                Binary::Less => a < b,
                Binary::LessEqual => a <= b,
                Binary::Greater => a > b,
                _ => a >= b,
            }))
        }
        Binary::Min | Binary::Max => Value::Real(a.real().zip(b.real()).map(|(a, b)| {
            if a == b {
                if a == 0.0 {
                    let negative = match operator {
                        Binary::Min => a.is_sign_negative() || b.is_sign_negative(),
                        _ => a.is_sign_negative() && b.is_sign_negative(),
                    };
                    if negative { -0.0 } else { 0.0 }
                } else {
                    a
                }
            } else if (operator == Binary::Min && a < b) || (operator == Binary::Max && a > b) {
                a
            } else {
                b
            }
        })),
        _ => Value::finite(a.real().zip(b.real()).and_then(|(a, b)| {
            Some(match operator {
                Binary::Add => a + b,
                Binary::Subtract => a - b,
                Binary::Multiply => a * b,
                Binary::Divide if b != 0.0 => a / b,
                Binary::Remainder if b != 0.0 => a % b,
                _ => return None,
            })
        })),
    }
}

pub(super) fn constant(nodes: &[TypedNode], index: usize) -> Result<f64> {
    let Some(value) = constant_value(nodes, index)?
        .real()
        .filter(|value| value.is_finite())
    else {
        bail!("Generator and initial-value arguments must be finite Real values.");
    };
    Ok(value)
}

pub(super) fn constant_value(nodes: &[TypedNode], index: usize) -> Result<Value> {
    Ok(match &nodes[index].node {
        Node::Number(value) => Value::Real(Some(f64::from_bits(*value))),
        Node::Boolean(value) => Value::Boolean(Some(*value)),
        Node::Unary(operator, a) => unary(*operator, constant_value(nodes, *a)?),
        Node::Binary(operator, a, b) => binary(
            *operator,
            constant_value(nodes, *a)?,
            constant_value(nodes, *b)?,
        ),
        _ => bail!("Expected a literal expression."),
    })
}
