create table submissions (id uuid primary key, project_id text not null references projects(id), user_id text not null,
  from_ms bigint not null, to_ms bigint not null, ruleset text not null,
  status text not null check (status in ('uploaded','judging','in_review','completed','withdrawn')),
  judge_status text not null default 'pending' check (judge_status in ('pending','done','unavailable','partial')),
  timeline jsonb not null,              -- events only: hashes and numbers (C3), kept for re-derivation
  excluded_action_ids jsonb not null default '[]',   -- plan 04 skipped claims in range (ruling 3)
  object_key text,                      -- encrypted turns; NULL once deleted
  object_deleted_at timestamptz, created_at timestamptz not null default now(), completed_at timestamptz,
  reviewer_id text, reviewed_verified_s bigint, reviewed_unverified_s bigint);
create unique index one_open_submission on submissions(project_id) where status in ('uploaded','judging','in_review');
create table labels (id bigserial primary key, submission_id uuid not null references submissions,
  action_id text not null, label text check (label in ('steering','testing','debugging','repetitive','dark_flow')),
  confidence int check (confidence between 0 and 1000), rationale text not null default '',
  source text not null check (source in ('judge','reviewer')), judge_version text, reviewer_id text, reason text,
  created_at timestamptz not null default now(),
  check ((source = 'judge') = (judge_version is not null)), check ((source = 'reviewer') = (reviewer_id is not null and reason is not null)));
create table adjustments (id bigserial primary key, submission_id uuid not null references submissions,
  action_id text not null, approve_unverified_s int not null default 0 check (approve_unverified_s >= 0),
  remove_verified_s int not null default 0 check (remove_verified_s >= 0),
  reviewer_id text not null, reason text not null check (length(reason) > 0), created_at timestamptz not null default now());
create table audit_log (id bigserial primary key, ts timestamptz not null default now(), actor text not null,
  action text not null, submission_id uuid, detail jsonb not null default '{}');

create function append_only() returns trigger language plpgsql as $$
begin
  raise exception '% is append-only', tg_table_name;
end $$;
create trigger labels_append_only before update or delete on labels for each row execute function append_only();
create trigger adjustments_append_only before update or delete on adjustments for each row execute function append_only();
create trigger audit_log_append_only before update or delete on audit_log for each row execute function append_only();
create trigger labels_no_truncate before truncate on labels for each statement execute function append_only();
create trigger adjustments_no_truncate before truncate on adjustments for each statement execute function append_only();
create trigger audit_log_no_truncate before truncate on audit_log for each statement execute function append_only();
