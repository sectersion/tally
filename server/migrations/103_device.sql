alter table device_tokens
  alter column project_id drop not null,
  add column user_id text references users(id),
  add constraint device_tokens_scope check (project_id is not null or user_id is not null);
create table device_codes (
  code_hash text primary key,
  user_code text not null unique,
  device_name text not null,
  user_id text references users(id),
  denied boolean not null default false,
  consumed boolean not null default false,
  last_poll_at timestamptz,
  expires_at timestamptz not null
);
