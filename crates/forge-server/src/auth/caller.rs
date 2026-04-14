// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! The [`Caller`] is the authenticated identity attached to every request
//! that flows through the gRPC interceptor (phase 3).
//!
//! Phase 1 only defines the type — phase 3 will populate it from the bearer
//! token and stash it in `tonic::Request::extensions`. Phases 3+ read it from
//! the request and pass it to the per-handler authorization helpers.

use super::tokens::Scope;

/// The authenticated principal making a gRPC call.
///
/// `Anonymous` is a real value, not a `None` — every request has a `Caller`.
/// The authorization helpers decide whether anonymous is acceptable for a
/// given operation (it is for read on a public repo; nothing else).
#[derive(Debug, Clone)]
pub enum Caller {
    Anonymous,
    Authenticated(AuthenticatedCaller),
}

#[derive(Debug, Clone)]
pub struct AuthenticatedCaller {
    pub user_id: i64,
    pub username: String,
    pub is_server_admin: bool,
    pub scopes: Vec<Scope>,
    /// Which kind of credential validated this caller. Useful for the
    /// "list active sessions" UI and audit logging later.
    pub credential: CredentialKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialKind {
    /// A web session token (cookie). Short-lived. Inherits all the user's
    /// permissions; sessions are not scope-restricted.
    Session,
    /// A personal access token. Long-lived; restricted to the scopes the
    /// user picked at creation time.
    PersonalAccessToken,
}

impl Caller {
    pub fn anonymous() -> Self {
        Self::Anonymous
    }

    pub fn is_anonymous(&self) -> bool {
        matches!(self, Self::Anonymous)
    }

    pub fn user_id(&self) -> Option<i64> {
        match self {
            Self::Authenticated(a) => Some(a.user_id),
            Self::Anonymous => None,
        }
    }

    pub fn username(&self) -> Option<&str> {
        match self {
            Self::Authenticated(a) => Some(&a.username),
            Self::Anonymous => None,
        }
    }

    pub fn is_server_admin(&self) -> bool {
        match self {
            Self::Authenticated(a) => a.is_server_admin,
            Self::Anonymous => false,
        }
    }

    /// Returns true if the caller carries the given scope. Sessions
    /// implicitly have every scope (they're "the user, full power"); PATs
    /// only have what was selected at creation time.
    pub fn has_scope(&self, want: Scope) -> bool {
        match self {
            Self::Anonymous => false,
            Self::Authenticated(a) => match a.credential {
                CredentialKind::Session => true,
                CredentialKind::PersonalAccessToken => a.scopes.contains(&want),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pat_caller(scopes: Vec<Scope>) -> Caller {
        Caller::Authenticated(AuthenticatedCaller {
            user_id: 1,
            username: "alice".into(),
            is_server_admin: false,
            scopes,
            credential: CredentialKind::PersonalAccessToken,
        })
    }

    fn session_caller() -> Caller {
        Caller::Authenticated(AuthenticatedCaller {
            user_id: 1,
            username: "alice".into(),
            is_server_admin: false,
            scopes: vec![],
            credential: CredentialKind::Session,
        })
    }

    #[test]
    fn anonymous_has_no_scopes_and_no_user() {
        let c = Caller::anonymous();
        assert!(c.is_anonymous());
        assert!(c.user_id().is_none());
        assert!(c.username().is_none());
        assert!(!c.is_server_admin());
        assert!(!c.has_scope(Scope::RepoRead));
    }

    #[test]
    fn pat_caller_only_has_listed_scopes() {
        let c = pat_caller(vec![Scope::RepoRead]);
        assert!(c.has_scope(Scope::RepoRead));
        assert!(!c.has_scope(Scope::RepoWrite));
        assert!(!c.has_scope(Scope::RepoAdmin));
        assert!(!c.has_scope(Scope::UserAdmin));
    }

    #[test]
    fn session_caller_has_every_scope() {
        let c = session_caller();
        assert!(c.has_scope(Scope::RepoRead));
        assert!(c.has_scope(Scope::RepoWrite));
        assert!(c.has_scope(Scope::RepoAdmin));
        assert!(c.has_scope(Scope::UserAdmin));
    }
}
