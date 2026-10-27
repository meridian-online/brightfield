//! Validation of a document against the vendored Ossie JSON Schema.

use std::sync::OnceLock;

use jsonschema::Validator;

const SCHEMA: &str = include_str!("../vendor/ossie/osi-schema.json");

/// One place where a document breaks the schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaIssue {
    /// JSON pointer to the offending value; empty for the document root.
    pub path: String,
    pub message: String,
}

fn validator() -> &'static Validator {
    static V: OnceLock<Validator> = OnceLock::new();
    V.get_or_init(|| {
        let schema: serde_json::Value =
            serde_json::from_str(SCHEMA).expect("vendored schema is JSON");
        jsonschema::validator_for(&schema).expect("vendored schema compiles")
    })
}

/// Every schema violation in `yaml`, or one issue if it is not YAML at all.
pub fn validate(yaml: &str) -> Vec<SchemaIssue> {
    let instance: serde_json::Value = match serde_yaml::from_str(yaml) {
        Ok(v) => v,
        Err(e) => {
            return vec![SchemaIssue {
                path: String::new(),
                message: format!("not valid YAML: {e}"),
            }]
        }
    };
    validator()
        .iter_errors(&instance)
        .map(|e| SchemaIssue {
            path: e.instance_path.to_string(),
            message: e.to_string(),
        })
        .collect()
}
