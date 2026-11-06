//! Reuse immutable programs while active computations retain them.

use super::{MAX_EXPRESSION, Program, Type};
use anyhow::{Result, ensure};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
};

#[derive(PartialEq, Eq, Hash)]
struct Signature {
    expression: String,
    inputs: Vec<(String, Type)>,
    captured: Vec<bool>,
}

#[derive(Default)]
pub(crate) struct Programs(Mutex<HashMap<Signature, Weak<Program>>>);

impl Programs {
    pub fn parse(
        &self,
        expression: &str,
        inputs: Vec<(String, Type)>,
        captured: Vec<bool>,
    ) -> Result<Arc<Program>> {
        ensure!(
            expression.len() <= MAX_EXPRESSION,
            "Formula is limited to {MAX_EXPRESSION} bytes."
        );
        let signature = Signature {
            expression: expression.into(),
            inputs,
            captured,
        };
        let mut programs = self.0.lock().expect("formula program cache poisoned");
        if let Some(program) = programs.get(&signature).and_then(Weak::upgrade) {
            return Ok(program);
        }
        programs.retain(|_, program| program.strong_count() > 0);
        let program = Arc::new(Program::parse_captured(
            expression,
            &signature.inputs,
            &signature.captured,
        )?);
        programs.insert(signature, Arc::downgrade(&program));
        Ok(program)
    }
}
