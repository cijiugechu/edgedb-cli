use std::error::Error;
use std::fmt;
use std::sync::Arc;

use crate::prompt;
use crate::prompt::variable::{self, VariableInput};
use crate::repl;
use edgedb_protocol::codec;
use edgedb_protocol::descriptors::{Descriptor, Typedesc};
use edgedb_protocol::value::Value;

#[derive(Debug)]
pub struct Canceled;

pub async fn input_variables(
    desc: &Typedesc,
    state: &mut repl::PromptRpc,
) -> Result<Value, anyhow::Error> {
    // only for protocol < 0.12
    if desc.is_empty_tuple() {
        return Ok(Value::Tuple(Vec::new()));
    }
    match desc.root() {
        Some(Descriptor::Tuple(tuple)) if desc.proto().is_at_most(0, 11) => {
            let mut val = Vec::with_capacity(tuple.element_types.len());
            for (idx, el) in tuple.element_types.iter().enumerate() {
                val.push(
                    input_item(&format!("{}", idx), desc.get(*el)?, desc, state, false)
                        .await?
                        .expect("no optional"),
                );
            }
            Ok(Value::Tuple(val))
        }
        Some(Descriptor::NamedTuple(tuple)) if desc.proto().is_at_most(0, 11) => {
            let mut fields = Vec::with_capacity(tuple.elements.len());
            let shape = tuple.elements[..].into();
            for el in tuple.elements.iter() {
                fields.push(
                    input_item(&el.name, desc.get(el.type_pos)?, desc, state, false)
                        .await?
                        .expect("no optional"),
                );
            }
            Ok(Value::NamedTuple { shape, fields })
        }
        Some(Descriptor::ObjectShape(obj)) if desc.proto().is_at_least(0, 12) => {
            let mut fields = Vec::with_capacity(obj.elements.len());
            let shape = obj.elements[..].into();
            for el in obj.elements.iter() {
                let optional = el.cardinality.map(|c| c.is_optional()).unwrap_or(false);
                fields.push(
                    input_item(&el.name, desc.get(el.type_pos)?, desc, state, optional).await?,
                );
            }
            Ok(Value::Object { shape, fields })
        }
        Some(root) => Err(anyhow::anyhow!("Unknown input type descriptor: {:?}", root)),
        // Since protocol 0.12
        None => Ok(Value::Nothing),
    }
}

async fn input_item(
    name: &str,
    mut item: &Descriptor,
    all: &Typedesc,
    state: &mut repl::PromptRpc,
    optional: bool,
) -> Result<Option<Value>, anyhow::Error> {
    if let Descriptor::Scalar(s) = item {
        item = all.get(s.base_type_pos)?;
    }
    match item {
        Descriptor::BaseScalar(s) => {
            let var_type: Arc<dyn VariableInput> = match *s.id {
                codec::STD_STR => Arc::new(variable::Str),
                codec::STD_UUID => Arc::new(variable::Uuid),
                codec::STD_INT16 => Arc::new(variable::Int16),
                codec::STD_INT32 => Arc::new(variable::Int32),
                codec::STD_INT64 => Arc::new(variable::Int64),
                codec::STD_FLOAT32 => Arc::new(variable::Float32),
                codec::STD_FLOAT64 => Arc::new(variable::Float64),
                codec::STD_DECIMAL => Arc::new(variable::Decimal),
                codec::STD_BOOL => Arc::new(variable::Bool),
                codec::STD_JSON => Arc::new(variable::Json),
                codec::STD_BIGINT => Arc::new(variable::BigInt),
                _ => return Err(anyhow::anyhow!("Unimplemented input type {}", *s.id)),
            };

            let val = match state.variable_input(name, var_type, optional, "").await? {
                prompt::VarInput::Value(val) => Some(val),
                prompt::VarInput::Interrupt => Err(Canceled)?,
                prompt::VarInput::Eof => None,
            };
            Ok(val)
        }
        _ => Err(anyhow::anyhow!(
            "Unimplemented input type descriptor: {:?}",
            item
        )),
    }
}

impl Error for Canceled {}

impl fmt::Display for Canceled {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        "Operation canceled".fmt(f)
    }
}
