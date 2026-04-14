use globset::{Glob, GlobSet, GlobSetBuilder};
use std::path::Path;

/// Default ignore patterns for Unreal Engine projects.
pub const DEFAULT_PATTERNS: &[&str] = &[
    // UE build outputs (top-level and inside plugins)
    "**/Binaries/**",
    "**/Intermediate/**",
    "**/Saved/**",
    "**/DerivedDataCache/**",
    "**/Build/**",
    // IDE
    "**/.vs/**",
    "**/.idea/**",
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
    // Forge internal
    ".forge/**",
];

/// Manages .forgeignore patterns for filtering files.
#[derive(Debug)]
pub struct ForgeIgnore {
    glob_set: GlobSet,
    patterns: Vec<String>,
}

impl Default for ForgeIgnore {
    fn default() -> Self {
        Self {
            glob_set: GlobSet::empty(),
            patterns: Vec::new(),
        }
    }
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
    ///
    /// Accepts a gitignore-ish subset: `#` comments, blank lines, and glob
    /// patterns. Gitignore-style trailing-slash semantics are normalized
    /// here — `Docs/node_modules/` is expanded into two registered globs,
    /// `Docs/node_modules` (matches the directory itself, so the status
    /// walker can prune descent) and `Docs/node_modules/**` (matches every
    /// file under it). Without this expansion, a literal trailing slash
    /// never matches anything because the paths we check against are
    /// always `/`-less at their tail.
    pub fn from_str(content: &str) -> Result<Self, ForgeIgnoreError> {
        let mut builder = GlobSetBuilder::new();
        let mut patterns = Vec::new();

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let raw = line.to_string();
            let trimmed = raw.trim_end_matches('/');
            // Directory form: `foo/` → ignore the dir AND its contents.
            // Plain form: `foo` → ignore the file/dir AND its contents
            // (matches common expectation; gitignore also treats bare
            // directory names this way when they're directories).
            let dir_glob = Glob::new(trimmed)
                .map_err(|e| ForgeIgnoreError::Pattern(e.to_string()))?;
            builder.add(dir_glob);

            let contents = format!("{}/**", trimmed);
            let contents_glob = Glob::new(&contents)
                .map_err(|e| ForgeIgnoreError::Pattern(e.to_string()))?;
            builder.add(contents_glob);

            patterns.push(raw);
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
    fn trailing_slash_pattern_matches_dir_and_contents() {
        // Regression: `Docs/node_modules/` was being passed to globset as
        // a literal pattern ending in '/', which never matched the
        // slash-less paths the status walker feeds in.
        let ignore = ForgeIgnore::from_str("Docs/node_modules/\n").unwrap();
        assert!(ignore.is_ignored("Docs/node_modules"));
        assert!(ignore.is_ignored("Docs/node_modules/package.json"));
        assert!(ignore.is_ignored("Docs/node_modules/.bin/acorn"));
        assert!(!ignore.is_ignored("Docs/src/index.js"));
    }

    #[test]
    fn bare_dir_name_matches_contents() {
        let ignore = ForgeIgnore::from_str("build\n").unwrap();
        assert!(ignore.is_ignored("build"));
        assert!(ignore.is_ignored("build/foo.o"));
    }

    #[test]
    fn test_default_content() {
        let content = ForgeIgnore::default_content();
        assert!(content.contains("Intermediate/**"));
    }
}
