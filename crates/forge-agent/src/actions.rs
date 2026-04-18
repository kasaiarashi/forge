// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the BSL 1.1..

//! Composite action resolver + builtin-primitive dispatcher.
//!
//! Workflow YAML is parsed loose-ly here (accepts unknown fields) because
//! the schema is iterating fast — anything we don't understand becomes a
//! no-op step with a warning rather than a parse error.

use anyhow::{anyhow, Result};
use indexmap::IndexMap;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ComposedStep {
    pub name: String,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub run: Option<String>,
    #[serde(default)]
    pub uses: Option<String>,
    #[serde(default)]
    pub with: IndexMap<String, String>,
    /// Explicit shell for `run:` steps. See
    /// `forge-server::services::actions::shell` for the accepted values;
    /// the agent mirrors that mapping so a workflow runs identically
    /// whether the server or an agent executes it.
    #[serde(default)]
    pub shell: Option<String>,
    #[serde(rename = "timeout-minutes", default)]
    pub timeout_minutes: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ActionDef {
    pub name: String,
    #[serde(default)]
    pub inputs: HashMap<String, InputSpec>,
    #[serde(default)]
    pub outputs: HashMap<String, String>,
    pub steps: Vec<ComposedStep>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct InputSpec {
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub default: Option<String>,
}

impl ActionDef {
    pub fn parse(yaml: &str) -> Result<Self> {
        serde_yaml::from_str(yaml).map_err(|e| anyhow!("parse action YAML: {e}"))
    }

    /// Resolve `inputs.*` defaults and validate required ones. Returns
    /// a flat map callers can thread into the expression context.
    pub fn resolve_inputs(
        &self,
        with: &IndexMap<String, String>,
    ) -> Result<HashMap<String, String>> {
        let mut out = HashMap::new();
        for (name, spec) in &self.inputs {
            if let Some(v) = with.get(name) {
                out.insert(name.clone(), v.clone());
            } else if let Some(d) = &spec.default {
                out.insert(name.clone(), d.clone());
            } else if spec.required {
                return Err(anyhow!(
                    "action '{}' requires input '{}' but none was provided",
                    self.name,
                    name
                ));
            } else {
                out.insert(name.clone(), String::new());
            }
        }
        Ok(out)
    }
}

/// Expand `${{ <scope>.<key> }}` references inside `input`. Scopes are:
///
///   * `inputs.X` — looked up in `inputs`.
///   * `steps.ID.outputs.Y` — looked up in `step_outputs[ID][Y]`.
///   * `env.X` — passed through to the shell, not rewritten.
pub fn expand_expr(
    input: &str,
    inputs: &HashMap<String, String>,
    step_outputs: &HashMap<String, HashMap<String, String>>,
) -> String {
    let re =
        regex::Regex::new(r"\$\{\{\s*([A-Za-z_][A-Za-z0-9_.]*)\s*\}\}").expect("static regex");
    re.replace_all(input, |caps: &regex::Captures| {
        let path = &caps[1];
        let parts: Vec<&str> = path.split('.').collect();
        match parts.as_slice() {
            ["inputs", k] => inputs.get(*k).cloned().unwrap_or_default(),
            ["steps", id, "outputs", k] => step_outputs
                .get(*id)
                .and_then(|m| m.get(*k).cloned())
                .unwrap_or_default(),
            // Unknown scope — leave untouched so shell picks it up.
            _ => caps[0].to_string(),
        }
    })
    .into_owned()
}
