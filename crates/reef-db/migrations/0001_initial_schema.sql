-- Reef schema. Portable SQL: runs unchanged on SQLite and PostgreSQL.
-- Conventions: every id is an application-generated BIGINT; timestamps are BIGINT epoch
-- milliseconds; booleans are BIGINT 0/1 (INTEGER where noted); JSON is TEXT.
--
-- Migrations expand and contract: add tables, columns, indexes and backfills, and only drop
-- or rename what the previous release no longer uses. Never edit an applied migration.

-- ---------------------------------------------------------------------------------------
-- Accounts: users and organizations share one login namespace, as on GitHub.
-- ---------------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS accounts (
  id            BIGINT PRIMARY KEY,
  kind          TEXT NOT NULL,              -- 'user' | 'org'
  login         TEXT NOT NULL,
  login_lower   TEXT NOT NULL,
  name          TEXT,
  email         TEXT,                       -- users: primary email
  email_lower   TEXT,
  bio           TEXT,
  company       TEXT,
  location      TEXT,
  blog          TEXT,
  avatar_url    TEXT,
  password_hash TEXT,                       -- users with native auth
  is_admin      BIGINT NOT NULL DEFAULT 0,  -- site administrator
  suspended     BIGINT NOT NULL DEFAULT 0,
  description   TEXT,                       -- orgs
  billing_email TEXT,                       -- orgs
  created_at    BIGINT NOT NULL,
  updated_at    BIGINT NOT NULL,
  next_project_number BIGINT NOT NULL DEFAULT 1,
  avatar_updated_at   BIGINT,
  external_org_id     TEXT                  -- orgs mirrored from the identity provider
);
CREATE UNIQUE INDEX IF NOT EXISTS accounts_login_lower ON accounts (login_lower);
CREATE UNIQUE INDEX IF NOT EXISTS accounts_email_lower ON accounts (email_lower);
CREATE UNIQUE INDEX IF NOT EXISTS accounts_external_org ON accounts (external_org_id) WHERE external_org_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS org_members (
  org_id     BIGINT NOT NULL,
  user_id    BIGINT NOT NULL,
  role       TEXT NOT NULL,                 -- 'admin' | 'member'
  created_at BIGINT NOT NULL,
  PRIMARY KEY (org_id, user_id)
);
CREATE INDEX IF NOT EXISTS org_members_user ON org_members (user_id);

-- Accounts linked to an OpenID Connect provider.
CREATE TABLE IF NOT EXISTS external_identities (
  user_id    BIGINT NOT NULL,
  issuer     TEXT NOT NULL,
  subject    TEXT NOT NULL,
  email      TEXT,
  created_at BIGINT NOT NULL,
  last_login_at BIGINT NOT NULL,
  PRIMARY KEY (issuer, subject)
);
CREATE INDEX IF NOT EXISTS external_identities_user ON external_identities (user_id);

-- Short-lived state of in-flight browser logins.
CREATE TABLE IF NOT EXISTS login_states (
  state      TEXT PRIMARY KEY,
  nonce      TEXT NOT NULL,
  pkce       TEXT NOT NULL,
  return_to  TEXT,
  created_at BIGINT NOT NULL
);

-- An identity from the provider that could not be matched to an account and is waiting for
-- the person to sign in and link it (or ask for a separate account). `sid`/`id_token` carry
-- the provider sign-in to the session that is opened once they do.
CREATE TABLE IF NOT EXISTS pending_identities (
  id             TEXT PRIMARY KEY,
  issuer         TEXT NOT NULL,
  subject        TEXT NOT NULL,
  email          TEXT,
  email_verified BIGINT NOT NULL DEFAULT 0,
  username       TEXT,
  name           TEXT,
  return_to      TEXT,
  created_at     BIGINT NOT NULL,
  sid            TEXT,
  id_token       TEXT
);

