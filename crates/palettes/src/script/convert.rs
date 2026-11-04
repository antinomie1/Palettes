//! Markup values to Luau values and back, by markup type.

use crate::markup::link::fields;
use aegle_loader::{Data, markup::Type};
use mlua::{Lua, Table, Value};
use std::rc::Rc;

/// The value a state of type `ty` holds when a script leaves a field out.
pub(crate) fn default(ty: &Type) -> Data {
    match ty {
        Type::Bool => Data::Bool(false),
        Type::Int => Data::Int(0),
        Type::Float => Data::Float(0.0),
        Type::String => Data::String("".into()),
        Type::List(_) => Data::List(Rc::from([])),
        Type::Record(name) => Data::Record(
            fields(name)
                .map(|f| f.iter().map(|(_, ty)| default(ty)).collect())
                .unwrap_or_else(|| Rc::from([])),
        ),
    }
}

fn kind(value: &Value) -> &'static str {
    value.type_name()
}

fn number(value: &Value) -> Option<f64> {
    match value {
        Value::Integer(n) => Some(*n as f64),
        Value::Number(n) => Some(*n),
        _ => None,
    }
}

/// Converts a Luau value to markup type `ty`; nil is the type's default.
pub(crate) fn to_data(value: &Value, ty: &Type) -> Result<Data, String> {
    let mismatch = || format!("expected {}, got {}", name(ty), kind(value));
    if value.is_nil() {
        return Ok(default(ty));
    }
    Ok(match ty {
        Type::Bool => Data::Bool(value.as_boolean().ok_or_else(mismatch)?),
        Type::Int => {
            let n = number(value).ok_or_else(mismatch)?;
            if !n.is_finite() {
                return Err(mismatch());
            }
            Data::Int(n.round() as i64)
        }
        Type::Float => {
            let n = number(value).ok_or_else(mismatch)? as f32;
            if !n.is_finite() {
                return Err(mismatch());
            }
            Data::Float(n)
        }
        Type::String => match value {
            Value::String(s) => Data::String(s.to_string_lossy().into()),
            Value::Integer(_) | Value::Number(_) => {
                Data::String(value.to_string().map_err(|e| e.to_string())?.into())
            }
            _ => return Err(mismatch()),
        },
        Type::List(item) => {
            let table = value.as_table().ok_or_else(mismatch)?;
            let items = table
                .sequence_values::<Value>()
                .map(|v| to_data(&v.map_err(|e| e.to_string())?, item))
                .collect::<Result<Vec<_>, _>>()?;
            Data::List(items.into())
        }
        Type::Record(record) => {
            let table = value.as_table().ok_or_else(mismatch)?;
            let fields = fields(record).ok_or_else(|| format!("unknown record `{record}`"))?;
            let values = fields
                .iter()
                .map(|(field, ty)| {
                    let v: Value = table.get(field.as_str()).map_err(|e| e.to_string())?;
                    to_data(&v, ty).map_err(|e| format!("{field}: {e}"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            Data::Record(values.into())
        }
    })
}

/// Converts a markup value of type `ty` to Luau; records become tables by
/// field name.
pub(crate) fn to_lua(lua: &Lua, data: &Data, ty: &Type) -> mlua::Result<Value> {
    Ok(match (data, ty) {
        (Data::Bool(b), _) => Value::Boolean(*b),
        (Data::Int(n), _) => Value::Number(*n as f64),
        (Data::Float(n), _) => Value::Number(f64::from(*n)),
        (Data::String(s), _) => Value::String(lua.create_string(s.as_bytes())?),
        (Data::List(items), Type::List(item)) => {
            let table = lua.create_table_with_capacity(items.len(), 0)?;
            for value in items.iter() {
                table.raw_push(to_lua(lua, value, item)?)?;
            }
            Value::Table(table)
        }
        (Data::Record(values), Type::Record(record)) => {
            let table: Table = lua.create_table()?;
            for ((field, ty), value) in fields(record)
                .iter()
                .flat_map(|f| f.iter())
                .zip(values.iter())
            {
                table.raw_set(field.as_str(), to_lua(lua, value, ty)?)?;
            }
            Value::Table(table)
        }
        _ => Value::Nil,
    })
}

/// A Luau value guessed from command-line text: a number or boolean when it
/// reads as one, else the text.
pub(crate) fn guess(lua: &Lua, text: &str) -> mlua::Result<Value> {
    Ok(match text {
        "true" => Value::Boolean(true),
        "false" => Value::Boolean(false),
        _ => match text.parse::<f64>() {
            Ok(n) if n.is_finite() => Value::Number(n),
            _ => Value::String(lua.create_string(text)?),
        },
    })
}

/// Parses command-line text as markup type `ty`.
pub(crate) fn parse(text: &str, ty: &Type) -> Result<Data, String> {
    let bad = || format!("`{text}` is not {}", name(ty));
    Ok(match ty {
        Type::Bool => Data::Bool(text.parse().map_err(|_| bad())?),
        Type::Int => Data::Int(text.parse().map_err(|_| bad())?),
        Type::Float => Data::Float(
            text.parse::<f32>()
                .ok()
                .filter(|n| n.is_finite())
                .ok_or_else(bad)?,
        ),
        Type::String => Data::String(text.into()),
        Type::List(_) | Type::Record(_) => return Err(bad()),
    })
}

fn name(ty: &Type) -> String {
    match ty {
        Type::Bool => "bool".into(),
        Type::Int => "int".into(),
        Type::Float => "float".into(),
        Type::String => "string".into(),
        Type::List(item) => format!("list<{}>", name(item)),
        Type::Record(record) => record.clone(),
    }
}
