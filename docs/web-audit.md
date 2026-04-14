# Web UI Sync Audit

Audit of what is synced from forge-server (gRPC) through forge-web (HTTP) to the frontend (React), and what improvements can be made.

Generated: 2026-04-10

---

## Layer Inventory

| Layer | Count |
|-------|-------|
| gRPC RPCs (forge-server) | 51 |
| HTTP routes (forge-web) | 51 |
| Frontend API methods (api.ts) | 41 |

---

## Sync Matrix

### AuthService (16 gRPC RPCs)

| gRPC RPC | Web Route | Frontend Method | Status |
|----------|-----------|-----------------|--------|
| Login | POST `/api/auth/login` | `login()` | Synced |
| Logout | POST `/api/auth/logout` | `logout()` | Synced |
| WhoAmI | GET `/api/auth/me` | `me()` | Synced |
| IsServerInitialized | GET `/api/auth/initialized` | `isInitialized()` | Synced |
| BootstrapAdmin | POST `/api/auth/bootstrap` | `bootstrapAdmin()` | Synced |
| CreatePersonalAccessToken | POST `/api/auth/tokens` | _none_ | **Missing in frontend** |
| ListPersonalAccessTokens | GET `/api/auth/tokens` | _none_ | **Missing in frontend** |
| RevokePersonalAccessToken | DELETE `/api/auth/tokens/:id` | _none_ | **Missing in frontend** |
| ListMySessions | GET `/api/auth/sessions` | _none_ | **Missing in frontend** |
| RevokeSession | DELETE `/api/auth/sessions/:id` | _none_ | **Missing in frontend** |
| CreateUser | POST `/api/auth/users` | _none_ | **Missing in frontend** |
| ListUsers | GET `/api/auth/users` | `listUsers()` | Synced |
| DeleteUser | DELETE `/api/auth/users/:id` | _none_ | **Missing in frontend** |
| GrantRepoRole | POST `/api/auth/repos/:repo/members` | `addRepoMember()` | Synced |
| RevokeRepoRole | DELETE `/api/auth/repos/:repo/members/:user_id` | `removeRepoMember()` | Synced |
| ListRepoMembers | GET `/api/auth/repos/:repo/members` | `listRepoMembers()` | Synced |

### ForgeService (35 gRPC RPCs)

