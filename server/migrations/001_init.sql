create table projects (
  id text primary key,
  public_name text not null unique,
  owner_id text null,
  created_at timestamptz not null default now()
);

create table chains (
  id bigserial primary key,
  project_id text not null references projects(id),
  ruleset text not null,
  head_hash text not null,
  count bigint not null default 0,
  verified_s bigint not null default 0,
  unique (project_id, ruleset)
);

create table ledger_entries (
  chain_id bigint not null references chains(id),
  seq bigint not null,
  hash text not null,
  prev_hash text not null,
  action_id text not null,
  ts bigint not null,
  kind text not null,
  verified_s bigint not null,
  unverified_s bigint not null,
  entry json not null,
  received_at timestamptz not null default now(),
  primary key (chain_id, seq),
  unique (chain_id, hash)
);

create table commits (
  id bigserial primary key,
  project_id text not null references projects(id),
  hash text not null,
  ts bigint not null,
  received_at timestamptz not null default now()
);
create index commits_project_received_idx on commits (project_id, received_at);

create table device_tokens (
  id bigserial primary key,
  project_id text not null references projects(id),
  token_hash text not null unique,
  label text not null default '',
  created_at timestamptz not null default now(),
  revoked_at timestamptz null
);

create function tally_append_only() returns trigger language plpgsql as $$ begin raise exception 'append-only table: %', TG_TABLE_NAME; end $$;

create trigger ledger_entries_no_mutate before update or delete on ledger_entries
  for each row execute function tally_append_only();
create trigger ledger_entries_no_truncate before truncate on ledger_entries
  for each statement execute function tally_append_only();
create trigger commits_no_mutate before update or delete on commits
  for each row execute function tally_append_only();
create trigger commits_no_truncate before truncate on commits
  for each statement execute function tally_append_only();