-- Multiple emails per account.
CREATE TABLE IF NOT EXISTS user_emails (
  id          BIGINT PRIMARY KEY,
  user_id     BIGINT NOT NULL,
  email       TEXT NOT NULL,
  email_lower TEXT NOT NULL,
  verified    INTEGER NOT NULL DEFAULT 0,
  is_primary  INTEGER NOT NULL DEFAULT 0,
  visibility  TEXT NOT NULL DEFAULT 'private',   -- 'public' | 'private'
  created_at  BIGINT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS user_emails_lower ON user_emails (email_lower);
CREATE INDEX IF NOT EXISTS user_emails_user ON user_emails (user_id);

-- GPG keys and SSH signing keys (commit signature verification).
CREATE TABLE IF NOT EXISTS gpg_keys (
  id                    BIGINT PRIMARY KEY,
  user_id               BIGINT NOT NULL,
  key_id                TEXT NOT NULL,        -- 16 hex chars, upper case
  primary_key_id        BIGINT,               -- NULL on primary keys; parent row id on subkey rows (unused today)
  fingerprint           TEXT NOT NULL,
  public_key            TEXT NOT NULL,        -- ASCII-armored
  emails                TEXT NOT NULL,        -- JSON array of user-id emails
  subkeys               TEXT NOT NULL,        -- JSON array of {key_id, fingerprint, can_sign, ...}
  can_sign              INTEGER NOT NULL DEFAULT 0,
  can_certify           INTEGER NOT NULL DEFAULT 0,
  can_encrypt_comms     INTEGER NOT NULL DEFAULT 0,
  can_encrypt_storage   INTEGER NOT NULL DEFAULT 0,
  expires_at            BIGINT,
  created_at            BIGINT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS gpg_keys_fingerprint ON gpg_keys (fingerprint);
CREATE INDEX IF NOT EXISTS gpg_keys_user ON gpg_keys (user_id);

CREATE TABLE IF NOT EXISTS ssh_signing_keys (
  id          BIGINT PRIMARY KEY,
  user_id     BIGINT NOT NULL,
  key         TEXT NOT NULL,                  -- "type base64"
  title       TEXT NOT NULL,
  fingerprint TEXT NOT NULL,
  created_at  BIGINT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS ssh_signing_keys_fingerprint ON ssh_signing_keys (fingerprint);
CREATE INDEX IF NOT EXISTS ssh_signing_keys_user ON ssh_signing_keys (user_id);

-- ---------------------------------------------------------------------------------------
-- Sessions, tokens, keys, device flow
-- ---------------------------------------------------------------------------------------
-- `sid` is the provider sign-in a web session was opened from (the ID token's `sid`), so
-- that a back-channel logout for that sign-in ends it here too; `id_token` is that sign-in's
-- ID token, the `id_token_hint` for the provider's end_session endpoint. Both NULL for
-- sessions not opened through OpenID Connect.
CREATE TABLE IF NOT EXISTS sessions (
  id          TEXT PRIMARY KEY,             -- sha256 of the cookie value
  user_id     BIGINT NOT NULL,
  created_at  BIGINT NOT NULL,
  expires_at  BIGINT NOT NULL,
  last_seen_at BIGINT NOT NULL,
  ip          TEXT,
  user_agent  TEXT,
  sid         TEXT,
  id_token    TEXT
);
CREATE INDEX IF NOT EXISTS sessions_user ON sessions (user_id);
CREATE INDEX IF NOT EXISTS sessions_sid ON sessions (sid) WHERE sid IS NOT NULL;

CREATE TABLE IF NOT EXISTS access_tokens (
  id           BIGINT PRIMARY KEY,
  user_id      BIGINT NOT NULL,
  kind         TEXT NOT NULL,               -- 'pat' | 'oauth'
  name         TEXT NOT NULL,
  token_hash   TEXT NOT NULL,
  token_last4  TEXT NOT NULL,
  scopes       TEXT NOT NULL,               -- comma separated GitHub scopes
  client_id    TEXT,                        -- oauth: the app that requested it
  created_at   BIGINT NOT NULL,
  expires_at   BIGINT,
  last_used_at BIGINT
);
CREATE UNIQUE INDEX IF NOT EXISTS access_tokens_hash ON access_tokens (token_hash);
CREATE INDEX IF NOT EXISTS access_tokens_user ON access_tokens (user_id);

CREATE TABLE IF NOT EXISTS ssh_keys (
  id          BIGINT PRIMARY KEY,
  user_id     BIGINT NOT NULL,
  title       TEXT NOT NULL,
  key_type    TEXT NOT NULL,
  public_key  TEXT NOT NULL,                -- "type base64" without comment
  fingerprint TEXT NOT NULL,                -- SHA256:...
  created_at  BIGINT NOT NULL,
  last_used_at BIGINT
);
CREATE UNIQUE INDEX IF NOT EXISTS ssh_keys_fingerprint ON ssh_keys (fingerprint);
CREATE INDEX IF NOT EXISTS ssh_keys_user ON ssh_keys (user_id);

-- OAuth device authorization grants (RFC 8628), used by `gh auth login`.
CREATE TABLE IF NOT EXISTS device_codes (
  device_code_hash TEXT PRIMARY KEY,
  user_code        TEXT NOT NULL,
  client_id        TEXT NOT NULL,
  scopes           TEXT NOT NULL,
  status           TEXT NOT NULL,           -- 'pending' | 'authorized' | 'denied' | 'redeemed'
  user_id          BIGINT,
  token_plain      TEXT,                    -- held until the client polls once, then cleared
  created_at       BIGINT NOT NULL,
  expires_at       BIGINT NOT NULL,
  last_poll_at     BIGINT
);
CREATE UNIQUE INDEX IF NOT EXISTS device_codes_user_code ON device_codes (user_code);

-- ---------------------------------------------------------------------------------------
-- Repositories
-- ---------------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS repositories (
  id               BIGINT PRIMARY KEY,
  owner_id         BIGINT NOT NULL,
  name             TEXT NOT NULL,
  name_lower       TEXT NOT NULL,
  description      TEXT,
  homepage         TEXT,
  private          BIGINT NOT NULL DEFAULT 0,
  default_branch   TEXT NOT NULL DEFAULT 'main',
  is_fork          BIGINT NOT NULL DEFAULT 0,
  parent_id        BIGINT,
  is_template      BIGINT NOT NULL DEFAULT 0,
  archived         BIGINT NOT NULL DEFAULT 0,
  has_issues       BIGINT NOT NULL DEFAULT 1,
  has_projects     BIGINT NOT NULL DEFAULT 1,
  has_wiki         BIGINT NOT NULL DEFAULT 1,
  has_discussions  BIGINT NOT NULL DEFAULT 0,
  allow_squash_merge BIGINT NOT NULL DEFAULT 1,
  allow_merge_commit BIGINT NOT NULL DEFAULT 1,
  allow_rebase_merge BIGINT NOT NULL DEFAULT 1,
  allow_auto_merge   BIGINT NOT NULL DEFAULT 0,
  delete_branch_on_merge BIGINT NOT NULL DEFAULT 0,
  stars_count      BIGINT NOT NULL DEFAULT 0,
  forks_count      BIGINT NOT NULL DEFAULT 0,
  watchers_count   BIGINT NOT NULL DEFAULT 0,
  open_issues_count BIGINT NOT NULL DEFAULT 0,
  size_kb          BIGINT NOT NULL DEFAULT 0,
  is_empty         BIGINT NOT NULL DEFAULT 1,
  created_at       BIGINT NOT NULL,
  updated_at       BIGINT NOT NULL,
  pushed_at        BIGINT,
  next_issue_number     BIGINT NOT NULL DEFAULT 1,
  next_milestone_number BIGINT NOT NULL DEFAULT 1,
  open_pulls_count      BIGINT NOT NULL DEFAULT 0,
  next_run_number       BIGINT NOT NULL DEFAULT 1,
  topics                TEXT NOT NULL DEFAULT '[]',   -- JSON array
  language              TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS repositories_owner_name ON repositories (owner_id, name_lower);

CREATE TABLE IF NOT EXISTS collaborators (
  repo_id    BIGINT NOT NULL,
  user_id    BIGINT NOT NULL,
  permission TEXT NOT NULL,                 -- 'read' | 'triage' | 'write' | 'maintain' | 'admin'
  created_at BIGINT NOT NULL,
  PRIMARY KEY (repo_id, user_id)
);
CREATE INDEX IF NOT EXISTS collaborators_user ON collaborators (user_id);

-- Repository events that hooks and the activity feed consume.
CREATE TABLE IF NOT EXISTS repo_events (
  id         BIGINT PRIMARY KEY,
  repo_id    BIGINT NOT NULL,
  actor_id   BIGINT,
  kind       TEXT NOT NULL,                 -- 'push' | 'create' | 'delete' | ...
  payload    TEXT NOT NULL,                 -- JSON
  created_at BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS repo_events_repo ON repo_events (repo_id, id);

-- Branch protection rules. `pattern` is a glob over branch names (main, release/*).
CREATE TABLE IF NOT EXISTS branch_protections (
  id                              BIGINT PRIMARY KEY,
  repo_id                         BIGINT NOT NULL,
  pattern                         TEXT NOT NULL,
  allow_force_pushes              BIGINT NOT NULL DEFAULT 0,
  allow_deletions                 BIGINT NOT NULL DEFAULT 0,
  require_pull_request            BIGINT NOT NULL DEFAULT 0,
  required_approving_review_count BIGINT NOT NULL DEFAULT 0,
  dismiss_stale_reviews           BIGINT NOT NULL DEFAULT 0,
  require_code_owner_reviews      BIGINT NOT NULL DEFAULT 0,
  enforce_admins                  BIGINT NOT NULL DEFAULT 0,
  required_status_checks          TEXT NOT NULL DEFAULT '[]',   -- JSON array of context names
  strict_status_checks            BIGINT NOT NULL DEFAULT 0,
  require_linear_history          BIGINT NOT NULL DEFAULT 0,
  require_signed_commits          BIGINT NOT NULL DEFAULT 0,
  lock_branch                     BIGINT NOT NULL DEFAULT 0,
  created_at                      BIGINT NOT NULL,
  updated_at                      BIGINT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS branch_protections_repo_pattern ON branch_protections (repo_id, pattern);

-- Releases.
CREATE TABLE IF NOT EXISTS releases (
  id                BIGINT PRIMARY KEY,
  repo_id           BIGINT NOT NULL,
  tag_name          TEXT NOT NULL,
  target_commitish  TEXT NOT NULL,
  name              TEXT,
  body              TEXT,
  draft             BIGINT NOT NULL DEFAULT 0,
  prerelease        BIGINT NOT NULL DEFAULT 0,
  author_id         BIGINT,
  created_at        BIGINT NOT NULL,
  updated_at        BIGINT NOT NULL,
  published_at      BIGINT
);
CREATE UNIQUE INDEX IF NOT EXISTS releases_repo_tag ON releases (repo_id, tag_name);
CREATE INDEX IF NOT EXISTS releases_repo ON releases (repo_id, created_at);

-- Stars, watching, pinned repositories.
CREATE TABLE IF NOT EXISTS stars (
  user_id    BIGINT NOT NULL,
  repo_id    BIGINT NOT NULL,
  created_at BIGINT NOT NULL,
  PRIMARY KEY (user_id, repo_id)
);
CREATE INDEX IF NOT EXISTS stars_repo ON stars (repo_id);

CREATE TABLE IF NOT EXISTS watches (
  user_id    BIGINT NOT NULL,
  repo_id    BIGINT NOT NULL,
  ignored    BIGINT NOT NULL DEFAULT 0,   -- 1 = explicitly ignoring notifications
  created_at BIGINT NOT NULL,
  PRIMARY KEY (user_id, repo_id)
);
CREATE INDEX IF NOT EXISTS watches_repo ON watches (repo_id);

CREATE TABLE IF NOT EXISTS pinned_repos (
  owner_id   BIGINT NOT NULL,
  repo_id    BIGINT NOT NULL,
  position   BIGINT NOT NULL DEFAULT 0,
  PRIMARY KEY (owner_id, repo_id)
);

-- Git LFS: objects stored under <data_dir>/lfs and file locks.
CREATE TABLE IF NOT EXISTS lfs_objects (
  repo_id    BIGINT NOT NULL,
  oid        TEXT NOT NULL,
  size       BIGINT NOT NULL,
  created_at BIGINT NOT NULL,
  PRIMARY KEY (repo_id, oid)
);
CREATE TABLE IF NOT EXISTS lfs_locks (
  id         BIGINT PRIMARY KEY,
  repo_id    BIGINT NOT NULL,
  path       TEXT NOT NULL,
  owner_id   BIGINT NOT NULL,
  created_at BIGINT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS lfs_locks_repo_path ON lfs_locks (repo_id, path);

-- Pull mirrors, push mirrors and source imports.
CREATE TABLE IF NOT EXISTS mirror_config (
  repo_id          BIGINT PRIMARY KEY,
  pull_url         TEXT NOT NULL,
  username         TEXT,
  password         TEXT,
  interval_minutes BIGINT NOT NULL DEFAULT 60,
  last_synced_at   BIGINT,
  last_error       TEXT,
  next_sync_at     BIGINT,
  created_at       BIGINT NOT NULL,
  updated_at       BIGINT NOT NULL,
  sync_lock_until  BIGINT,                    -- lease (epoch ms) held by the instance syncing it
  sync_requested   BIGINT NOT NULL DEFAULT 0  -- a "sync now" that arrived while the lease was held
);
CREATE TABLE IF NOT EXISTS push_mirrors (
  id              BIGINT PRIMARY KEY,
  repo_id         BIGINT NOT NULL,
  url             TEXT NOT NULL,
  username        TEXT,
  password        TEXT,
  pending         BIGINT NOT NULL DEFAULT 0,
  last_attempt_at BIGINT,
  last_pushed_at  BIGINT,
  last_error      TEXT,
  created_at      BIGINT NOT NULL,
  lock_until      BIGINT,                     -- lease (epoch ms) held by the instance pushing it
  pending_gen     BIGINT NOT NULL DEFAULT 0   -- bumped by every "needs a push" mark; a push clears `pending` only if none arrived meanwhile
);
CREATE INDEX IF NOT EXISTS push_mirrors_repo ON push_mirrors (repo_id);
CREATE TABLE IF NOT EXISTS repo_imports (
  repo_id      BIGINT PRIMARY KEY,
  vcs          TEXT NOT NULL,
  vcs_url      TEXT NOT NULL,
  username     TEXT,
  password     TEXT,
  status       TEXT NOT NULL,
  message      TEXT,
  commit_count BIGINT,
  actor_id     BIGINT,
  started_at   BIGINT NOT NULL,
  finished_at  BIGINT
);

-- ---------------------------------------------------------------------------------------
-- Issues, labels, milestones, comments, timeline events, reactions.
-- Pull requests are issues with is_pull = 1 plus a row in `pulls`.
-- ---------------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS labels (
  id          BIGINT PRIMARY KEY,
  repo_id     BIGINT NOT NULL,
  name        TEXT NOT NULL,
  name_lower  TEXT NOT NULL,
  color       TEXT NOT NULL,              -- 6 hex digits, no '#'
  description TEXT,
  is_default  BIGINT NOT NULL DEFAULT 0,
  created_at  BIGINT NOT NULL,
  updated_at  BIGINT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS labels_repo_name ON labels (repo_id, name_lower);

CREATE TABLE IF NOT EXISTS milestones (
  id          BIGINT PRIMARY KEY,
  repo_id     BIGINT NOT NULL,
  number      BIGINT NOT NULL,
  title       TEXT NOT NULL,
  description TEXT,
  state       TEXT NOT NULL DEFAULT 'open',
  due_on      BIGINT,
  created_at  BIGINT NOT NULL,
  updated_at  BIGINT NOT NULL,
  closed_at   BIGINT
);
CREATE UNIQUE INDEX IF NOT EXISTS milestones_repo_number ON milestones (repo_id, number);

CREATE TABLE IF NOT EXISTS issues (
  id             BIGINT PRIMARY KEY,
  repo_id        BIGINT NOT NULL,
  number         BIGINT NOT NULL,
  is_pull        BIGINT NOT NULL DEFAULT 0,
  title          TEXT NOT NULL,
  body           TEXT,
  state          TEXT NOT NULL DEFAULT 'open',   -- 'open' | 'closed'
  state_reason   TEXT,                           -- 'completed' | 'not_planned' | 'reopened' | 'duplicate'
  author_id      BIGINT NOT NULL,
  milestone_id   BIGINT,
  locked         BIGINT NOT NULL DEFAULT 0,
  lock_reason    TEXT,
  comments_count BIGINT NOT NULL DEFAULT 0,
  created_at     BIGINT NOT NULL,
  updated_at     BIGINT NOT NULL,
  closed_at      BIGINT,
  closed_by_id   BIGINT
);
CREATE UNIQUE INDEX IF NOT EXISTS issues_repo_number ON issues (repo_id, number);
CREATE INDEX IF NOT EXISTS issues_repo_state ON issues (repo_id, is_pull, state, updated_at);
CREATE INDEX IF NOT EXISTS issues_author ON issues (author_id);

CREATE TABLE IF NOT EXISTS issue_labels (
  issue_id BIGINT NOT NULL,
  label_id BIGINT NOT NULL,
  PRIMARY KEY (issue_id, label_id)
);
CREATE INDEX IF NOT EXISTS issue_labels_label ON issue_labels (label_id);

CREATE TABLE IF NOT EXISTS issue_assignees (
  issue_id BIGINT NOT NULL,
  user_id  BIGINT NOT NULL,
  PRIMARY KEY (issue_id, user_id)
);
CREATE INDEX IF NOT EXISTS issue_assignees_user ON issue_assignees (user_id);

CREATE TABLE IF NOT EXISTS issue_comments (
  id         BIGINT PRIMARY KEY,
  issue_id   BIGINT NOT NULL,
  repo_id    BIGINT NOT NULL,
  author_id  BIGINT NOT NULL,
  body       TEXT NOT NULL,
  created_at BIGINT NOT NULL,
  updated_at BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS issue_comments_issue ON issue_comments (issue_id, id);

-- Timeline events: closed, reopened, labeled, unlabeled, assigned, unassigned, milestoned,
-- demilestoned, renamed, locked, unlocked, referenced, cross-referenced, ...
CREATE TABLE IF NOT EXISTS issue_events (
  id         BIGINT PRIMARY KEY,
  issue_id   BIGINT NOT NULL,
  repo_id    BIGINT NOT NULL,
  actor_id   BIGINT,
  event      TEXT NOT NULL,
  payload    TEXT NOT NULL DEFAULT '{}',        -- JSON: label, assignee, milestone, rename, commit_id, ...
  created_at BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS issue_events_issue ON issue_events (issue_id, id);

CREATE TABLE IF NOT EXISTS reactions (
  id           BIGINT PRIMARY KEY,
  subject_type TEXT NOT NULL,                  -- 'issue' | 'issue_comment' | ...
  subject_id   BIGINT NOT NULL,
  user_id      BIGINT NOT NULL,
  content      TEXT NOT NULL,                  -- '+1' '-1' 'laugh' 'hooray' 'confused' 'heart' 'rocket' 'eyes'
  created_at   BIGINT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS reactions_unique ON reactions (subject_type, subject_id, user_id, content);

-- ---------------------------------------------------------------------------------------
-- Pull requests: an issue row (is_pull = 1) plus this metadata, reviews, review requests,
-- review (line) comments, and commit statuses.
-- ---------------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS pulls (
  issue_id              BIGINT PRIMARY KEY,
  repo_id               BIGINT NOT NULL,
  head_repo_id          BIGINT NOT NULL,
  head_ref              TEXT NOT NULL,
  head_sha              TEXT NOT NULL,
  base_ref              TEXT NOT NULL,
  base_sha              TEXT NOT NULL,
  draft                 BIGINT NOT NULL DEFAULT 0,
  merged                BIGINT NOT NULL DEFAULT 0,
  merged_at             BIGINT,
  merged_by_id          BIGINT,
  merge_commit_sha      TEXT,
  merge_method          TEXT,
  mergeable             TEXT NOT NULL DEFAULT 'unknown',  -- 'mergeable' | 'conflicting' | 'unknown'
  mergeable_sha         TEXT,                             -- head+base the flag was computed for
  additions             BIGINT NOT NULL DEFAULT 0,
  deletions             BIGINT NOT NULL DEFAULT 0,
  changed_files         BIGINT NOT NULL DEFAULT 0,
  commits_count         BIGINT NOT NULL DEFAULT 0,
  maintainer_can_modify BIGINT NOT NULL DEFAULT 0,
  auto_merge            TEXT
);
CREATE INDEX IF NOT EXISTS pulls_repo_head ON pulls (repo_id, head_ref);
CREATE INDEX IF NOT EXISTS pulls_repo_base ON pulls (repo_id, base_ref);

CREATE TABLE IF NOT EXISTS pull_reviews (
  id           BIGINT PRIMARY KEY,
  pull_id      BIGINT NOT NULL,
  repo_id      BIGINT NOT NULL,
  author_id    BIGINT NOT NULL,
  state        TEXT NOT NULL,     -- 'pending' | 'commented' | 'approved' | 'changes_requested' | 'dismissed'
  body         TEXT,
  commit_sha   TEXT,
  submitted_at BIGINT,
  created_at   BIGINT NOT NULL,
  updated_at   BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS pull_reviews_pull ON pull_reviews (pull_id, id);

CREATE TABLE IF NOT EXISTS pull_review_requests (
  pull_id    BIGINT NOT NULL,
  user_id    BIGINT NOT NULL,
  created_at BIGINT NOT NULL,
  PRIMARY KEY (pull_id, user_id)
);

CREATE TABLE IF NOT EXISTS review_comments (
  id             BIGINT PRIMARY KEY,
  pull_id        BIGINT NOT NULL,
  repo_id        BIGINT NOT NULL,
  review_id      BIGINT,
  author_id      BIGINT NOT NULL,
  body           TEXT NOT NULL,
  path           TEXT NOT NULL,
  line           BIGINT,
  start_line     BIGINT,
  side           TEXT NOT NULL DEFAULT 'RIGHT',
  commit_sha     TEXT NOT NULL,
  in_reply_to_id BIGINT,
  created_at     BIGINT NOT NULL,
  updated_at     BIGINT NOT NULL,
  resolved       INTEGER NOT NULL DEFAULT 0,
  resolved_by_id BIGINT
);
CREATE INDEX IF NOT EXISTS review_comments_pull ON review_comments (pull_id, id);

CREATE TABLE IF NOT EXISTS commit_statuses (
  id          BIGINT PRIMARY KEY,
  repo_id     BIGINT NOT NULL,
  sha         TEXT NOT NULL,
  state       TEXT NOT NULL,       -- 'error' | 'failure' | 'pending' | 'success'
  context     TEXT NOT NULL,
  description TEXT,
  target_url  TEXT,
  creator_id  BIGINT,
  created_at  BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS commit_statuses_sha ON commit_statuses (repo_id, sha, id);

-- ---------------------------------------------------------------------------------------
-- Projects (GitHub Projects V2 shape): a project belongs to a user or organization, has
-- fields (Status is a single-select), items (issues, pull requests, drafts) with field
-- values, views (board, table), workflows, status updates and item history.
-- ---------------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS projects (
  id           BIGINT PRIMARY KEY,
  owner_id     BIGINT NOT NULL,
  number       BIGINT NOT NULL,
  title        TEXT NOT NULL,
  short_description TEXT,
  readme       TEXT,
  public       BIGINT NOT NULL DEFAULT 0,
  closed       BIGINT NOT NULL DEFAULT 0,
  creator_id   BIGINT NOT NULL,
  created_at   BIGINT NOT NULL,
  updated_at   BIGINT NOT NULL,
  closed_at    BIGINT
);
CREATE UNIQUE INDEX IF NOT EXISTS projects_owner_number ON projects (owner_id, number);

CREATE TABLE IF NOT EXISTS project_fields (
  id         BIGINT PRIMARY KEY,
  project_id BIGINT NOT NULL,
  name       TEXT NOT NULL,
  data_type  TEXT NOT NULL,        -- 'single_select' | 'text' | 'number' | 'date' | 'iteration'
  options    TEXT NOT NULL DEFAULT '[]',   -- JSON [{id, name, color, description}]
  position   BIGINT NOT NULL DEFAULT 0,
  created_at BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS project_fields_project ON project_fields (project_id, position);

CREATE TABLE IF NOT EXISTS project_items (
  id           BIGINT PRIMARY KEY,
  project_id   BIGINT NOT NULL,
  content_type TEXT NOT NULL,      -- 'issue' | 'pull' | 'draft'
  content_id   BIGINT,             -- issues.id for issue/pull
  draft_title  TEXT,
  draft_body   TEXT,
  position     BIGINT NOT NULL DEFAULT 0,
  archived     BIGINT NOT NULL DEFAULT 0,
  creator_id   BIGINT,
  created_at   BIGINT NOT NULL,
  updated_at   BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS project_items_project ON project_items (project_id, position);
CREATE UNIQUE INDEX IF NOT EXISTS project_items_content ON project_items (project_id, content_type, content_id);

CREATE TABLE IF NOT EXISTS project_item_values (
  item_id  BIGINT NOT NULL,
  field_id BIGINT NOT NULL,
  value    TEXT,                    -- option id for single_select, text, number, ISO date
  PRIMARY KEY (item_id, field_id)
);

-- `settings` is one JSON document per view: {column_field_id, group_by_field_id, sort:
-- {field_id, direction}, visible_fields: [ids], filter}.
CREATE TABLE IF NOT EXISTS project_views (
  id         BIGINT PRIMARY KEY,
  project_id BIGINT NOT NULL,
  number     BIGINT NOT NULL,
  name       TEXT NOT NULL,
  layout     TEXT NOT NULL DEFAULT 'board',   -- 'board' | 'table'
  group_by_field_id BIGINT,
  filter     TEXT,
  position   BIGINT NOT NULL DEFAULT 0,
  created_at BIGINT NOT NULL,
  settings   TEXT NOT NULL DEFAULT '{}'
);

-- Repositories linked to a project (GitHub: "linked repositories").
CREATE TABLE IF NOT EXISTS project_repositories (
  project_id BIGINT NOT NULL,
  repo_id    BIGINT NOT NULL,
  PRIMARY KEY (project_id, repo_id)
);

CREATE TABLE IF NOT EXISTS project_workflows (
  id         BIGINT PRIMARY KEY,
  project_id BIGINT NOT NULL,
  kind       TEXT NOT NULL,        -- item_added | item_reopened | item_closed | changes_requested | review_approved | pull_merged | auto_archive | auto_add | auto_close
  enabled    BIGINT NOT NULL DEFAULT 1,
  config     TEXT NOT NULL DEFAULT '{}',   -- JSON: {status: option id} | {filter} | {repo, filter}
  created_at BIGINT NOT NULL,
  updated_at BIGINT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS project_workflows_kind ON project_workflows (project_id, kind);

CREATE TABLE IF NOT EXISTS project_status_updates (
  id          BIGINT PRIMARY KEY,
  project_id  BIGINT NOT NULL,
  author_id   BIGINT NOT NULL,
  status      TEXT NOT NULL,       -- INACTIVE | ON_TRACK | AT_RISK | OFF_TRACK | COMPLETE
  start_date  TEXT,
  target_date TEXT,
  body        TEXT,
  created_at  BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS project_status_updates_project ON project_status_updates (project_id, created_at);

-- What happened to items over time, so insights can rebuild the state at any date.
CREATE TABLE IF NOT EXISTS project_item_history (
  id         BIGINT PRIMARY KEY,
  project_id BIGINT NOT NULL,
  item_id    BIGINT NOT NULL,
  kind       TEXT NOT NULL,        -- added | removed | archived | unarchived | value
  field_id   BIGINT,
  value      TEXT,
  prev_value TEXT,
  created_at BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS project_item_history_project ON project_item_history (project_id, created_at);

-- ---------------------------------------------------------------------------------------
-- Actions: workflows, runs, jobs, logs, artifacts, secrets, and the runners that execute
-- jobs (the stock actions/runner, speaking its native protocol).
-- ---------------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS runners (
  id            BIGINT PRIMARY KEY,
  scope_kind    TEXT NOT NULL,            -- 'repo' | 'org'
  scope_id      BIGINT NOT NULL,
  name          TEXT NOT NULL,
  os            TEXT,
  version       TEXT,
  labels        TEXT NOT NULL,            -- JSON array of strings
  ephemeral     BIGINT NOT NULL DEFAULT 0,
  disable_update BIGINT NOT NULL DEFAULT 0,
  client_id     TEXT NOT NULL,            -- OAuth client id (GUID) the runner authenticates as
  public_key    TEXT NOT NULL,            -- JSON {exponent, modulus} (base64) of the runner's RSA key
  agent_json    TEXT,                     -- the TaskAgent document as registered
  busy          BIGINT NOT NULL DEFAULT 0,
  created_at    BIGINT NOT NULL,
  updated_at    BIGINT NOT NULL,
  last_seen_at  BIGINT
);
CREATE UNIQUE INDEX IF NOT EXISTS runners_scope_name ON runners (scope_kind, scope_id, name);
CREATE UNIQUE INDEX IF NOT EXISTS runners_client ON runners (client_id);

-- Registration / removal tokens handed to `config.sh --token`.
CREATE TABLE IF NOT EXISTS runner_tokens (
  token_hash  TEXT PRIMARY KEY,
  scope_kind  TEXT NOT NULL,
  scope_id    BIGINT NOT NULL,
  purpose     TEXT NOT NULL,              -- 'register' | 'remove'
  created_by  BIGINT,
  expires_at  BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS runner_sessions (
  session_id     TEXT PRIMARY KEY,        -- GUID
  runner_id      BIGINT NOT NULL,
  encryption_key TEXT NOT NULL,           -- base64 AES-256 key (stored plain; sent RSA-encrypted)
  owner_name     TEXT,
  created_at     BIGINT NOT NULL,
  last_poll_at   BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS runner_sessions_runner ON runner_sessions (runner_id);

CREATE TABLE IF NOT EXISTS workflows (
  id          BIGINT PRIMARY KEY,
  repo_id     BIGINT NOT NULL,
  path        TEXT NOT NULL,              -- .github/workflows/ci.yml
  name        TEXT NOT NULL,
  state       TEXT NOT NULL,              -- active | disabled_manually | deleted
  created_at  BIGINT NOT NULL,
  updated_at  BIGINT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS workflows_repo_path ON workflows (repo_id, path);

CREATE TABLE IF NOT EXISTS workflow_runs (
  id                  BIGINT PRIMARY KEY,
  repo_id             BIGINT NOT NULL,
  workflow_id         BIGINT NOT NULL,
  run_number          BIGINT NOT NULL,
  run_attempt         BIGINT NOT NULL DEFAULT 1,
  name                TEXT NOT NULL,      -- workflow name
  display_title       TEXT NOT NULL,      -- run-name / commit title / PR title
  event               TEXT NOT NULL,
  status              TEXT NOT NULL,      -- queued | in_progress | completed
  conclusion          TEXT,               -- success | failure | cancelled | skipped | startup_failure | ...
  head_sha            TEXT NOT NULL,
  head_branch         TEXT,
  git_ref             TEXT NOT NULL,
  path                TEXT NOT NULL,
  actor_id            BIGINT,
  triggering_actor_id BIGINT,
  event_payload       TEXT NOT NULL,      -- JSON: the `github.event` context
  workflow_yaml       TEXT NOT NULL,      -- snapshot of the file at head_sha
  pull_number         BIGINT,
  inputs              TEXT,               -- JSON (workflow_dispatch)
  error               TEXT,               -- why the run could not start
  created_at          BIGINT NOT NULL,
  updated_at          BIGINT NOT NULL,
  started_at          BIGINT,
  completed_at        BIGINT
);
CREATE INDEX IF NOT EXISTS workflow_runs_repo ON workflow_runs (repo_id, created_at);
CREATE INDEX IF NOT EXISTS workflow_runs_sha ON workflow_runs (repo_id, head_sha);
CREATE INDEX IF NOT EXISTS workflow_runs_workflow ON workflow_runs (workflow_id, created_at);
CREATE INDEX IF NOT EXISTS workflow_runs_created ON workflow_runs (created_at);
CREATE INDEX IF NOT EXISTS workflow_runs_completed ON workflow_runs (completed_at);
-- Retention walks completed runs by when they finished, COALESCE(completed_at, updated_at),
-- then id: an expression index in exactly that shape keeps the walk off a scan.
CREATE INDEX IF NOT EXISTS workflow_runs_finished ON workflow_runs ((COALESCE(completed_at, updated_at)), id) WHERE status = 'completed';

CREATE TABLE IF NOT EXISTS workflow_jobs (
  id            BIGINT PRIMARY KEY,
  run_id        BIGINT NOT NULL,
  repo_id       BIGINT NOT NULL,
  run_attempt   BIGINT NOT NULL DEFAULT 1,
  job_key       TEXT NOT NULL,            -- key under `jobs:`
  name          TEXT NOT NULL,            -- display name (with matrix leg)
  matrix        TEXT,                     -- JSON combination
  needs         TEXT NOT NULL,            -- JSON array of job keys
  labels        TEXT NOT NULL,            -- JSON array of runs-on labels ('[]' until resolved)
  status        TEXT NOT NULL,            -- waiting | queued | in_progress | completed
  conclusion    TEXT,                     -- success | failure | cancelled | skipped | timed_out
  runner_id     BIGINT,
  runner_name   TEXT,
  request_id    BIGINT,
  lock_token    TEXT,
  locked_until  BIGINT,
  cancel_requested BIGINT NOT NULL DEFAULT 0,
  plan_id       TEXT NOT NULL,            -- GUIDs the runner protocol needs
  timeline_id   TEXT NOT NULL,
  job_guid      TEXT NOT NULL,
  message       TEXT,                     -- JSON AgentJobRequestMessage once dispatched
  timeline      TEXT,                     -- JSON array of timeline records (steps) as reported
  outputs       TEXT,                     -- JSON map
  order_index   BIGINT NOT NULL,
  created_at    BIGINT NOT NULL,
  updated_at    BIGINT NOT NULL,
  started_at    BIGINT,
  completed_at  BIGINT
);
CREATE INDEX IF NOT EXISTS workflow_jobs_run ON workflow_jobs (run_id);
CREATE INDEX IF NOT EXISTS workflow_jobs_status ON workflow_jobs (status);
CREATE UNIQUE INDEX IF NOT EXISTS workflow_jobs_request ON workflow_jobs (request_id);

CREATE TABLE IF NOT EXISTS job_logs (
  id          BIGINT PRIMARY KEY,
  job_id      BIGINT NOT NULL,
  path        TEXT NOT NULL,              -- relative to <data_dir>/actions
  line_count  BIGINT NOT NULL DEFAULT 0,
  created_at  BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS job_logs_job ON job_logs (job_id);

-- Per-job GITHUB_TOKEN.
CREATE TABLE IF NOT EXISTS job_tokens (
  token_hash  TEXT PRIMARY KEY,
  job_id      BIGINT NOT NULL,
  repo_id     BIGINT NOT NULL,
  permissions TEXT NOT NULL,              -- JSON map scope -> read|write
  expires_at  BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS artifacts (
  id          BIGINT PRIMARY KEY,
  run_id      BIGINT NOT NULL,
  repo_id     BIGINT NOT NULL,
  name        TEXT NOT NULL,
  size        BIGINT NOT NULL DEFAULT 0,
  path        TEXT,                       -- zip on disk, relative to <data_dir>/actions
  expired     BIGINT NOT NULL DEFAULT 0,
  expires_at  BIGINT,
  created_at  BIGINT NOT NULL,
  updated_at  BIGINT NOT NULL,
  retry_after BIGINT                       -- when retention retries a file removal that failed
);
CREATE INDEX IF NOT EXISTS artifacts_run ON artifacts (run_id);
CREATE INDEX IF NOT EXISTS artifacts_expires ON artifacts (expires_at);

CREATE TABLE IF NOT EXISTS actions_secrets (
  scope_kind  TEXT NOT NULL,              -- 'repo' | 'org'
  scope_id    BIGINT NOT NULL,
  name        TEXT NOT NULL,
  value_enc   TEXT NOT NULL,              -- AES-GCM under the instance key
  created_at  BIGINT NOT NULL,
  updated_at  BIGINT NOT NULL,
  PRIMARY KEY (scope_kind, scope_id, name)
);

CREATE TABLE IF NOT EXISTS actions_variables (
  scope_kind  TEXT NOT NULL,
  scope_id    BIGINT NOT NULL,
  name        TEXT NOT NULL,
  value       TEXT NOT NULL,
  created_at  BIGINT NOT NULL,
  updated_at  BIGINT NOT NULL,
  PRIMARY KEY (scope_kind, scope_id, name)
);

-- ---------------------------------------------------------------------------------------
-- Notifications, site settings, webhooks
-- ---------------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS notifications (
  id           BIGINT PRIMARY KEY,
  user_id      BIGINT NOT NULL,
  actor_id     BIGINT,
  repo_id      BIGINT,
  subject_type TEXT NOT NULL,             -- 'issue' | 'pull' | 'release' | 'run' | 'system'
  subject_id   BIGINT,
  reason       TEXT NOT NULL,             -- mention | assign | review_requested | author | comment | state_change | subscribed | ci_activity
  title        TEXT NOT NULL,
  url          TEXT NOT NULL,             -- site-relative path
  unread       BIGINT NOT NULL DEFAULT 1,
  created_at   BIGINT NOT NULL,
  updated_at   BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS notifications_user ON notifications (user_id, unread, updated_at);
CREATE INDEX IF NOT EXISTS notifications_subject ON notifications (user_id, subject_type, subject_id);

-- Settings changed from the admin console; they override the file until reset.
CREATE TABLE IF NOT EXISTS site_settings (
  key        TEXT PRIMARY KEY,
  value      TEXT NOT NULL,
  updated_at BIGINT NOT NULL,
  updated_by BIGINT
);

CREATE TABLE IF NOT EXISTS webhooks (
  id                BIGINT PRIMARY KEY,
  owner_kind        TEXT NOT NULL,              -- 'repo' | 'org'
  owner_id          BIGINT NOT NULL,
  url               TEXT NOT NULL,
  content_type      TEXT NOT NULL DEFAULT 'json',   -- 'json' | 'form'
  secret            TEXT,
  events            TEXT NOT NULL DEFAULT '["push"]', -- JSON array; ["*"] = everything
  active            BIGINT NOT NULL DEFAULT 1,
  insecure_ssl      BIGINT NOT NULL DEFAULT 0,
  last_status       BIGINT,
  last_delivered_at BIGINT,
  created_at        BIGINT NOT NULL,
  updated_at        BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS webhooks_owner ON webhooks (owner_kind, owner_id);

CREATE TABLE IF NOT EXISTS webhook_deliveries (
  id               BIGINT PRIMARY KEY,
  hook_id          BIGINT NOT NULL,
  guid             TEXT NOT NULL,
  event            TEXT NOT NULL,
  action           TEXT,
  payload          TEXT NOT NULL,              -- JSON
  request_headers  TEXT NOT NULL DEFAULT '{}', -- JSON object
  response_status  BIGINT,
  response_headers TEXT,                       -- JSON object
  response_body    TEXT,                       -- truncated to 64 KB
  duration_ms      BIGINT,
  status           TEXT NOT NULL DEFAULT 'pending', -- 'pending' | 'delivered' | 'failed'
  attempt          BIGINT NOT NULL DEFAULT 0,
  next_attempt_at  BIGINT,
  lock_until       BIGINT,
  redelivery_of    BIGINT,
  repo_id          BIGINT,
  delivered_at     BIGINT,
  created_at       BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS webhook_deliveries_hook ON webhook_deliveries (hook_id, id);
CREATE INDEX IF NOT EXISTS webhook_deliveries_pending ON webhook_deliveries (status, next_attempt_at);

-- Webhook outbox. Recording a repository event, an issue event or an issue comment inserts a
-- row here in the same transaction, so the webhook poller sees every event exactly when it
-- commits, whatever order ids and commits interleave in across instances (an id cursor would
-- skip an event with a smaller id that commits after a larger one was polled). The poller
-- claims rows under a lease, and deletes each row in the transaction that records its
-- deliveries.
CREATE TABLE IF NOT EXISTS webhook_outbox (
  id         BIGINT PRIMARY KEY,
  source     TEXT NOT NULL,       -- 'repo_events' | 'issue_events' | 'issue_comments'
  event_id   BIGINT NOT NULL,
  created_at BIGINT NOT NULL,
  claim      TEXT,                -- the claiming poller's token while leased
  lock_until BIGINT
);
CREATE UNIQUE INDEX IF NOT EXISTS webhook_outbox_event ON webhook_outbox (source, event_id);