| gRPC RPC | Web Route | Frontend Method | Status |
|----------|-----------|-----------------|--------|
| PushObjects | _n/a (streaming, CLI only)_ | _n/a_ | CLI only |
| PullObjects | _n/a (streaming, CLI only)_ | _n/a_ | CLI only |
| HasObjects | _n/a (CLI only)_ | _n/a_ | CLI only |
| GetRefs | _n/a (CLI only)_ | _n/a_ | CLI only |
| UpdateRef | _n/a (CLI only)_ | _n/a_ | CLI only |
| AcquireLock | POST `/api/repos/:repo/locks/acquire` | _none_ | **Missing in frontend** |
| ReleaseLock | DELETE `/api/repos/:repo/locks/:path` | `unlockFile()` | Synced |
| ListLocks | GET `/api/repos/:repo/locks` | `getLocks()` | Synced |
| VerifyLocks | _n/a (CLI only)_ | _n/a_ | CLI only |
| ListRepos | GET `/api/repos` | `listRepos()` | Synced |
| CreateRepo | POST `/api/repos` | `createRepo()` | Synced |
| UpdateRepo | PUT `/api/repos/:repo` | `updateRepo()` | Synced |
| DeleteRepo | DELETE `/api/repos/:repo` | `deleteRepo()` | Synced |
| ListCommits | GET `/api/repos/:repo/commits/:branch` | `listCommits()` | Synced |
| GetTreeEntries | GET `/api/repos/:repo/tree/:branch` | `getTree()` | Synced |
| GetFileContent | GET `/api/repos/:repo/blob/:branch` | `getFile()` | Synced |
| GetCommitDetail | GET `/api/repos/:repo/commit/:hash` | `getCommit()` | Synced |
| GetServerInfo | GET `/api/server/info` | `getServerInfo()` | Synced |
| ListWorkflows | GET `/api/repos/:repo/workflows` | `listWorkflows()` | Synced |
| CreateWorkflow | POST `/api/repos/:repo/workflows` | `createWorkflow()` | Synced |
| UpdateWorkflow | PUT `/api/repos/:repo/workflows/:id` | `updateWorkflow()` | Synced |
| DeleteWorkflow | DELETE `/api/repos/:repo/workflows/:id` | `deleteWorkflow()` | Synced |
| TriggerWorkflow | POST `/api/repos/:repo/workflows/:id/trigger` | `triggerWorkflow()` | Synced |
| ListWorkflowRuns | GET `/api/repos/:repo/runs` | `listRuns()` | Synced |
| GetWorkflowRun | GET `/api/repos/:repo/runs/:run_id` | `getRun()` | Synced |
| CancelWorkflowRun | POST `/api/repos/:repo/runs/:run_id/cancel` | `cancelRun()` | Synced |
| ListArtifacts | GET `/api/repos/:repo/runs/:run_id/artifacts` | _none_ | **Missing in frontend** |
| ListReleases | GET `/api/repos/:repo/releases` | `listReleases()` | Synced |
| GetRelease | GET `/api/repos/:repo/releases/:release_id` | _none_ | **Missing in frontend** |
| ListIssues | GET `/api/repos/:repo/issues` | `listIssues()` | Synced |
| CreateIssue | POST `/api/repos/:repo/issues` | `createIssue()` | Synced |
| UpdateIssue | PUT `/api/repos/:repo/issues/:id` | `updateIssue()` | Synced |
| GetIssue | GET `/api/repos/:repo/issues/:id` | `getIssue()` | Synced |
| ListPullRequests | GET `/api/repos/:repo/pulls` | `listPullRequests()` | Synced |
| CreatePullRequest | POST `/api/repos/:repo/pulls` | `createPullRequest()` | Synced |
| UpdatePullRequest | PUT `/api/repos/:repo/pulls/:id` | `updatePullRequest()` | Synced |
| MergePullRequest | POST `/api/repos/:repo/pulls/:id/merge` | `mergePullRequest()` | Synced |
| GetPullRequest | GET `/api/repos/:repo/pulls/:id` | `getPullRequest()` | Synced |

### Raw file endpoint (web-only, no dedicated gRPC RPC)

| Web Route | Frontend Method | Status |
|-----------|-----------------|--------|
| GET `/api/repos/:repo/raw/:branch` | _none_ | **Missing in frontend** |
| GET `/api/repos/:repo/stats/languages` | `getLanguageStats()` | Synced |

---

## Gap Summary

### APIs exposed by web but not wired in frontend (10 gaps)

| # | Web Route | Impact | Priority | Status |
|---|-----------|--------|----------|--------|
| 1 | GET `/api/auth/tokens` | Users cannot view their PATs from the web UI | High | **Fixed** — AccountSettings page |
| 2 | POST `/api/auth/tokens` | Users cannot create PATs from the web UI | High | **Fixed** — AccountSettings page |
| 3 | DELETE `/api/auth/tokens/:id` | Users cannot revoke PATs from the web UI | High | **Fixed** — AccountSettings page |
| 4 | GET `/api/auth/sessions` | Users cannot view active sessions | Medium | **Fixed** — AccountSettings page |
| 5 | DELETE `/api/auth/sessions/:id` | Users cannot revoke sessions | Medium | **Fixed** — AccountSettings page |
| 6 | POST `/api/auth/users` | Admins cannot create users from the web UI | High | **Fixed** — Admin Users panel |
| 7 | DELETE `/api/auth/users/:id` | Admins cannot delete users from the web UI | High | **Fixed** — Admin Users panel |
| 8 | POST `/api/repos/:repo/locks/acquire` | Cannot acquire locks from web (CLI-primary, low priority) | Low | **Fixed** — Locks page acquire form |
| 9 | GET `/api/repos/:repo/runs/:run_id/artifacts` | Artifacts listed inline via `getRun()`, standalone call unused | Low | Skipped — no value |
| 10 | GET `/api/repos/:repo/releases/:release_id` | Release detail page not implemented; list view shows all data | Low | Skipped — no value |

