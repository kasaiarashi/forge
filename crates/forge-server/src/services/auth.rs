// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

use tonic::{Request, Status};

pub fn make_auth_interceptor(
    enabled: bool,
    tokens: Vec<String>,
) -> impl Fn(Request<()>) -> Result<Request<()>, Status> + Clone + Send + Sync + 'static {
    move |req: Request<()>| {
        if !enabled {
            return Ok(req);
        }
        let meta = req.metadata();
        let token = meta
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));
        match token {
            Some(t) if tokens.iter().any(|valid| valid == t) => Ok(req),
            _ => Err(Status::unauthenticated("invalid or missing token")),
        }
    }
}
