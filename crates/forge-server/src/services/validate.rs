// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use tonic::Status;

/// Validate a repository name.
pub fn repo_name(name: &str) -> Result<(), Status> {
    if name.is_empty() || name.len() > 128 {
        return Err(Status::invalid_argument("repo name must be 1-128 characters"));
    }
    if name.contains("..") || name.contains('/') || name.contains('\\') {
        return Err(Status::invalid_argument(
            "repo name cannot contain '..', '/' or '\\'",
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(Status::invalid_argument(
            "repo name must be alphanumeric, hyphens, underscores, or dots",
        ));
    }
    Ok(())
}

/// Validate a ref name (branch or tag).
pub fn ref_name(name: &str) -> Result<(), Status> {
    if name.is_empty() || name.len() > 256 {
        return Err(Status::invalid_argument("ref name must be 1-256 characters"));
    }
    if name.contains("..") {
        return Err(Status::invalid_argument("ref name cannot contain '..'"));
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || "/-_.".contains(c))
    {
        return Err(Status::invalid_argument(
            "ref name contains invalid characters",
        ));
    }
    Ok(())
}

/// Validate a file path.
pub fn path(path: &str) -> Result<(), Status> {
    if path.len() > 4096 {
        return Err(Status::invalid_argument("path too long (max 4096)"));
    }
    if path.contains('\0') {
        return Err(Status::invalid_argument("path contains null byte"));
    }
    if path.starts_with('/') || path.starts_with('\\') {
        return Err(Status::invalid_argument("path must be relative"));
    }
    if path.split('/').any(|c| c == "..") || path.split('\\').any(|c| c == "..") {
        return Err(Status::invalid_argument(
            "path cannot contain '..' components",
        ));
    }
    Ok(())
}
