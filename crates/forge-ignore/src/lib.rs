use globset::{Glob, GlobSet, GlobSetBuilder};
use std::path::Path;

/// Default ignore patterns for Unreal Engine projects.
pub const DEFAULT_PATTERNS: &[&str] = &[
    // UE build outputs
    "Binaries/**",
    "Intermediate/**",
    "Saved/**",
    "DerivedDataCache/**",
    "Build/**",
    // IDE
    ".vs/**",
    ".idea/**",
    "*.sln",
    "*.suo",
    "*.opensdf",
    "*.sdf",
    "*.VC.db",
    "*.VC.opendb",
    // OS
    ".DS_Store",
    "Thumbs.db",
    // Logs and temp
    "*.log",
    "*.tmp",
    "*.temp",
    "*.dmp",
];

/// Manages .forgeignore patterns for filtering files.
#[derive(Debug)]
pub struct ForgeIgnore {
    glob_set: GlobSet,
    patterns: Vec<String>,
}

impl ForgeIgnore {
    /// Load ignore patterns from a .forgeignore file.
    pub fn from_file(path: &Path) -> Result<Self, ForgeIgnoreError> {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(ForgeIgnoreError::Io(e)),
        };
        Self::from_str(&content)
    }

    /// Parse ignore patterns from string content.
    pub fn from_str(content: &str) -> Result<Self, ForgeIgnoreError> {
        let mut builder = GlobSetBuilder::new();
        let mut patterns = Vec::new();

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let glob = Glob::new(line).map_err(|e| ForgeIgnoreError::Pattern(e.to_string()))?;
            builder.add(glob);
            patterns.push(line.to_string());
        }

        let glob_set = builder.build().map_err(|e| ForgeIgnoreError::Pattern(e.to_string()))?;
        Ok(Self { glob_set, patterns })
    }

    /// Check if a path should be ignored.
    pub fn is_ignored(&self, path: &str) -> bool {
        self.glob_set.is_match(path)
    }

    /// Get the list of patterns.
    pub fn patterns(&self) -> &[String] {
        &self.patterns
    }

    /// Generate default .forgeignore content.
    pub fn default_content() -> String {
        let mut content = String::from("# Forge ignore patterns\n\n# Unreal Engine\n");
        for pattern in DEFAULT_PATTERNS {
            content.push_str(pattern);
            content.push('\n');
        }
        content
    }
}

#[derive(Debug)]
pub enum ForgeIgnoreError {
    Io(std::io::Error),
    Pattern(String),
}

impl std::fmt::Display for ForgeIgnoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "IO error: {}", e),
            Self::Pattern(e) => write!(f, "Invalid pattern: {}", e),
        }
    }
}

impl std::error::Error for ForgeIgnoreError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ignore_patterns() {
        let ignore = ForgeIgnore::from_str("*.uasset\nBinaries/**\n# comment\n").unwrap();
        assert!(ignore.is_ignored("Binaries/Win64/game.exe"));
        assert!(!ignore.is_ignored("Content/Maps/Level.umap"));
    }

    #[test]
    fn test_default_content() {
        let content = ForgeIgnore::default_content();
        assert!(content.contains("Intermediate/**"));
    }
}
