use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

/// A deliberately small, strict subset of JSON Schema used at the tool boundary.
///
/// Object schemas never permit undeclared properties. Keeping this invariant in
/// the type makes it impossible to accidentally register a permissive tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JsonSchema {
    Null,
    Boolean,
    Integer {
        minimum: Option<i64>,
        maximum: Option<i64>,
    },
    Number {
        minimum: Option<f64>,
        maximum: Option<f64>,
    },
    String {
        min_length: Option<usize>,
        max_length: Option<usize>,
        #[serde(default)]
        enum_values: Vec<String>,
    },
    Array {
        items: Box<JsonSchema>,
        min_items: Option<usize>,
        max_items: Option<usize>,
    },
    Object {
        properties: BTreeMap<String, JsonSchema>,
        #[serde(default)]
        required: BTreeSet<String>,
    },
}

impl JsonSchema {
    pub fn string() -> Self {
        Self::String {
            min_length: None,
            max_length: None,
            enum_values: Vec::new(),
        }
    }

    pub fn object(
        properties: impl IntoIterator<Item = (impl Into<String>, JsonSchema)>,
        required: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self::Object {
            properties: properties
                .into_iter()
                .map(|(name, schema)| (name.into(), schema))
                .collect(),
            required: required.into_iter().map(Into::into).collect(),
        }
    }

    pub fn validate_definition(&self) -> Result<(), SchemaDefinitionError> {
        self.validate_definition_at("$")
    }

    fn validate_definition_at(&self, path: &str) -> Result<(), SchemaDefinitionError> {
        match self {
            Self::Integer { minimum, maximum } => {
                if minimum.zip(*maximum).is_some_and(|(min, max)| min > max) {
                    return Err(SchemaDefinitionError::InvalidBounds(path.into()));
                }
            }
            Self::Number { minimum, maximum } => {
                if minimum.is_some_and(|value| !value.is_finite())
                    || maximum.is_some_and(|value| !value.is_finite())
                    || minimum.zip(*maximum).is_some_and(|(min, max)| min > max)
                {
                    return Err(SchemaDefinitionError::InvalidBounds(path.into()));
                }
            }
            Self::String {
                min_length,
                max_length,
                enum_values,
            } => {
                if min_length
                    .zip(*max_length)
                    .is_some_and(|(min, max)| min > max)
                {
                    return Err(SchemaDefinitionError::InvalidBounds(path.into()));
                }
                let unique: BTreeSet<_> = enum_values.iter().collect();
                if unique.len() != enum_values.len() {
                    return Err(SchemaDefinitionError::DuplicateEnumValue(path.into()));
                }
            }
            Self::Array {
                items,
                min_items,
                max_items,
            } => {
                if min_items
                    .zip(*max_items)
                    .is_some_and(|(min, max)| min > max)
                {
                    return Err(SchemaDefinitionError::InvalidBounds(path.into()));
                }
                items.validate_definition_at(&format!("{path}.items"))?;
            }
            Self::Object {
                properties,
                required,
            } => {
                if let Some(name) = required
                    .iter()
                    .find(|name| !properties.contains_key(name.as_str()))
                {
                    return Err(SchemaDefinitionError::UnknownRequiredProperty {
                        path: path.into(),
                        property: name.clone(),
                    });
                }
                for (name, schema) in properties {
                    if name.is_empty() || name.chars().any(char::is_control) {
                        return Err(SchemaDefinitionError::InvalidPropertyName {
                            path: path.into(),
                            property: name.clone(),
                        });
                    }
                    schema.validate_definition_at(&format!("{path}.{}", escape_path(name)))?;
                }
            }
            Self::Null | Self::Boolean => {}
        }
        Ok(())
    }

    pub fn validate(&self, value: &Value) -> Result<(), SchemaViolation> {
        self.validate_at(value, "$")
    }

    fn validate_at(&self, value: &Value, path: &str) -> Result<(), SchemaViolation> {
        match self {
            Self::Null if value.is_null() => Ok(()),
            Self::Boolean if value.is_boolean() => Ok(()),
            Self::Integer { minimum, maximum } => {
                let number = value.as_i64().ok_or_else(|| wrong_type(path, "integer"))?;
                validate_bounds(number, *minimum, *maximum, path)
            }
            Self::Number { minimum, maximum } => {
                let number = value.as_f64().ok_or_else(|| wrong_type(path, "number"))?;
                validate_bounds(number, *minimum, *maximum, path)
            }
            Self::String {
                min_length,
                max_length,
                enum_values,
            } => {
                let text = value.as_str().ok_or_else(|| wrong_type(path, "string"))?;
                let length = text.chars().count();
                if min_length.is_some_and(|min| length < min)
                    || max_length.is_some_and(|max| length > max)
                {
                    return Err(SchemaViolation::new(path, "string length is out of bounds"));
                }
                if !enum_values.is_empty() && !enum_values.iter().any(|allowed| allowed == text) {
                    return Err(SchemaViolation::new(path, "value is not in the enum"));
                }
                Ok(())
            }
            Self::Array {
                items,
                min_items,
                max_items,
            } => {
                let values = value.as_array().ok_or_else(|| wrong_type(path, "array"))?;
                if min_items.is_some_and(|min| values.len() < min)
                    || max_items.is_some_and(|max| values.len() > max)
                {
                    return Err(SchemaViolation::new(path, "array length is out of bounds"));
                }
                for (index, item) in values.iter().enumerate() {
                    items.validate_at(item, &format!("{path}[{index}]"))?;
                }
                Ok(())
            }
            Self::Object {
                properties,
                required,
            } => {
                let object = value
                    .as_object()
                    .ok_or_else(|| wrong_type(path, "object"))?;
                self.validate_object(object, properties, required, path)
            }
            _ => Err(wrong_type(path, self.type_name())),
        }
    }

