create table github_installations (
  id bigint primary key,                 -- installation.id
  account_login text not null,
  revoked_at timestamptz                 -- installation deleted or suspended
);
create table github_repos (
  id bigint primary key,                 -- repository.id; never key by name
  full_name text not null,               -- current name, updated on rename/transfer
  installation_id bigint references github_installations(id)
);
create table project_repos (
  project_id text not null references projects(id),
  github_repo_id bigint not null references github_repos(id),
  linked_at timestamptz not null default now(),
  unlinked_at timestamptz,
  primary key (project_id, github_repo_id, linked_at)
);
create unique index project_repos_one_live on project_repos (github_repo_id) where unlinked_at is null;
create table repo_events (                -- audit trail shown on the page: renamed, transferred, removed, uninstalled
  id bigserial primary key, github_repo_id bigint not null, kind text not null,
  detail text not null, at timestamptz not null default now()
);
create table github_user_tokens (         -- short-lived proof of access; deleted on github_app_authorization.revoked
  user_id text primary key references users(id) on delete cascade,
  github_user_id bigint not null unique,
  access_token text not null,            -- needed raw to call GitHub; expires per GitHub (8 h)
  expires_at timestamptz not null
);
create table webhook_deliveries (
  delivery_id text primary key, event text not null, received_at timestamptz not null default now()
);
create table push_receipts (
  delivery_id text not null references webhook_deliveries(delivery_id),
  sha text not null,
  github_repo_id bigint not null,
  project_id text references projects(id),   -- null when the repo had no live link at receive time
  ref text not null,
  is_head boolean not null,
  forced boolean not null,
  claimed_ts bigint,                         -- commit timestamp from the payload, ms; untrusted
  received_at timestamptz not null default now(),
  primary key (delivery_id, sha)
);
create index push_receipts_project on push_receipts (project_id, received_at);
-- append-only, as plan 06 does for ledger_entries and commits
create trigger push_receipts_no_update before update or delete on push_receipts for each row execute function tally_append_only();
create trigger push_receipts_no_truncate before truncate on push_receipts for each statement execute function tally_append_only();
create trigger webhook_deliveries_no_update before update or delete on webhook_deliveries for each row execute function tally_append_only();
create trigger webhook_deliveries_no_truncate before truncate on webhook_deliveries for each statement execute function tally_append_only();