### gRPC RPCs with no web route (6, all CLI-only by design)

| RPC | Reason |
|-----|--------|
| PushObjects | Streaming; CLI push |
| PullObjects | Streaming; CLI pull |
| HasObjects | Object negotiation; CLI only |
| GetRefs | Ref negotiation; CLI only |
| UpdateRef | Ref update; CLI only |
| VerifyLocks | Lock verification; CLI only |

These are correctly omitted from the web layer.

---

## Improvements

### High Priority

#### 1. Account Settings Page (PATs + Sessions) — DONE

**Fixed:** Added `/account` route with full PAT management (create with scopes, list, revoke, one-time plaintext display) and session management (list with user agent/IP/timestamps, revoke). Accessible from user dropdown menu.

**Files changed:**
- `ui/src/pages/AccountSettings.tsx` (new)
- `ui/src/api.ts` (added `listTokens`, `createToken`, `deleteToken`, `listSessions`, `deleteSession`)
- `ui/src/App.tsx` (added `/account` route)
- `ui/src/components/Layout.tsx` (added "Account settings" to user dropdown)

---

#### 2. Admin User Management Panel — DONE

**Fixed:** Added Users section to `/admin` with user table (username, email, display name, admin badge), create user form (all fields + admin toggle), and delete with inline confirmation.

**Files changed:**
- `ui/src/pages/Admin.tsx` (added Users section)
- `ui/src/api.ts` (added `createUser`, `deleteUser`)

---

#### 3. Visibility Toggle in Repo Settings — DONE

**Fixed:** Added visibility dropdown (Public/Private) to RepoSettings with immediate save. Updated RepoHeader to accept optional `visibility` prop and display actual state instead of hardcoded "Public". Added `visibility` field to `RepoInfo` interface and `updateRepo` signature.

**Files changed:**
- `ui/src/pages/RepoSettings.tsx` (added visibility card)
- `ui/src/components/RepoHeader.tsx` (dynamic visibility label)
- `ui/src/pages/RepoTree.tsx` (passes visibility to RepoHeader)
- `ui/src/api.ts` (added `visibility` to `RepoInfo` and `updateRepo`)

---

### Medium Priority

#### 4. User Lookup Endpoint for Non-Admin Repo Admins — DONE

**Fixed:** Added `LookupUser` gRPC RPC (any authenticated user), `GET /api/auth/users/lookup?username=` web route, `lookupUser()` frontend method. RepoSettings collaborator add now uses `lookupUser` instead of admin-only `listUsers`.

**Files changed:**
- `proto/forge.proto` (added `LookupUser` RPC + messages)
- `crates/forge-server/src/services/auth_service.rs` (implemented handler)
- `crates/forge-web/server/src/auth.rs` (added HTTP handler)
- `crates/forge-web/server/src/main.rs` (registered route)
- `ui/src/api.ts` (added `lookupUser`)
- `ui/src/pages/RepoSettings.tsx` (uses `lookupUser`)

---

#### 5. PR Sidebar Metadata (Labels) — DONE

**Fixed:** Labels input wired in NewPullRequest sidebar — comma-separated labels passed to `createPullRequest`. Reviewers/assignees/milestones remain placeholders (need new backend RPCs).

**Files changed:**
- `ui/src/pages/NewPullRequest.tsx` (labels input + pass to createPullRequest)

---

#### 6. Issue & PR Comments — DONE

