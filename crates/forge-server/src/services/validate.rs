// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use tonic::Status;

/// Validate a single repo path segment (owner OR name half).
fn repo_segment(seg: &str, label: &str) -> Result<(), Status> {
    if seg.is_empty() || seg.len() > 64 {
        return Err(Status::invalid_argument(format!(
            "{label} must be 1-64 characters"
        )));
    }
    if seg.contains("..") || seg.contains('\\') {
        return Err(Status::invalid_argument(format!(
            "{label} cannot contain '..' or '\\'"
        )));
    }
    if seg.starts_with('.') {
        return Err(Status::invalid_argument(format!(
            "{label} cannot start with '.'"
        )));
    }
    if !seg
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(Status::invalid_argument(format!(
            "{label} must be alphanumeric, hyphens, underscores, or dots"
        )));
    }
    Ok(())
}

/// Validate a repository identifier in `<owner>/<name>` form. The
/// gRPC layer normalizes bare `<name>` callers to this form first by
/// prepending the authenticated caller's username (see
/// [`crate::services::grpc::ForgeGrpcService::resolve_repo_path`]), so
/// by the time this function runs the input always has the slash.
pub fn repo_name(name: &str) -> Result<(), Status> {
    if name.is_empty() || name.len() > 128 {
        return Err(Status::invalid_argument("repo path must be 1-128 characters"));
    }
    let mut parts = name.splitn(2, '/');
    let owner = parts.next().unwrap_or("");
    let repo = parts.next().ok_or_else(|| {
        Status::invalid_argument("repo path must be in '<owner>/<name>' form")
    })?;
    if repo.contains('/') {
        return Err(Status::invalid_argument(
            "repo path must contain exactly one '/' separator",
        ));
    }
    repo_segment(owner, "owner")?;
    repo_segment(repo, "repo name")?;
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
