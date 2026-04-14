// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! Workflow YAML definition structs.

use indexmap::IndexMap;
use serde::Deserialize;
use std::collections::HashMap;

/// Top-level workflow definition parsed from YAML.
#[derive(Debug, Clone, Deserialize)]
pub struct WorkflowDef {
    pub name: String,
    pub on: TriggerDef,
    #[serde(default)]
    pub env: HashMap<String, String>,
    pub jobs: IndexMap<String, JobDef>,
}

/// Trigger configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct TriggerDef {
    pub push: Option<PushTrigger>,
    pub manual: Option<bool>,
}

/// Push trigger with branch filters.
#[derive(Debug, Clone, Deserialize)]
pub struct PushTrigger {
    #[serde(default)]
    pub branches: Vec<String>,
}

/// A job within a workflow.
#[derive(Debug, Clone, Deserialize)]
pub struct JobDef {
    pub name: String,
    pub steps: Vec<StepDef>,
}

/// A single step within a job.
#[derive(Debug, Clone, Deserialize)]
pub struct StepDef {
    pub name: String,
    /// Shell command to run.
    pub run: Option<String>,
    /// Upload an artifact.
    pub artifact: Option<ArtifactDef>,
    /// Create a release.
    pub release: Option<ReleaseDef>,
    /// Per-step wall-clock cap. Unset → engine default (30m). Applied to
    /// `run:` steps; artifact/release bookkeeping steps are cheap and
    /// exempt.
    #[serde(rename = "timeout-minutes", default)]
    pub timeout_minutes: Option<u64>,
}

/// Artifact upload definition.
#[derive(Debug, Clone, Deserialize)]
pub struct ArtifactDef {
    pub name: String,
    pub path: String,
}

/// Release creation definition.
#[derive(Debug, Clone, Deserialize)]
pub struct ReleaseDef {
    pub tag: String,
    pub name: String,
    #[serde(default)]
    pub artifacts: Vec<String>,
}

impl WorkflowDef {
    /// Parse a workflow from YAML text.
    pub fn parse(yaml: &str) -> Result<Self, serde_yaml::Error> {
        serde_yaml::from_str(yaml)
    }

    /// Check if this workflow should trigger on a push to the given branch.
    pub fn matches_push(&self, ref_name: &str) -> bool {
        if let Some(push) = &self.on.push {
            let branch = ref_name
                .strip_prefix("refs/heads/")
                .unwrap_or(ref_name);
            if push.branches.is_empty() {
                return true; // no filter = match all
            }
            push.branches.iter().any(|b| b == branch || b == "*")
        } else {
            false
        }
    }

    /// Check if manual trigger is enabled.
    pub fn allows_manual(&self) -> bool {
        self.on.manual.unwrap_or(false)
    }
}
