-- App-encrypted bundle blobs (replaces the S3 bucket; keys are uuid v4).
create table if not exists bundles (
  key text primary key,
  bytes bytea not null,
  created_at timestamptz not null default now()
);