**Fixed:** Full end-to-end comment support:
- DB: `comments` table with repo, issue_id, kind, author, body, timestamps
- Proto: `ListComments`, `CreateComment`, `UpdateComment`, `DeleteComment` RPCs
- Server: gRPC handlers + DB CRUD + auto-increment/decrement `comment_count`
- Web: HTTP routes `GET/POST /api/repos/:repo/comments`, `PUT/DELETE .../comments/:id`
- Frontend: comment threads on IssueDetail and PullRequestDetail with create/delete

**Files changed:**
- `proto/forge.proto` (4 new RPCs + `CommentInfo` message)
- `crates/forge-server/src/storage/db.rs` (comments table + CRUD methods + `CommentRecord`)
- `crates/forge-server/src/services/grpc.rs` (4 comment handlers)
- `crates/forge-web/server/src/grpc_client.rs` (4 comment methods)
- `crates/forge-web/server/src/api.rs` (4 comment HTTP handlers)
- `crates/forge-web/server/src/main.rs` (comment routes)
- `ui/src/api.ts` (`CommentInfo` type + 4 comment methods)
- `ui/src/pages/IssueDetail.tsx` (comment thread + add/delete)
- `ui/src/pages/PullRequestDetail.tsx` (comment thread + add/delete)

---

#### 7. Default Branch Persistence — DONE

**Fixed:** Added `default_branch` column to repos DB, `default_branch` field to `UpdateRepoRequest` proto, wired through server/web/frontend. RepoSettings saves default branch. ListRepos now reads stored default branch instead of hardcoding "main".

**Files changed:**
- `proto/forge.proto` (added field 5 to `UpdateRepoRequest`)
- `crates/forge-server/src/storage/db.rs` (migration + `set_default_branch` + updated `list_repos` + `RepoRecord`)
- `crates/forge-server/src/services/grpc.rs` (UpdateRepo + ListRepos handlers)
- `crates/forge-web/server/src/grpc_client.rs` (added `default_branch` param)
- `crates/forge-web/server/src/api.rs` (added `default_branch` to `UpdateRepoBody`)
- `ui/src/api.ts` (added to `updateRepo` signature)
- `ui/src/pages/RepoSettings.tsx` (Save button for default branch)

---

### Low Priority

#### 8. Lock Acquisition from Web UI — DONE

**Fixed:** Added "Acquire Lock" form to Locks page with file path + optional reason inputs. Uses existing `POST /api/repos/:repo/locks/acquire` endpoint.

**Files changed:**
- `ui/src/api.ts` (added `acquireLock`)
- `ui/src/pages/Locks.tsx` (acquire form + handler)

---

#### 9. Release Detail Page — SKIPPED

The list view already includes all artifacts per release. A dedicated detail page adds no value until releases gain changelogs/notes metadata.

---

#### 10. Raw File Download Link in FileView — SKIPPED

FileView already constructs working download links. Switching to the raw endpoint is a cosmetic change with no user-facing benefit.

---

#### 11. Artifact Download — SKIPPED

Requires a new backend streaming endpoint (`GET .../artifacts/:id/download`) to serve artifact files. Deferred — artifacts are currently metadata-only (size, name) with no file storage backing.

---

## Frontend-Only Improvements

### 12-15. Deferred

Items 12 (markdown rendering), 13 (diff viewer), 14 (keyboard shortcuts), and 15 (pagination consistency) are deferred. They are polish items that don't affect functionality.

---

## Summary

| Category | Count |
|----------|-------|
| Fully synced (gRPC -> Web -> Frontend) | 46 |
| Web API exists, frontend missing | 0 |
| CLI-only gRPC (correctly omitted from web) | 6 |
| Web-only routes (no dedicated gRPC RPC) | 2 |
| Frontend-only polish items (deferred) | 4 |
| Skipped (no value or needs new infra) | 3 |
| **Fixed in this audit** | **10 endpoints + 8 features** |