    fn validate_object(
        &self,
        object: &Map<String, Value>,
        properties: &BTreeMap<String, JsonSchema>,
        required: &BTreeSet<String>,
        path: &str,
    ) -> Result<(), SchemaViolation> {
        for name in required {
            if !object.contains_key(name) {
                return Err(SchemaViolation::new(
                    format!("{path}.{}", escape_path(name)),
                    "required property is missing",
                ));
            }
        }
        for (name, value) in object {
            let schema = properties.get(name).ok_or_else(|| {
                SchemaViolation::new(
                    format!("{path}.{}", escape_path(name)),
                    "additional properties are forbidden",
                )
            })?;
            schema.validate_at(value, &format!("{path}.{}", escape_path(name)))?;
        }
        Ok(())
    }

    /// Emits standards-shaped JSON Schema for transport or documentation.
    pub fn to_json_schema(&self) -> Value {
        match self {
            Self::Null => json!({"type": "null"}),
            Self::Boolean => json!({"type": "boolean"}),
            Self::Integer { minimum, maximum } => numeric_schema("integer", *minimum, *maximum),
            Self::Number { minimum, maximum } => numeric_schema("number", *minimum, *maximum),
            Self::String {
                min_length,
                max_length,
                enum_values,
            } => {
                let mut schema = Map::from_iter([("type".into(), Value::String("string".into()))]);
                insert_optional(&mut schema, "minLength", *min_length);
                insert_optional(&mut schema, "maxLength", *max_length);
                if !enum_values.is_empty() {
                    schema.insert("enum".into(), json!(enum_values));
                }
                Value::Object(schema)
            }
            Self::Array {
                items,
                min_items,
                max_items,
            } => {
                let mut schema = Map::from_iter([
                    ("type".into(), Value::String("array".into())),
                    ("items".into(), items.to_json_schema()),
                ]);
                insert_optional(&mut schema, "minItems", *min_items);
                insert_optional(&mut schema, "maxItems", *max_items);
                Value::Object(schema)
            }
            Self::Object {
                properties,
                required,
            } => json!({
                "type": "object",
                "properties": properties
                    .iter()
                    .map(|(name, schema)| (name.clone(), schema.to_json_schema()))
                    .collect::<Map<_, _>>(),
                "required": required,
                "additionalProperties": false
            }),
        }
    }

    fn type_name(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Boolean => "boolean",
            Self::Integer { .. } => "integer",
            Self::Number { .. } => "number",
            Self::String { .. } => "string",
            Self::Array { .. } => "array",
            Self::Object { .. } => "object",
        }
    }
}

fn validate_bounds<T: PartialOrd>(
    value: T,
    minimum: Option<T>,
    maximum: Option<T>,
    path: &str,
) -> Result<(), SchemaViolation> {
    if minimum.is_some_and(|min| value < min) || maximum.is_some_and(|max| value > max) {
        Err(SchemaViolation::new(path, "number is out of bounds"))
    } else {
        Ok(())
    }
}

fn wrong_type(path: &str, expected: &str) -> SchemaViolation {
    SchemaViolation::new(path, format!("expected {expected}"))
}

fn escape_path(name: &str) -> String {
    name.replace('\\', "\\\\").replace('.', "\\.")
}

fn numeric_schema<T: Serialize>(kind: &str, minimum: Option<T>, maximum: Option<T>) -> Value {
    let mut schema = Map::from_iter([("type".into(), Value::String(kind.into()))]);
    insert_optional(&mut schema, "minimum", minimum);
    insert_optional(&mut schema, "maximum", maximum);
    Value::Object(schema)
}

fn insert_optional<T: Serialize>(map: &mut Map<String, Value>, key: &str, value: Option<T>) {
    if let Some(value) = value {
        map.insert(
            key.into(),
            serde_json::to_value(value).expect("schema values are serializable"),
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaViolation {
    pub path: String,
    pub message: String,
}

impl SchemaViolation {
    fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for SchemaViolation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.path, self.message)
    }
}

impl std::error::Error for SchemaViolation {}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SchemaDefinitionError {
    #[error("{0}: schema bounds are invalid")]
    InvalidBounds(String),
    #[error("{0}: enum contains duplicate values")]
    DuplicateEnumValue(String),
    #[error("{path}: required property `{property}` is not declared")]
    UnknownRequiredProperty { path: String, property: String },
    #[error("{path}: property name `{property}` is invalid")]
    InvalidPropertyName { path: String, property: String },
}
